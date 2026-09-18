use std::process::Command;

#[cfg(unix)]
#[test]
fn experimental_flag_reaches_preflight_and_preserves_other_checks() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("surround.mp4");
    fs::write(&file, "placeholder").unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let mut metadata: serde_json::Value =
        serde_json::from_str(include_str!("../../procast-core/tests/fixtures/h264.json")).unwrap();
    metadata["streams"][1]["channels"] = serde_json::json!(6);
    // A missing subtitle gives a deterministic failure after media preflight,
    // before any discovery, serving, or receiver connection.
    for (experimental, codec, expected) in [
        (false, "aac", "--experimental-direct-play"),
        (true, "aac", "missing.vtt"),
        (false, "ac3", "AC-3 requires --experimental-direct-play"),
        (true, "ac3", "missing.vtt"),
        (true, "eac3", "outside the available direct-play profiles"),
    ] {
        metadata["streams"][1]["codec_name"] = serde_json::json!(codec);
        fs::write(dir.path().join("probe.json"), metadata.to_string()).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_procast"));
        command
            .arg("cast")
            .arg(&file)
            .arg("--ffprobe")
            .arg(&probe)
            .arg("--subtitles")
            .arg(dir.path().join("missing.vtt"));
        if experimental {
            command.arg("--experimental-direct-play");
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
        if experimental && matches!(codec, "aac" | "ac3") {
            assert!(stderr.contains("Experimental direct play"), "{stderr}");
        }
    }
}

#[test]
fn help_and_argument_errors_are_available_without_ffprobe() {
    let output = Command::new(env!("CARGO_BIN_EXE_procast"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for command in ["inspect", "devices", "cast"] {
        assert!(help.contains(command));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_procast"))
        .args(["inspect", "movie.mkv", "--timeout", "0"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn signals_wait_for_probe_cleanup_and_return_distinct_exit_codes() {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Stdio, time::Duration};
    for (command, signal, exit_code) in [
        ("inspect", "-INT", 130),
        ("inspect", "-TERM", 143),
        ("cast", "-INT", 130),
        ("cast", "-TERM", 143),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let probe = dir.path().join("probe");
        fs::write(&probe, "#!/bin/sh\necho $$ > \"$0.pid\"\nexec sleep 60\n").unwrap();
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
        let file = dir.path().join("movie");
        fs::write(&file, "placeholder").unwrap();
        let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_procast"))
            .arg(command)
            .arg(file)
            .arg("--ffprobe")
            .arg(&probe)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let probe_pid = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(dir.path().join("probe.pid")).await
                    && let Ok(pid) = text.trim().parse::<u32>()
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("probe did not start");
        assert!(
            Command::new("kill")
                .args([signal, &pid.to_string()])
                .status()
                .unwrap()
                .success()
        );
        let output = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
            .await
            .expect("CLI did not shut down promptly")
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(exit_code),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(!Path::new(&format!("/proc/{probe_pid}")).exists());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn force_transcode_signals_reap_encoder_and_remove_partial_output() {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Stdio, time::Duration};
    for (flag, signal, exit_code) in [
        ("--force-transcode", "-INT", 130),
        ("--force-transcode", "-TERM", 143),
        ("--transcode-audio", "-INT", 130),
        ("--transcode-audio", "-TERM", 143),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("compatible.mp4");
        fs::write(&file, "unchanged input").unwrap();
        let probe = dir.path().join("probe");
        fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
        fs::write(
            dir.path().join("probe.json"),
            include_str!("../../procast-core/tests/fixtures/h264.json"),
        )
        .unwrap();
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
        let ffmpeg = dir.path().join("ffmpeg");
        fs::write(
            &ffmpeg,
            r#"#!/bin/sh
case "$*" in *-encoders*) printf ' V..... libx264 encoder\n A..... aac encoder\n'; exit 0;; esac
for last do :; done
printf partial > "$last"
echo $$ > "$0.pid"
printf 'out_time_us=1000000\nprogress=continue\n'
exec sleep 60
"#,
        )
        .unwrap();
        fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o700)).unwrap();
        let cache = dir.path().join("cache");
        let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_procast"))
            .arg("cast")
            .arg(&file)
            .arg(flag)
            .arg("--transcode-dir")
            .arg(&cache)
            .arg("--ffprobe")
            .arg(&probe)
            .arg("--ffmpeg")
            .arg(&ffmpeg)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let encoder_pid = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(s) = tokio::fs::read_to_string(dir.path().join("ffmpeg.pid")).await
                    && let Ok(pid) = s.trim().parse::<u32>()
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("force-transcode did not start the encoder for compatible media");
        assert!(
            Command::new("kill")
                .args([signal, &pid.to_string()])
                .status()
                .unwrap()
                .success()
        );
        let result = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert_eq!(result.status.code(), Some(exit_code), "{stderr}");
        assert!(stderr.contains("cleanup completed"), "{stderr}");
        assert!(!Path::new(&format!("/proc/{encoder_pid}")).exists());
        assert!(fs::read_dir(&cache).unwrap().next().is_none());
        assert_eq!(fs::read(&file).unwrap(), b"unchanged input");
    }
}
