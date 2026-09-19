use std::path::Path;
use tokio::sync::watch;
use yeet_core::{
    CancellationToken,
    conversion::{self, Phase, State},
};

async fn convert(file: &Path) -> State {
    let (updates, state) = watch::channel(State::default());
    conversion::run(
        conversion::Request {
            file: file.into(),
            probe: Default::default(),
            ffmpeg: "ffmpeg".into(),
            inhibit_sleep: false,
            cache: Default::default(),
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
