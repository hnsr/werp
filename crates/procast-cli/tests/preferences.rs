#![cfg(unix)]
use std::{fs, net::TcpListener, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn config_location_language_order_and_cli_subtitle_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join("config/procast");
    fs::create_dir_all(&config_dir).unwrap();
    let config = config_dir.join("config.toml");
    let video = dir.path().join("movie.mp4");
    fs::write(&video, "video").unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let mut metadata: serde_json::Value =
        serde_json::from_str(include_str!("../../procast-core/tests/fixtures/h264.json")).unwrap();
    for (index, language) in [(2, "eng"), (3, "dut")] {
        metadata["streams"].as_array_mut().unwrap().push(serde_json::json!({"index":index,"codec_type":"subtitle","codec_name":"subrip","tags":{"language":language}}));
    }
    fs::write(dir.path().join("probe.json"), metadata.to_string()).unwrap();
    let converter = dir.path().join("converter");
    fs::write(&converter, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\nprintf 'WEBVTT\\n\\n00:00.000 --> 00:01.000\\nCaption\\n'\n").unwrap();
    fs::set_permissions(&converter, fs::Permissions::from_mode(0o700)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let run = |extra: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_procast"))
            .arg("cast")
            .arg(&video)
            .args([
                "--host",
                "127.0.0.1",
                "--cast-port",
                &port.to_string(),
                "--no-inhibit-sleep",
                "--no-resume",
            ])
            .arg("--ffprobe")
            .arg(&probe)
            .arg("--ffmpeg")
            .arg(&converter)
            .args(extra)
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_STATE_HOME", dir.path().join("state"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        String::from_utf8(output.stderr).unwrap()
    };
    fs::write(&config, "[subtitles]\nlanguages=['nl','en']\n").unwrap();
    assert!(run(&[]).contains("embedded stream #3"));
    assert!(
        fs::read_to_string(dir.path().join("converter.args"))
            .unwrap()
            .contains("0:3\n")
    );
    assert!(run(&["--subtitle-track", "2"]).contains("embedded stream #2"));
    assert!(run(&["--no-subtitles"]).contains("Subtitles: none selected"));
    assert!(run(&["--no-config"]).contains("embedded stream #2"));
    fs::write(&config, "[subtitles]\nauto_load=false\n").unwrap();
    assert!(run(&[]).contains("Subtitles: none selected"));
    assert!(run(&["--auto-subtitles"]).contains("embedded stream #2"));
    assert!(run(&["--subtitles", "missing.vtt"]).contains("missing.vtt"));
    let alternate = dir.path().join("alternate.toml");
    fs::write(&alternate, "[subtitles]\nlanguages=['nl']\n").unwrap();
    assert!(run(&["--config", alternate.to_str().unwrap()]).contains("embedded stream #3"));
    fs::write(&config, "[subtitles]\nauto_lod=true\n").unwrap();
    assert!(run(&[]).contains("invalid configuration"));
}
