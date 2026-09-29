use std::path::Path;
use tokio::sync::watch;
use werp_core::{
    CancellationToken,
    conversion::{self, Phase, State},
};

async fn convert(file: &Path) -> State {
    convert_with(file, werp_core::media::DirectPlayPolicy::Conservative).await
}

async fn convert_with(file: &Path, policy: werp_core::media::DirectPlayPolicy) -> State {
    let (updates, state) = watch::channel(State::default());
    conversion::run(
        conversion::Request {
            file: file.into(),
            probe: Default::default(),
            ffmpeg: "ffmpeg".into(),
            inhibit_sleep: false,
            cache: Default::default(),
            policy,
        },
        updates,
        &CancellationToken::new(),
    )
    .await;
    let state = state.borrow().clone();
    assert_eq!(state.phase, Phase::Completed, "{:?}", state.error);
    state
}

#[tokio::test]
#[ignore = "requires real FFmpeg with libx264, AAC, PCM, FFV1 and ffprobe"]
async fn offline_conversion_copies_encodes_reuses_and_skips_compatible_sources() {
    let dir = tempfile::tempdir().unwrap();
    // Exercise every branch using tiny local fixtures; no receiver or network.
    for (name, video, audio, channels, operation) in [
        (
            "remux.mkv",
            "libx264",
            "aac",
            "2",
            "Copying video and audio into MP4",
        ),
        (
            "audio.mkv",
            "libx264",
            "pcm_s16le",
            "6",
            "Copying video; converting audio to stereo AAC",
        ),
        (
            "full.mkv",
            "ffv1",
            "pcm_s16le",
            "6",
            "Converting to H.264 and stereo AAC in MP4",
        ),
    ] {
        let file = dir.path().join(name);
        let result = tokio::process::Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=15:duration=1",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0|0|0.25*sin(440*2*PI*t)|0|0|0:c=5.1:s=48000:d=1",
                "-c:v",
                video,
                "-threads",
                "2",
                "-c:a",
                audio,
                "-ac",
                channels,
            ])
            .arg(&file)
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let original = std::fs::read(&file).unwrap();
        let first = convert(&file).await;
        assert!(!first.reused && !first.already_compatible);
        let planned = first.planned_target.as_ref().unwrap();
        assert_eq!(planned.container, "mp4");
        let target = first.target.as_ref().unwrap();
        for kind in ["video", "audio"] {
            let before = planned
                .streams
                .iter()
                .find(|stream| stream.kind == kind)
                .unwrap();
            let after = target
                .streams
                .iter()
                .find(|stream| stream.kind == kind)
                .unwrap();
            assert_eq!(before.codec, after.codec);
            assert_eq!(before.profile, after.profile);
            assert_eq!(before.channels, after.channels);
        }
        assert_eq!(first.operation.as_deref(), Some(operation));
        assert_eq!(first.fraction, Some(1.0));
        let output = first.output.unwrap();
        assert_eq!(output.parent(), file.parent());
        assert!(output.with_extension("mp4.json").exists());
        assert_eq!(std::fs::read(&file).unwrap(), original);
        let second = convert(&file).await;
        assert!(second.reused);
        assert_eq!(second.output.as_ref(), Some(&output));
        let already = convert(&output).await;
        assert!(already.already_compatible);
        assert_eq!(already.output.as_ref(), Some(&output));
        // Actual AAC output must retain centre-only source audio in both ears.
        let decoded = tokio::process::Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(&output)
            .args(["-vn", "-c:a", "pcm_f32le", "-f", "f32le", "-"])
            .output()
            .await
            .unwrap();
        assert!(decoded.status.success());
        let samples: Vec<f32> = decoded
            .stdout
            .chunks_exact(4)
            .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
            .collect();
        let energy = |channel| {
            samples
                .iter()
                .skip(channel)
                .step_by(2)
                .map(|s| f64::from(*s).powi(2))
                .sum::<f64>()
        };
        assert!(energy(0) > 1.0 && energy(1) > 1.0);
        assert!((energy(0) / energy(1) - 1.0).abs() < 0.05);
    }
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("session-")
    }));
}

#[tokio::test]
#[ignore = "requires real FFmpeg with libx264/libx265, AAC and ffprobe"]
async fn selected_device_preview_conversion_and_cast_cache_share_policy() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hevc-surround.mkv");
    let generated = tokio::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=24:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=1",
            "-c:v",
            "libx265",
            "-threads",
            "2",
            "-pix_fmt",
            "yuv420p10le",
            "-preset",
            "ultrafast",
            "-x265-params",
            "pools=1:frame-threads=1:log-level=error",
            "-c:a",
            "aac",
            "-ac",
            "6",
            "-b:a",
            "192k",
        ])
        .arg(&file)
        .output()
        .await
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let policy = werp_core::devices::database().policy(Some("DIW7022"));
    let before = std::fs::read_dir(dir.path()).unwrap().count();
    let preview = conversion::preview(
        &file,
        &Default::default(),
        policy,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        preview.planned_target.unwrap().streams[0].codec.as_deref(),
        Some("hevc")
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        before,
        "preview must not prepare any files"
    );
    let copied = convert_with(&file, policy).await;
    assert_eq!(
        copied.operation.as_deref(),
        Some("Copying video and audio into MP4")
    );
    let target = copied.target.unwrap();
    assert_eq!(target.streams[0].codec.as_deref(), Some("hevc"));
    assert_eq!(target.streams[1].channels, Some(6));
    assert!(convert_with(&target.path, policy).await.already_compatible);
    // Normal casting with this model sees the same recipe, without an encoder.
    let source = werp_core::media::inspect(&file, &Default::default(), &CancellationToken::new())
        .await
        .unwrap();
    let mut reused = false;
    let cached = werp_core::cache::prepare(
        &source,
        Path::new("/nonexistent/ffmpeg"),
        &Default::default(),
        &werp_core::transcode::TranscodeOptions {
            mode: werp_core::transcode::TranscodeMode::Remux,
            playback_policy: policy,
            ..Default::default()
        },
        &Default::default(),
        &CancellationToken::new(),
        |event| reused |= matches!(event, werp_core::cache::Event::Reused(_)),
    )
    .await
    .unwrap();
    assert!(reused);
    assert_eq!(cached.info.path, target.path);
    cached.close().unwrap();
    // Tightening a model through an override must change its preparation recipe.
    let restricted = werp_core::devices::database().with_overrides("schema_version=1\n[[devices]]\nid='KPN DIW7022'\n[devices.playback]\nallow_aac_surround=false").unwrap().policy(Some("DIW7022"));
    let audio = convert_with(&file, restricted).await;
    assert_eq!(
        audio.operation.as_deref(),
        Some("Copying video; converting audio to stereo AAC")
    );
    let audio_target = audio.target.unwrap();
    assert_eq!(audio_target.streams[0].codec.as_deref(), Some("hevc"));
    assert_eq!(audio_target.streams[1].channels, Some(2));
    let baseline = convert(&file).await;
    assert_eq!(
        baseline.target.unwrap().streams[0].codec.as_deref(),
        Some("h264")
    );
    let repeated = convert(&file).await;
    assert!(repeated.reused);
    assert_eq!(repeated.target.unwrap().streams[1].channels, Some(2));
}
