#![cfg(unix)]

use procast_core::{CancellationToken, ProcastError, subtitles};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};

#[tokio::test]
async fn rejects_missing_converter_non_utf8_and_invalid_cues() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("captions.srt");
    let missing = dir.path().join("missing-ffmpeg");
    let token = CancellationToken::new();
    fs::write(&path, "1\n00:00:00,000 --> 00:00:01,000\nHello\n").unwrap();
    assert!(matches!(
        subtitles::prepare(&path, &missing, &token).await,
        Err(ProcastError::MissingExecutable(_))
    ));
    for bytes in [
        b"\xff".as_slice(),
        b"1\n00:00:02,000 --> 00:00:01,000\nHello\n",
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            subtitles::prepare(&path, &missing, &token).await,
            Err(ProcastError::Subtitles(_))
        ));
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancelled_conversion_reaps_ffmpeg_and_removes_temporary_input() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("captions ü ; $(echo never).srt");
    let content = "1\n00:00:00,000 --> 00:00:01,000\nHello\n";
    fs::write(&source, content).unwrap();
    let converter = dir.path().join("ffmpeg");
    fs::write(
        &converter,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\necho $$ > \"$0.pid\"\nexec sleep 60\n",
    )
    .unwrap();
    fs::set_permissions(&converter, fs::Permissions::from_mode(0o700)).unwrap();
    let token = CancellationToken::new();
    let control = async {
        loop {
            if let Ok(text) = tokio::fs::read_to_string(dir.path().join("ffmpeg.pid")).await
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                token.cancel();
                break pid;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    let (result, pid) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(subtitles::prepare(&source, &converter, &token), control)
    })
    .await
    .expect("conversion cancellation hung");
    assert!(matches!(result, Err(ProcastError::Cancelled)));
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    let args = fs::read_to_string(dir.path().join("ffmpeg.args")).unwrap();
    let args: Vec<_> = args.lines().collect();
    let input = Path::new(args[args.iter().position(|arg| *arg == "-i").unwrap() + 1]);
    assert!(
        !input.parent().unwrap().exists(),
        "temporary subtitle directory survived cancellation"
    );
    assert_eq!(fs::read_to_string(source).unwrap(), content);
}
