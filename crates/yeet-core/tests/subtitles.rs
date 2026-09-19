#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};
use yeet_core::{CancellationToken, YeetError, subtitles};

#[tokio::test]
async fn rejects_missing_converter_non_utf8_and_invalid_cues() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("captions.srt");
    let missing = dir.path().join("missing-ffmpeg");
    let token = CancellationToken::new();
    fs::write(&path, "1\n00:00:00,000 --> 00:00:01,000\nHello\n").unwrap();
    assert!(matches!(
        subtitles::prepare(&path, &missing, &token).await,
        Err(YeetError::MissingExecutable(_))
    ));
    for bytes in [
        b"\xff".as_slice(),
        b"1\n00:00:02,000 --> 00:00:01,000\nHello\n",
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            subtitles::prepare(&path, &missing, &token).await,
            Err(YeetError::Subtitles(_))
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
    assert!(matches!(result, Err(YeetError::Cancelled)));
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

#[tokio::test]
#[ignore = "requires real ffmpeg/ffprobe with libx264 and subtitle codecs"]
async fn embedded_languages_mp4_text_and_ass_extract_to_timed_webvtt() {
    use yeet_core::{
        config::SubtitlePreferences,
        media::{self, ProbeOptions},
    };
    async fn ffmpeg(args: &[&std::ffi::OsStr]) {
        let output = tokio::process::Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error"])
            .args(args)
            .output()
            .await
            .expect("ffmpeg required");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let english = dir.path().join("English ü.srt");
    let dutch = dir.path().join("Dutch.srt");
    let forced = dir.path().join("Forced.srt");
    for (path, text) in [
        (&english, "ENGLISH FULL"),
        (&dutch, "NEDERLANDS"),
        (&forced, "FORCED ONLY"),
    ] {
        fs::write(path, format!("1\n00:00:01,000 --> 00:00:02,000\n{text}\n")).unwrap();
    }
    let mkv = dir.path().join("embedded ; subtitles.mkv");
    ffmpeg(&[
        "-f".as_ref(),
        "lavfi".as_ref(),
        "-i".as_ref(),
        "color=size=160x90:rate=15".as_ref(),
        "-i".as_ref(),
        english.as_os_str(),
        "-i".as_ref(),
        dutch.as_os_str(),
        "-i".as_ref(),
        forced.as_os_str(),
        "-t".as_ref(),
        "4".as_ref(),
        "-map".as_ref(),
        "0:v".as_ref(),
        "-map".as_ref(),
        "1:s".as_ref(),
        "-map".as_ref(),
        "2:s".as_ref(),
        "-map".as_ref(),
        "3:s".as_ref(),
        "-c:v".as_ref(),
        "libx264".as_ref(),
        "-pix_fmt".as_ref(),
        "yuv420p".as_ref(),
        "-c:s".as_ref(),
        "srt".as_ref(),
        "-metadata:s:s:0".as_ref(),
        "language=eng".as_ref(),
        "-metadata:s:s:1".as_ref(),
        "language=nld".as_ref(),
        "-metadata:s:s:2".as_ref(),
        "language=eng".as_ref(),
        "-metadata:s:s:2".as_ref(),
        "title=Forced".as_ref(),
        "-disposition:s:0".as_ref(),
        "0".as_ref(),
        "-disposition:s:2".as_ref(),
        "default+forced".as_ref(),
        mkv.as_os_str(),
    ])
    .await;
    let token = CancellationToken::new();
    let probe = ProbeOptions::default();
    let info = media::inspect(&mkv, &probe, &token).await.unwrap();
    for (languages, index, text) in [
        (vec!["en".into(), "nl".into()], 1, "ENGLISH FULL"),
        (vec!["nl".into(), "en".into()], 2, "NEDERLANDS"),
    ] {
        let selected = subtitles::select(
            &subtitles::Request::Auto,
            &info,
            &mkv,
            &SubtitlePreferences {
                auto_load: true,
                languages,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            selected,
            Some(subtitles::Selection::Text {
                index,
                styled: false
            })
        );
        let prepared = subtitles::extract(&info, index, Path::new("ffmpeg"), &token)
            .await
            .unwrap();
        let vtt = fs::read_to_string(&prepared.path).unwrap();
        assert!(
            vtt.contains(text) && vtt.contains("00:01.000 --> 00:02.000"),
            "{vtt}"
        );
        prepared.close().unwrap();
    }
    for (extension, codec) in [("mp4", "mov_text"), ("mkv", "ass")] {
        let output = dir.path().join(format!("converted.{extension}"));
        ffmpeg(&[
            "-i".as_ref(),
            mkv.as_os_str(),
            "-map".as_ref(),
            "0:v".as_ref(),
            "-map".as_ref(),
            "0:1".as_ref(),
            "-c:v".as_ref(),
            "copy".as_ref(),
            "-c:s".as_ref(),
            codec.as_ref(),
            output.as_os_str(),
        ])
        .await;
        let info = media::inspect(&output, &probe, &token).await.unwrap();
        let track = info.streams.iter().find(|s| s.kind == "subtitle").unwrap();
        assert_eq!(track.codec.as_deref(), Some(codec));
        let prepared = subtitles::extract(&info, track.index, Path::new("ffmpeg"), &token)
            .await
            .unwrap();
        assert!(
            fs::read_to_string(&prepared.path)
                .unwrap()
                .contains("ENGLISH FULL")
        );
        prepared.close().unwrap();
    }
    let ass = dir.path().join("external.ass");
    ffmpeg(&[
        "-i".as_ref(),
        english.as_os_str(),
        "-c:s".as_ref(),
        "ass".as_ref(),
        ass.as_os_str(),
    ])
    .await;
    let prepared = subtitles::prepare(&ass, Path::new("ffmpeg"), &token)
        .await
        .unwrap();
    assert!(
        fs::read_to_string(&prepared.path)
            .unwrap()
            .contains("ENGLISH FULL")
    );
    prepared.close().unwrap();
}
