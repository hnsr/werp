#![cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};
use werp_core::{
    CancellationToken, WerpError,
    media::{self, MediaInfo, ProbeOptions},
    transcode::{self, TranscodeMode, TranscodeOptions},
};

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
        ..Default::default()
    }
}

fn cache_empty(options: &TranscodeOptions) {
    let path = options.directory.as_ref().unwrap();
    assert!(!path.exists() || fs::read_dir(path).unwrap().next().is_none());
}

#[tokio::test]
#[ignore = "requires real FFmpeg with libx264, AAC and ffprobe"]
async fn hdr_copy_paths_preserve_signalling_and_experimental_encoding_completes() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let probe = ProbeOptions::default();
    for transfer in ["smpte2084", "arib-std-b67"] {
        let path = dir.path().join(format!("{transfer}.mkv"));
        let output = tokio::process::Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=24:duration=1",
                "-f",
                "lavfi",
                "-i",
                "sine=sample_rate=48000:duration=1",
                "-c:v",
                "libx264",
                "-vf",
                &format!(
                    "setparams=color_primaries=bt2020:color_trc={transfer}:colorspace=bt2020nc"
                ),
                "-threads",
                "2",
                "-color_trc",
                transfer,
                "-color_primaries",
                "bt2020",
                "-colorspace",
                "bt2020nc",
                "-c:a",
                "aac",
                "-ac",
                "2",
            ])
            .arg(&path)
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let original = fs::read(&path).unwrap();
        let info = media::inspect(&path, &probe, &cancel).await.unwrap();
        assert_eq!(info.streams[0].color_transfer.as_deref(), Some(transfer));
        let hash_args = [
            "-map",
            "0:v:0",
            "-c:v",
            "copy",
            "-f",
            "streamhash",
            "-hash",
            "sha256",
            "-",
        ];
        let original_hash = ffmpeg_output(&path, &hash_args).await;
        for mode in [TranscodeMode::Remux, TranscodeMode::AudioOnly] {
            let mut options = options(dir.path());
            options.mode = mode;
            let prepared = transcode::prepare(
                &info,
                Path::new("ffmpeg"),
                &probe,
                &options,
                &cancel,
                |_| {},
            )
            .await
            .unwrap();
            assert_eq!(
                prepared.info.streams[0].color_transfer.as_deref(),
                Some(transfer)
            );
            assert_eq!(
                ffmpeg_output(&prepared.info.path, &hash_args).await,
                original_hash
            );
            let colour = tokio::process::Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=color_transfer,color_primaries,color_space",
                    "-of",
                    "json",
                ])
                .arg(&prepared.info.path)
                .output()
                .await
                .unwrap();
            assert!(colour.status.success());
            let metadata: serde_json::Value = serde_json::from_slice(&colour.stdout).unwrap();
            assert_eq!(metadata["streams"][0]["color_primaries"], "bt2020");
            assert_eq!(metadata["streams"][0]["color_space"], "bt2020nc");
            prepared.close().unwrap();
            cache_empty(&options);
        }
        let mut options = options(dir.path());
        options.mode = TranscodeMode::AudioVideo;
        let prepared = transcode::prepare(
            &info,
            Path::new("ffmpeg"),
            &probe,
            &options,
            &cancel,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(prepared.info.streams[0].codec.as_deref(), Some("h264"));
        assert_eq!(
            prepared.info.streams[0].pixel_format.as_deref(),
            Some("yuv420p")
        );
        // Successful decoding verifies the experimental path runs, not HDR colour correctness.
        ffmpeg_output(&prepared.info.path, &["-map", "0:v:0", "-f", "null", "-"]).await;
        prepared.close().unwrap();
        cache_empty(&options);
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[tokio::test]
async fn remux_needs_no_encoders_and_rejects_lost_audio() {
    let dir = tempfile::tempdir().unwrap();
    let (mut info, probe) = input(dir.path()).await;
    info.container = "matroska,webm".into();
    let mut options = options(dir.path());
    options.mode = TranscodeMode::Remux;
    let ffmpeg = dir.path().join("ffmpeg");
    script(
        &ffmpeg,
        r#"
case "$*" in *-encoders*|*libx264*|*-b:a*|*-vf*) exit 9;; esac
for last do :; done
printf 'prepared media' > "$last"
"#,
    );
    let prepared = transcode::prepare(
        &info,
        &ffmpeg,
        &probe,
        &options,
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    prepared.close().unwrap();
    cache_empty(&options);
    info.streams[0].color_transfer = Some("smpte2084".into());
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
    assert!(error.to_string().contains("colour transfer differs"));
    cache_empty(&options);
    info.streams[0].color_transfer = None;
    let mut silent: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/h264.json")).unwrap();
    silent["streams"].as_array_mut().unwrap().truncate(1);
    fs::write(dir.path().join("probe.json"), silent.to_string()).unwrap();
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
    assert!(error.to_string().contains("remuxed audio is missing"));
    cache_empty(&options);
    info.streams.truncate(1);
    let prepared = transcode::prepare(
        &info,
        &ffmpeg,
        &probe,
        &options,
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    prepared.close().unwrap();
    cache_empty(&options);
}

#[tokio::test]
#[ignore = "requires real FFmpeg with libx264, AAC, Matroska/MP4 and ffprobe"]
async fn real_remux_preserves_stream_payloads_timing_and_drops_extra_streams() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source [test].mkv");
    let subs = dir.path().join("captions.srt");
    let attachment = dir.path().join("note.txt");
    fs::write(&subs, "1\n00:00:00,000 --> 00:00:02,000\nTest caption\n").unwrap();
    fs::write(&attachment, "Test attachment").unwrap();
    let output = tokio::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24:duration=3",
            "-itsoffset",
            "0.25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2.5",
            "-i",
        ])
        .arg(&subs)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-map",
            "2:s:0",
            "-c:v",
            "libx264",
            "-profile:v",
            "high",
            "-level:v",
            "4.1",
            "-threads",
            "2",
            "-c:a",
            "aac",
            "-ac",
            "1",
            "-c:s",
            "srt",
            "-attach",
        ])
        .arg(&attachment)
        .args(["-metadata:s:t", "mimetype=text/plain"])
        .arg(&source)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let original = fs::read(&source).unwrap();
    let cancel = CancellationToken::new();
    let probe = ProbeOptions::default();
    let info = media::inspect(&source, &probe, &cancel).await.unwrap();
    assert!(info.streams.iter().any(|s| s.kind == "subtitle"));
    assert!(info.streams.iter().any(|s| s.kind == "attachment"));
    let mut options = options(dir.path());
    options.mode = TranscodeMode::Remux;
    options.timeout = Duration::from_secs(30);
    let prepared = transcode::prepare(
        &info,
        Path::new("ffmpeg"),
        &probe,
        &options,
        &cancel,
        |_| {},
    )
    .await
    .unwrap();
    media::validate_direct_play(&prepared.info).unwrap();
    assert_eq!(prepared.info.streams.len(), 2);
    assert_eq!(
        prepared.info.streams[1].channels,
        Some(1),
        "mono must not be re-encoded to stereo"
    );
    for stream in ["0:v:0", "0:a:0"] {
        let args = [
            "-map",
            stream,
            "-c",
            "copy",
            "-f",
            "streamhash",
            "-hash",
            "sha256",
            "-",
        ];
        assert_eq!(
            ffmpeg_output(&source, &args).await,
            ffmpeg_output(&prepared.info.path, &args).await,
            "{stream} payload changed"
        );
    }
    let input_offset = first_pts(&source, "a:0").await - first_pts(&source, "v:0").await;
    let output_offset =
        first_pts(&prepared.info.path, "a:0").await - first_pts(&prepared.info.path, "v:0").await;
    assert!(
        (input_offset - output_offset).abs() < 0.003,
        "relative stream offset changed: {input_offset} -> {output_offset}"
    );
    assert_eq!(fs::read(&source).unwrap(), original);
    let encoded = fs::read(&prepared.info.path).unwrap();
    let atom = |name: &[u8]| encoded.windows(4).position(|w| w == name).unwrap();
    assert!(atom(b"moov") < atom(b"mdat"));
    prepared.close().unwrap();
    cache_empty(&options);
}

