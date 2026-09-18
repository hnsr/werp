#![cfg(unix)]
use procast_core::{
    CancellationToken, ProcastError,
    media::{self, MediaInfo, ProbeOptions},
    transcode::{self, TranscodeOptions},
};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

async fn input(directory: &Path) -> (MediaInfo, ProbeOptions) {
    let video = directory.join("input ; [épisode].mkv");
    fs::write(&video, b"original input").unwrap();
    let probe = directory.join("probe");
    script(&probe, "cat \"$0.json\"");
    fs::write(
        directory.join("probe.json"),
        include_str!("fixtures/h264.json"),
    )
    .unwrap();
    let probe = ProbeOptions {
        executable: probe,
        ..Default::default()
    };
    (
        media::inspect(&video, &probe, &CancellationToken::new())
            .await
            .unwrap(),
        probe,
    )
}

fn options(directory: &Path) -> TranscodeOptions {
    TranscodeOptions {
        directory: Some(directory.join("cache")),
        timeout: Duration::from_secs(5),
    }
}

fn cache_empty(options: &TranscodeOptions) {
    let path = options.directory.as_ref().unwrap();
    assert!(!path.exists() || fs::read_dir(path).unwrap().next().is_none());
}

#[tokio::test]
async fn preparation_rejects_hdr_ambiguity_missing_encoders_and_insufficient_space() {
    let dir = tempfile::tempdir().unwrap();
    let (mut info, probe) = input(dir.path()).await;
    let options = options(dir.path());
    let ffmpeg = dir.path().join("ffmpeg");
    script(
        &ffmpeg,
        "printf ' V..... libx264 encoder\n A..... aac encoder\n'",
    );
    for hdr in ["smpte2084", "arib-std-b67"] {
        info.streams[0].color_transfer = Some(hdr.into());
        let error = transcode::prepare(
            &info,
            &ffmpeg,
            &probe,
            &options,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("tone mapping"));
    }
    info.streams[0].color_transfer = None;
    info.streams[0].dolby_vision = true;
    assert!(
        transcode::prepare(
            &info,
            &ffmpeg,
            &probe,
            &options,
            &CancellationToken::new(),
            |_| {}
        )
        .await
        .is_err()
    );
    info.streams[0].dolby_vision = false;
    let fixture = include_str!("fixtures/h264.json");
    let mut tracks: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let audio = tracks["streams"][1].clone();
    tracks["streams"].as_array_mut().unwrap().push(audio);
    fs::write(dir.path().join("probe.json"), tracks.to_string()).unwrap();
    let ambiguous = media::inspect(&info.path, &probe, &CancellationToken::new())
        .await
        .unwrap();
    let error = transcode::prepare(
        &ambiguous,
        &ffmpeg,
        &probe,
        &options,
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .err()
    .unwrap();
    assert!(error.to_string().contains("multiple audio tracks"));
    fs::write(dir.path().join("probe.json"), fixture).unwrap();
    info.duration_seconds = Some(1e15);
    let error = transcode::prepare(
        &info,
        &ffmpeg,
        &probe,
        &options,
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .err()
    .unwrap();
    assert!(
        error
            .to_string()
            .contains("insufficient preparation disk space")
    );
    cache_empty(&options);
    info.duration_seconds = Some(2.0);
    script(&ffmpeg, "printf ' A..... aac encoder\n'");
    let error = transcode::prepare(
        &info,
        &ffmpeg,
        &probe,
        &options,
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .err()
    .unwrap();
    assert!(error.to_string().contains("lacks the libx264 encoder"));
    cache_empty(&options);
    fs::remove_file(&ffmpeg).unwrap();
    assert!(matches!(
        transcode::prepare(
            &info,
            &ffmpeg,
            &probe,
            &options,
            &CancellationToken::new(),
            |_| {}
        )
        .await,
        Err(ProcastError::MissingExecutable(_))
    ));
}

#[tokio::test]
async fn failed_encoding_and_output_validation_remove_partial_media() {
    for fail_encoding in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (info, probe) = input(dir.path()).await;
        let options = options(dir.path());
        let ffmpeg = dir.path().join("ffmpeg");
        script(
            &ffmpeg,
            &format!(
                r#"
case "$*" in *-encoders*) printf ' V..... libx264 encoder\n A..... aac encoder\n'; exit 0;; esac
for last do :; done
printf 'partial media' > "$last"
printf 'encoding diagnostic' >&2
exit {}
"#,
                if fail_encoding { 7 } else { 0 }
            ),
        );
        if !fail_encoding {
            // Source metadata was already read; make output probing reject the output.
            fs::write(
                dir.path().join("probe.json"),
                include_str!("fixtures/h264.json").replace("h264", "hevc"),
            )
            .unwrap();
        }
        let error = transcode::prepare(
            &info,
            &ffmpeg,
            &probe,
            &options,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .err()
        .unwrap();
        if fail_encoding {
            assert!(error.to_string().contains("encoding diagnostic"));
        } else {
            assert!(matches!(error, ProcastError::UnsupportedMedia(_)));
        }
        cache_empty(&options);
        assert_eq!(fs::read(&info.path).unwrap(), b"original input");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_and_timeout_reap_encoder_and_remove_output() {
    for interrupt in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (info, probe) = input(dir.path()).await;
        let mut options = options(dir.path());
        options.timeout = Duration::from_millis(500);
        let ffmpeg = dir.path().join("ffmpeg");
        script(
            &ffmpeg,
            r#"
case "$*" in *-encoders*) printf ' V..... libx264 encoder\n A..... aac encoder\n'; exit 0;; esac
for last do :; done
printf 'partial media' > "$last"
echo $$ > "$0.pid"
printf 'out_time_us=1000000\nprogress=continue\n'
exec sleep 60
"#,
        );
        let token = CancellationToken::new();
        let pid_path = dir.path().join("ffmpeg.pid");
        let control = async {
            let pid = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if let Ok(s) = tokio::fs::read_to_string(&pid_path).await
                        && let Ok(pid) = s.trim().parse::<u32>()
                    {
                        break pid;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            if interrupt {
                token.cancel();
            }
            pid
        };
        let mut progress = Vec::new();
        let (result, pid) = tokio::join!(
            transcode::prepare(&info, &ffmpeg, &probe, &options, &token, |p| progress
                .push(p.fraction)),
            control
        );
        let error = result.err().unwrap();
        if interrupt {
            assert!(matches!(error, ProcastError::Cancelled), "{error}");
        } else {
            assert!(matches!(error, ProcastError::TimedOut(_)), "{error}");
        }
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        assert!(progress.contains(&0.0));
        cache_empty(&options);
    }
}

async fn generate(path: &Path, size: &str, rate: &str, audio: bool) {
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.args([
        "-nostdin",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc2=size={size}:rate={rate}:duration=1"),
    ]);
    if audio {
        cmd.args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=5.1:d=1"]);
    }
    cmd.args(["-c:v", "ffv1", "-c:a", "pcm_s16le", "-threads", "2"])
        .arg(path)
        .kill_on_drop(true);
    let output = cmd.output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "requires real FFmpeg with libx264, AAC, FFV1 and ffprobe"]
async fn real_transcode_converts_surround_scales_caps_fps_and_preserves_sources() {
    for (size, rate, audio) in [("320x180", "24000/1001", true), ("1922x1082", "60", false)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original [test].mkv");
        generate(&path, size, rate, audio).await;
        let original = fs::read(&path).unwrap();
        let cancel = CancellationToken::new();
        let probe = ProbeOptions::default();
        let info = media::inspect(&path, &probe, &cancel).await.unwrap();
        let mut options = options(dir.path());
        options.timeout = Duration::from_secs(30);
        let mut progress = Vec::new();
        let prepared =
            transcode::prepare(&info, Path::new("ffmpeg"), &probe, &options, &cancel, |p| {
                progress.push(p.fraction)
            })
            .await
            .unwrap();
        media::validate_direct_play(&prepared.info).unwrap();
        let video = &prepared.info.streams[0];
        assert!(video.width.unwrap() <= 1920 && video.height.unwrap() <= 1080);
        assert!(video.frame_rate.unwrap() <= 30.01);
        if size == "320x180" {
            assert_eq!((video.width, video.height), (Some(320), Some(180)));
        }
        assert_eq!(prepared.info.streams.len(), if audio { 2 } else { 1 });
        if audio {
            assert_eq!(prepared.info.streams[1].channels, Some(2));
        }
        assert_eq!(fs::read(&path).unwrap(), original);
        let encoded = fs::read(&prepared.info.path).unwrap();
        let atom = |name: &[u8]| encoded.windows(4).position(|w| w == name).unwrap();
        assert!(
            atom(b"moov") < atom(b"mdat"),
            "faststart metadata must precede video"
        );
        assert_eq!(progress.last(), Some(&1.0));
        let output = prepared.info.path.clone();
        prepared.close().unwrap();
        assert!(!output.exists());
        cache_empty(&options);
    }
}
