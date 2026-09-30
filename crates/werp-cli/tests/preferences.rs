#![cfg(unix)]
use std::{fs, net::TcpListener, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn shared_http_port_and_cli_overrides_reach_the_media_server() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join("config/werp");
    fs::create_dir_all(&config_dir).unwrap();
    let config = config_dir.join("config.toml");
    let alternate = dir.path().join("alternate.toml");
    let video = dir.path().join("movie.mp4");
    fs::write(&video, "video").unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        dir.path().join("probe.json"),
        include_str!("../../werp-core/tests/fixtures/h264.json"),
    )
    .unwrap();
    // An occupied HTTP port fails before any Cast connection. A closed local
    // Cast port makes the OS-assigned cases terminate without contacting a TV.
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = occupied.local_addr().unwrap().port();
    let cast = TcpListener::bind("127.0.0.1:0").unwrap();
    let cast_port = cast.local_addr().unwrap().port();
    drop(cast);
    let run = |extra: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_werp"))
            .arg(&video)
            .args([
                "--host",
                "127.0.0.1",
                "--cast-port",
                &cast_port.to_string(),
                "--bind-address",
                "127.0.0.1",
                "--no-inhibit-sleep",
                "--no-resume",
                "--no-subtitles",
            ])
            .arg("--ffprobe")
            .arg(&probe)
            .args(extra)
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_STATE_HOME", dir.path().join("state"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        String::from_utf8(output.stderr).unwrap()
    };
    let assert_refused = |stderr: String| {
        assert!(stderr.contains("Connection refused"), "{stderr}");
        assert!(!stderr.contains("Address already in use"), "{stderr}");
    };
    assert_refused(run(&[])); // Missing config defaults to OS assignment.
    fs::write(&config, format!("http_port = {port}\n")).unwrap();
    let stderr = run(&[]);
    assert!(stderr.contains("Address already in use"), "{stderr}");
    assert_refused(run(&["--http-port", "0"]));
    assert_refused(run(&["--no-config"]));
    fs::write(&alternate, "http_port = 0\n").unwrap();
    assert_refused(run(&["--config", alternate.to_str().unwrap()]));
    fs::write(&config, "http_port = 0\n").unwrap();
    let stderr = run(&["--http-port", &port.to_string()]);
    assert!(stderr.contains("Address already in use"), "{stderr}");
}

#[test]
fn config_location_language_order_and_cli_subtitle_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join("config/werp");
    fs::create_dir_all(&config_dir).unwrap();
    let config = config_dir.join("config.toml");
    let video = dir.path().join("movie.mp4");
    fs::write(&video, "video").unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let mut metadata: serde_json::Value =
        serde_json::from_str(include_str!("../../werp-core/tests/fixtures/h264.json")).unwrap();
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
        let output = Command::new(env!("CARGO_BIN_EXE_werp"))
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
    fs::write(&config, "[cli.subtitles]\nlanguages=['nl','en']\n").unwrap();
    assert!(run(&[]).contains("embedded stream #3"));
    assert!(
        fs::read_to_string(dir.path().join("converter.args"))
            .unwrap()
            .contains("0:3\n")
    );
    assert!(run(&["--subtitle-track", "2"]).contains("embedded stream #2"));
    assert!(run(&["--no-subtitles"]).contains("Subtitles: none selected"));
    assert!(run(&["--no-config"]).contains("embedded stream #2"));
    fs::write(&config, "[cli.subtitles]\nauto_load=false\n").unwrap();
    assert!(run(&[]).contains("Subtitles: none selected"));
    assert!(run(&["--auto-subtitles"]).contains("embedded stream #2"));
    assert!(run(&["--subtitles", "missing.vtt"]).contains("missing.vtt"));
    let alternate = dir.path().join("alternate.toml");
    fs::write(&alternate, "[cli.subtitles]\nlanguages=['nl']\n").unwrap();
    assert!(run(&["--config", alternate.to_str().unwrap()]).contains("embedded stream #3"));
    fs::write(&config, "[cli.subtitles]\nlanguages=['nl']\n").unwrap();
    let overrides = config_dir.join("devices.toml");
    fs::write(&overrides, "schema_version=1\nunknown=true").unwrap();
    assert!(run(&[]).contains("device database"));
    assert!(run(&["--config", alternate.to_str().unwrap()]).contains("device database"));
    assert!(run(&["--no-config"]).contains("embedded stream #2"));
    assert!(run(&["--profile", "baseline"]).contains("embedded stream #3"));
    assert!(run(&["--profile", "extended"]).contains("embedded stream #3"));
    fs::write(&overrides, "schema_version=1").unwrap();
    assert!(run(&[]).contains("embedded stream #3"));
    fs::write(&config, "[cli.subtitles]\nauto_lod=true\n").unwrap();
    assert!(run(&[]).contains("invalid configuration"));
}