#[tokio::test]
async fn audio_only_requires_compatible_video_and_audio_but_no_video_encoder() {
    let dir = tempfile::tempdir().unwrap();
    let (mut info, probe) = input(dir.path()).await;
    let mut options = options(dir.path());
    options.mode = TranscodeMode::AudioOnly;
    let ffmpeg = dir.path().join("ffmpeg");
    script(
        &ffmpeg,
        r#"
case "$*" in *-encoders*) printf ' A..... aac encoder\n'; exit 0;; esac
for last do :; done
printf 'prepared media' > "$last"
"#,
    );
    // The output probe reports stereo AAC, while the input is AC-3.
    info.streams[1].codec = Some("ac3".into());
    info.streams[1].channels = Some(6);
    let prepared = transcode::prepare(
        &info,
        &ffmpeg,
        &probe,
        &options,
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    prepared.close().unwrap();
    cache_empty(&options);

    // Reject inputs that would need video encoding or broader container support,
    // before starting FFmpeg (which is now deliberately missing).
    fs::remove_file(&ffmpeg).unwrap();
    for case in ["hevc", "4k", "no-audio", "multiple-audio"] {
        let (mut invalid, _) = input(dir.path()).await;
        match case {
            "mkv" => invalid.container = "matroska,webm".into(),
            "hevc" => invalid.streams[0].codec = Some("hevc".into()),
            "4k" => invalid.streams[0].width = Some(3840),
            "no-audio" => invalid.streams.truncate(1),
            "multiple-audio" => {
                let (mut other, _) = input(dir.path()).await;
                invalid.streams.push(other.streams.remove(1));
            }
            _ => unreachable!(),
        }
        let error = transcode::prepare(
            &invalid,
            &ffmpeg,
            &probe,
            &options,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .err()
        .unwrap();
        assert!(!matches!(error, WerpError::MissingExecutable(_)), "{case}");
        cache_empty(&options);
    }
    script(&ffmpeg, "printf ' V..... libx264 encoder\n'");
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
    assert!(error.to_string().contains("lacks the aac encoder"));
}

async fn ffmpeg_output(path: &Path, args: &[&str]) -> Vec<u8> {
    let output = tokio::process::Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

async fn first_pts(path: &Path, stream: &str) -> f64 {
    let output = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            stream,
            "-show_packets",
            "-show_entries",
            "packet=pts_time",
            "-of",
            "json",
        ])
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["packets"][0]["pts_time"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
#[ignore = "requires FFmpeg with libx264/libx265, AAC, AC-3/E-AC-3 and ffprobe"]
async fn extended_mkv_preparation_preserves_video_and_converts_only_unsupported_audio() {
    use werp_core::{
        media::DirectPlayPolicy,
        playback::{self, Mode},
    };
    for (video_codec, audio_codec, expected) in [
        ("libx264", "ac3", Mode::Audio),
        ("libx265", "aac", Mode::Remux),
        ("libx265", "eac3", Mode::Audio),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mkv");
        let mut command = tokio::process::Command::new("ffmpeg");
        command.args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24:duration=2",
            "-itsoffset",
            "0.25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=1.5",
            "-c:v",
            video_codec,
            "-threads",
            "2",
        ]);
        if video_codec == "libx265" {
            command.args([
                "-pix_fmt",
                "yuv420p10le",
                "-preset",
                "ultrafast",
                "-x265-params",
                "pools=1:frame-threads=1:log-level=error",
            ]);
        } else {
            command.args(["-profile:v", "high", "-level:v", "4.1"]);
        }
        command.args(["-c:a", audio_codec, "-ac", "6", "-b:a", "192k"]);
        let generated = command
            .arg(&source)
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert!(
            generated.status.success(),
            "{}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let cancel = CancellationToken::new();
        let probe = ProbeOptions::default();
        let info = media::inspect(&source, &probe, &cancel).await.unwrap();
        let plan = playback::select(&info, Mode::Auto, DirectPlayPolicy::Extended).unwrap();
        assert_eq!(plan.mode, expected, "{video_codec}/{audio_codec}");
        let mut options = options(dir.path());
        options.timeout = Duration::from_secs(30);
        options.mode = plan.preparation().unwrap();
        options.playback_policy = plan.policy;
        let output = transcode::prepare(
            &info,
            Path::new("ffmpeg"),
            &probe,
            &options,
            &cancel,
            |_| {},
        )
        .await
        .unwrap();
        let args = [
            "-map",
            "0:v:0",
            "-c",
            "copy",
            "-f",
            "streamhash",
            "-hash",
            "sha256",
            "-",
        ];
        assert_eq!(
            ffmpeg_output(&source, &args).await,
            ffmpeg_output(&output.info.path, &args).await
        );
        assert_eq!(output.info.streams[1].codec.as_deref(), Some("aac"));
        assert_eq!(output.info.streams[1].profile.as_deref(), Some("LC"));
        assert_eq!(
            output.info.streams[1].channels,
            Some(if expected == Mode::Remux { 6 } else { 2 })
        );
        let source_offset = first_pts(&source, "a:0").await - first_pts(&source, "v:0").await;
        let output_offset =
            first_pts(&output.info.path, "a:0").await - first_pts(&output.info.path, "v:0").await;
        assert!((source_offset - output_offset).abs() < 0.05);
        if expected == Mode::Remux {
            let args = [
                "-map",
                "0:a:0",
                "-c",
                "copy",
                "-f",
                "streamhash",
                "-hash",
                "sha256",
                "-",
            ];
            assert_eq!(
                ffmpeg_output(&source, &args).await,
                ffmpeg_output(&output.info.path, &args).await
            );
        }
        output.close().unwrap();
        cache_empty(&options);
    }
}

#[tokio::test]
#[ignore = "requires real FFmpeg with libx264, AC-3, AAC and ffprobe"]
async fn real_audio_only_preserves_video_packets_and_audio_offset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("surround [test].mp4");
    // Offset audible AC-3 from video, so resetting stream timelines independently
    // would be caught. B-frames exercise copied video with negative DTS as well.
    let output = tokio::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24:duration=3",
            "-itsoffset",
            "0.25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2.5",
            "-c:v",
            "libx264",
            "-profile:v",
            "high",
            "-level:v",
            "4.1",
            "-threads",
            "2",
            "-c:a",
            "ac3",
            "-ac",
            "6",
            "-b:a",
            "384k",
            "-movflags",
            "+faststart",
        ])
        .arg(&path)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let original = fs::read(&path).unwrap();
    let cancel = CancellationToken::new();
    let probe = ProbeOptions::default();
    let info = media::inspect(&path, &probe, &cancel).await.unwrap();
    assert_eq!(info.streams[1].codec.as_deref(), Some("ac3"));
    let mut options = options(dir.path());
    options.mode = TranscodeMode::AudioOnly;
    options.timeout = Duration::from_secs(30);
    let prepared = transcode::prepare(
        &info,
        Path::new("ffmpeg"),
        &probe,
        &options,
        &cancel,
        |_| {},
    )
    .await
    .unwrap();
    media::validate_direct_play(&prepared.info).unwrap();
    let audio = &prepared.info.streams[1];
    assert_eq!(audio.codec.as_deref(), Some("aac"));
    assert_eq!(audio.channels, Some(2));
    let hash_args = [
        "-map",
        "0:v:0",
        "-c:v",
        "copy",
        "-f",
        "streamhash",
        "-hash",
        "sha256",
        "-",
    ];
    assert_eq!(
        ffmpeg_output(&path, &hash_args).await,
        ffmpeg_output(&prepared.info.path, &hash_args).await,
        "encoded video payload changed"
    );
    let input_offset = first_pts(&path, "a:0").await - first_pts(&path, "v:0").await;
    let output_offset =
        first_pts(&prepared.info.path, "a:0").await - first_pts(&prepared.info.path, "v:0").await;
    assert!(
        (input_offset - output_offset).abs() < 0.05,
        "audio offset changed from {input_offset} to {output_offset}"
    );
    let pcm = ffmpeg_output(
        &prepared.info.path,
        &["-map", "0:a:0", "-f", "s16le", "-c:a", "pcm_s16le", "-"],
    )
    .await;
    assert!(
        pcm.chunks_exact(2)
            .any(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs() > 100),
        "converted audio is silent"
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    let encoded = fs::read(&prepared.info.path).unwrap();
    let atom = |name: &[u8]| encoded.windows(4).position(|w| w == name).unwrap();
    assert!(atom(b"moov") < atom(b"mdat"));
    prepared.close().unwrap();
    cache_empty(&options);
}

