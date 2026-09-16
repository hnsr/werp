#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use procast_core::{
    CancellationToken, ProcastError,
    media::{ProbeOptions, inspect},
};

fn fake_probe(dir: &Path, script: &str) -> ProbeOptions {
    let executable = dir.join("fake ffprobe");
    fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    ProbeOptions {
        executable,
        timeout: Duration::from_secs(5),
    }
}

fn input(dir: &Path) -> PathBuf {
    let path = dir.join("épisode ; $(touch NEVER) [test].mkv");
    fs::write(&path, "placeholder").unwrap();
    path
}

#[tokio::test]
async fn preserves_path_arguments_and_closes_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let file = input(dir.path());
    let options = fake_probe(
        dir.path(),
        r#"
if read -r value; then exit 20; fi
printf '%s\n' "$@" > "$0.args"
printf '%s' '{"format":{"format_name":"matroska,webm","duration":"12.5"},"streams":[]}'
"#,
    );
    let info = inspect(&file, &options, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(info.duration_seconds, Some(12.5));
    assert_eq!(info.path, file.canonicalize().unwrap());
    let args = fs::read_to_string(dir.path().join("fake ffprobe.args")).unwrap();
    let args: Vec<_> = args.lines().collect();
    assert_eq!(args[args.len() - 2], "-i");
    assert_eq!(
        args.last().unwrap(),
        &file.canonicalize().unwrap().to_str().unwrap()
    );
}

#[tokio::test]
async fn distinguishes_input_executable_and_metadata_errors() {
    let dir = tempfile::tempdir().unwrap();
    let file = input(dir.path());
    let token = CancellationToken::new();
    let missing = ProbeOptions {
        executable: dir.path().join("missing"),
        ..Default::default()
    };
    assert!(matches!(
        inspect(&dir.path().join("absent.mkv"), &missing, &token).await,
        Err(ProcastError::InputIo { .. })
    ));
    assert!(matches!(
        inspect(dir.path(), &missing, &token).await,
        Err(ProcastError::NotRegularFile(_))
    ));
    assert!(matches!(
        inspect(&file, &missing, &token).await,
        Err(ProcastError::MissingExecutable(_))
    ));
    let options = fake_probe(dir.path(), "printf 'not json'");
    assert!(matches!(
        inspect(&file, &options, &token).await,
        Err(ProcastError::InvalidMetadata(_))
    ));
    token.cancel();
    assert!(matches!(
        inspect(&file, &options, &token).await,
        Err(ProcastError::Cancelled)
    ));
}

#[tokio::test]
async fn drains_large_stderr_and_retains_the_failure_tail() {
    let dir = tempfile::tempdir().unwrap();
    let file = input(dir.path());
    let options = fake_probe(
        dir.path(),
        r#"
head -c 131072 /dev/zero >&2
printf 'final diagnostic' >&2
exit 7
"#,
    );
    match inspect(&file, &options, &CancellationToken::new())
        .await
        .unwrap_err()
    {
        ProcastError::ProcessFailed { status, stderr, .. } => {
            assert_eq!(status.code(), Some(7));
            assert!(stderr.ends_with("final diagnostic"));
            assert!(stderr.len() <= 32768);
        }
        error => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn rejects_excessive_metadata_without_waiting_for_eof() {
    let dir = tempfile::tempdir().unwrap();
    let file = input(dir.path());
    let options = fake_probe(dir.path(), "head -c 4194305 /dev/zero\nexec sleep 60");
    let result = inspect(&file, &options, &CancellationToken::new()).await;
    assert!(
        matches!(result, Err(ProcastError::OutputLimit { .. })),
        "{result:?}"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_and_timeout_kill_and_reap_the_child() {
    for cancel in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let file = input(dir.path());
        let mut options = fake_probe(dir.path(), "echo $$ > \"$0.pid\"\nexec sleep 60");
        options.timeout = Duration::from_secs(1);
        let token = CancellationToken::new();
        let pid_path = dir.path().join("fake ffprobe.pid");
        let control = async {
            let pid = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if let Ok(text) = tokio::fs::read_to_string(&pid_path).await
                        && let Ok(pid) = text.trim().parse::<u32>()
                    {
                        break pid;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("fake probe did not start");
            if cancel {
                token.cancel();
            }
            pid
        };
        let (result, pid) = tokio::join!(inspect(&file, &options, &token), control);
        if cancel {
            assert!(matches!(result, Err(ProcastError::Cancelled)), "{result:?}");
        } else {
            assert!(
                matches!(result, Err(ProcastError::TimedOut(_))),
                "{result:?}"
            );
        }
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "child {pid} survived or was not reaped"
        );
    }
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe; run explicitly with --ignored"]
async fn inspects_generated_video_audio_and_subtitles_with_real_ffprobe() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("épisode [test] with spaces.mkv");
    let subtitles = dir.path().join("sample.srt");
    fs::write(
        &subtitles,
        "1\n00:00:00,000 --> 00:00:00,800\nHello Procast!\n",
    )
    .unwrap();
    let output = tokio::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "color=size=160x90:rate=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-i",
        ])
        .arg(subtitles)
        .args([
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-map",
            "2:s",
            "-t",
            "1",
            "-c:v",
            "ffv1",
            "-c:a",
            "pcm_s16le",
            "-c:s",
            "srt",
            "-threads",
            "1",
            "-metadata:s:s:0",
            "language=eng",
        ])
        .arg(&file)
        .kill_on_drop(true)
        .output()
        .await
        .expect("ffmpeg is required");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info = inspect(&file, &ProbeOptions::default(), &CancellationToken::new())
        .await
        .unwrap();
    assert!(info.container.contains("matroska"));
    assert_eq!(info.streams.len(), 3);
    assert_eq!(info.streams[0].codec.as_deref(), Some("ffv1"));
    assert_eq!(info.streams[1].sample_rate_hz, Some(48000));
    assert_eq!(info.streams[2].kind, "subtitle");
    assert_eq!(info.streams[2].language.as_deref(), Some("eng"));
    assert!((info.duration_seconds.unwrap() - 1.0).abs() < 0.1);
}
