use std::process::Command;

#[cfg(unix)]
#[test]
fn force_direct_bypasses_cli_hdr_preflight_but_keeps_subtitle_validation() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hdr.mp4");
    fs::write(&file, "original").unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let mut metadata: serde_json::Value =
        serde_json::from_str(include_str!("../../werp-core/tests/fixtures/h264.json")).unwrap();
    metadata["streams"][0]["color_transfer"] = "smpte2084".into();
    fs::write(dir.path().join("probe.json"), metadata.to_string()).unwrap();
    for forced in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_werp"));
        command
            .args(["--no-config", "--no-resume", "--no-inhibit-sleep"])
            .arg(&file)
            .arg("--ffprobe")
            .arg(&probe)
            .args(["--host", "127.0.0.1", "--subtitles"])
            .arg(dir.path().join("missing.vtt"));
        if forced {
            command.arg("--force-direct");
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        if forced {
            assert!(stderr.contains("Forced direct playback"), "{stderr}");
            assert!(stderr.contains("missing.vtt"), "{stderr}");
            assert!(!stderr.contains("tone mapping"), "{stderr}");
        } else {
            assert!(stderr.contains("tone mapping"), "{stderr}");
        }
    }
}

#[cfg(unix)]
#[test]
fn experimental_profile_reaches_preflight_and_preserves_other_checks() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("surround.mp4");
    fs::write(&file, "placeholder").unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let mut metadata: serde_json::Value =
        serde_json::from_str(include_str!("../../werp-core/tests/fixtures/h264.json")).unwrap();
    metadata["streams"][1]["channels"] = serde_json::json!(6);
    // A missing subtitle gives a deterministic failure after media preflight,
    // before any discovery, serving, or receiver connection.
    for (experimental, codec, expected) in [
        (false, "aac", "--profile extended"),
        (true, "aac", "missing.vtt"),
        (
            false,
            "ac3",
            "AC-3 passthrough requires --profile experimental",
        ),
        (true, "ac3", "missing.vtt"),
        (true, "eac3", "outside the available direct-play profiles"),
    ] {
        metadata["streams"][1]["codec_name"] = serde_json::json!(codec);
        fs::write(dir.path().join("probe.json"), metadata.to_string()).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_werp"));
        command.arg("--no-config");
        command
            .arg("--no-resume")
            .arg("--no-inhibit-sleep")
            .arg(&file)
            .arg("--ffprobe")
            .arg(&probe)
            .args(["--host", "127.0.0.1"])
            .arg("--subtitles")
            .arg(dir.path().join("missing.vtt"));
        if experimental {
            command.args(["--mode", "direct", "--profile", "experimental"]);
        } else {
            command.args(["--mode", "direct", "--profile", "baseline"]);
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
    let output = Command::new(env!("CARGO_BIN_EXE_werp"))
        .arg("--no-config")
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for command in ["inspect", "devices"] {
        assert!(help.contains(command));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_werp"))
        .arg("--no-config")
        .args(["inspect", "movie.mkv", "--timeout", "0"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn auto_remux_reuses_saved_output_and_readonly_source_falls_back_to_user_cache() {
    use std::{fs, net::TcpListener, os::unix::fs::PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("source");
    fs::create_dir(&parent).unwrap();
    let source = parent.join("input.mkv");
    fs::write(&source, "original").unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\nfor last do :; done\ncase \"$last\" in *input.mkv) cat \"$0.input.json\";; *) cat \"$0.output.json\";; esac\n").unwrap();
    let fixture = include_str!("../../werp-core/tests/fixtures/h264.json");
    fs::write(dir.path().join("probe.output.json"), fixture).unwrap();
    fs::write(
        dir.path().join("probe.input.json"),
        fixture.replace("mov,mp4,m4a,3gp,3g2,mj2", "matroska,webm"),
    )
    .unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let ffmpeg = dir.path().join("ffmpeg");
    fs::write(
        &ffmpeg,
        "#!/bin/sh\nfor last do :; done\nprintf prepared > \"$last\"\n",
    )
    .unwrap();
    fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o700)).unwrap();
    // Reserve a loopback address without listening for Cast TLS: after closing
    // it, refusal lets us verify preparation without a real receiver.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let run = || {
        let output = Command::new(env!("CARGO_BIN_EXE_werp"))
            .arg("--no-config")
            .arg("--no-resume")
            .arg("--no-inhibit-sleep")
            .arg(&source)
            .args(["--host", "127.0.0.1", "--cast-port", &port.to_string()])
            .arg("--ffprobe")
            .arg(&probe)
            .arg("--ffmpeg")
            .arg(&ffmpeg)
            .env("XDG_CACHE_HOME", dir.path().join("user-cache"))
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("Selected Remux"), "{stderr}");
        assert!(stderr.contains("Connection refused"), "{stderr}");
        stderr
    };
    let first = run();
    assert!(first.contains("Saved prepared file"), "{first}");
    fs::rename(&ffmpeg, dir.path().join("saved-ffmpeg")).unwrap();
    let second = run();
    assert!(second.contains("Reusing prepared file"), "{second}");
    fs::rename(dir.path().join("saved-ffmpeg"), &ffmpeg).unwrap();
    // Change source content so the adjacent entry is ineligible; no metadata-only hit.
    fs::write(&source, "modified").unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
    let third = run();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        third.contains("Source folder is not writable; using cache"),
        "{third}"
    );
    assert!(third.contains("user-cache"), "{third}");
    assert_eq!(fs::read(&source).unwrap(), b"modified");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn signals_wait_for_probe_cleanup_and_return_distinct_exit_codes() {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Stdio, time::Duration};
    for (command, signal, exit_code) in [
        ("inspect", "-INT", 130),
        ("inspect", "-TERM", 143),
        ("", "-INT", 130),
        ("", "-TERM", 143),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let probe = dir.path().join("probe");
        fs::write(&probe, "#!/bin/sh\necho $$ > \"$0.pid\"\nexec sleep 60\n").unwrap();
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
        let inhibitor = dir.path().join("systemd-inhibit");
        fs::write(
            &inhibitor,
            "#!/bin/sh\necho $$ > \"$0.pid\"\nprintf R\nread -r ignored || :\n",
        )
        .unwrap();
        fs::set_permissions(&inhibitor, fs::Permissions::from_mode(0o700)).unwrap();
        let search_path = std::env::join_paths(std::iter::once(dir.path().to_path_buf()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        let file = dir.path().join("movie");
        fs::write(&file, "placeholder").unwrap();
        let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_werp"))
            .arg("--no-config")
            .args((!command.is_empty()).then_some(command))
            .env("PATH", search_path)
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
        let inhibitor_pid = dir.path().join("systemd-inhibit.pid");
        if command.is_empty() {
            let helper_pid = fs::read_to_string(inhibitor_pid).unwrap();
            assert!(!Path::new(&format!("/proc/{}", helper_pid.trim())).exists());
            assert!(String::from_utf8_lossy(&output.stderr).contains("Sleep inhibition active"));
        } else {
            assert!(!inhibitor_pid.exists(), "inspection must not inhibit sleep");
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn sleep_inhibition_is_best_effort_and_released_on_preflight_failure() {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};
    for mode in ["missing", "denied", "active", "disabled"] {
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("systemd-inhibit");
        if mode != "missing" {
            let body = if mode == "denied" {
                "exit 1"
            } else {
                "echo $$ > \"$0.pid\"; printf R; read -r ignored || :"
            };
            fs::write(&helper, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_werp"));
        command.arg("--no-config");
        command
            .arg("--no-resume")
            .arg(dir.path().join("missing.mp4"))
            .env("PATH", dir.path());
        if mode == "disabled" {
            command.arg("--no-inhibit-sleep");
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(
            stderr.contains("missing.mp4"),
            "preflight must still run: {stderr}"
        );
        if mode == "active" {
            assert!(stderr.contains("Sleep inhibition active"), "{stderr}");
            let pid = fs::read_to_string(dir.path().join("systemd-inhibit.pid")).unwrap();
            assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
        } else if mode == "disabled" {
            assert!(!stderr.contains("inhibit sleep"), "{stderr}");
            assert!(!dir.path().join("systemd-inhibit.pid").exists());
        } else {
            assert!(stderr.contains("Could not inhibit sleep"), "{stderr}");
            assert!(!stderr.contains("Sleep inhibition active"), "{stderr}");
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn preparation_signals_reap_encoder_and_remove_partial_output() {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Stdio, time::Duration};
    for (mode, signal, exit_code) in [
        ("transcode", "-INT", 130),
        ("transcode", "-TERM", 143),
        ("audio", "-INT", 130),
        ("audio", "-TERM", 143),
        ("remux", "-INT", 130),
        ("remux", "-TERM", 143),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("compatible.mp4");
        fs::write(&file, "unchanged input").unwrap();
        let probe = dir.path().join("probe");
        fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
        fs::write(
            dir.path().join("probe.json"),
            include_str!("../../werp-core/tests/fixtures/h264.json"),
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
        let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_werp"))
            .arg("--no-config")
            .arg("--no-resume")
            .arg("--no-inhibit-sleep")
            .arg(&file)
            .args(["--mode", mode])
            .args(["--host", "127.0.0.1", "--no-cache"])
            .arg("--cache-dir")
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
        .expect("preparation did not start FFmpeg for compatible media");
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