#[tokio::test]
async fn unsupported_bitmap_burn_in_is_rejected_before_starting_ffmpeg() {
    let dir = tempfile::tempdir().unwrap();
    let (mut info, probe) = input(dir.path()).await;
    let mut options = options(dir.path());
    options.bitmap_subtitle = Some(2);
    for codec in ["dvd_subtitle", "dvb_subtitle"] {
        info.streams.retain(|stream| stream.kind != "subtitle");
        info.streams.push(media::StreamInfo {
            index: 2,
            kind: "subtitle".into(),
            codec: Some(codec.into()),
            ..Default::default()
        });
        let error = transcode::prepare(
            &info,
            &dir.path().join("missing-ffmpeg"),
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
                .contains("selected bitmap subtitle stream does not exist or is unsupported")
        );
        cache_empty(&options);
    }
}

#[tokio::test]
async fn preparation_rejects_dolby_vision_ambiguity_missing_encoders_and_insufficient_space() {
    let dir = tempfile::tempdir().unwrap();
    let (mut info, probe) = input(dir.path()).await;
    let options = options(dir.path());
    let ffmpeg = dir.path().join("ffmpeg");
    script(
        &ffmpeg,
        "printf ' V..... libx264 encoder\n A..... aac encoder\n'",
    );
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
        Err(WerpError::MissingExecutable(_))
    ));
}

#[tokio::test]
async fn failed_encoding_and_output_validation_remove_partial_media() {
    for fail_encoding in [true, false] {
        for mode in [
            TranscodeMode::AudioVideo,
            TranscodeMode::AudioOnly,
            TranscodeMode::Remux,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (info, probe) = input(dir.path()).await;
            let mut options = options(dir.path());
            options.mode = mode;
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
                assert!(matches!(error, WerpError::UnsupportedMedia(_)));
            }
            cache_empty(&options);
            assert_eq!(fs::read(&info.path).unwrap(), b"original input");
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_and_timeout_reap_encoder_and_remove_output() {
    for interrupt in [true, false] {
        for mode in [
            TranscodeMode::AudioVideo,
            TranscodeMode::AudioOnly,
            TranscodeMode::Remux,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (info, probe) = input(dir.path()).await;
            let mut options = options(dir.path());
            options.timeout = Duration::from_millis(500);
            options.mode = mode;
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
                assert!(matches!(error, WerpError::Cancelled), "{error}");
            } else {
                assert!(matches!(error, WerpError::TimedOut(_)), "{error}");
            }
            assert!(!Path::new(&format!("/proc/{pid}")).exists());
            assert!(progress.contains(&0.0));
            cache_empty(&options);
        }
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
