#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Helper {
    child: Child,
    replies: mpsc::Receiver<Value>,
}
impl Helper {
    fn new(probe: Option<&Path>, state: &Path) -> Self {
        Self::with_ffmpeg(probe, None, state)
    }
    fn with_ffmpeg(probe: Option<&Path>, ffmpeg: Option<&Path>, state: &Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_werp-backend"));
        command
            .arg("--no-inhibit-sleep")
            .env("XDG_STATE_HOME", state)
            .env("XDG_CONFIG_HOME", state.with_extension("config"));
        if let Some(probe) = probe {
            command.arg("--ffprobe").arg(probe);
        }
        if let Some(ffmpeg) = ffmpeg {
            command.arg("--ffmpeg").arg(ffmpeg);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if tx
                    .send(serde_json::from_str(&line).expect("stdout must contain JSON only"))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self { child, replies }
    }
    fn write(&mut self, text: &str) {
        let input = self.child.stdin.as_mut().unwrap();
        input.write_all(text.as_bytes()).unwrap();
        input.flush().unwrap();
    }
    fn send(&mut self, value: Value) {
        self.write(&(value.to_string() + "\n"));
    }
    fn read(&self) -> Value {
        self.replies
            .recv_timeout(Duration::from_secs(5))
            .expect("helper response timeout")
    }
    fn wait(&mut self) {
        let start = std::time::Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "helper survived shutdown"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn shared_config_is_validated_at_start_and_can_be_corrected_without_restart() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let path = state.with_extension("config").join("werp/config.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "http_port = 65536\n").unwrap();
    let mut helper = Helper::new(None, &state);
    helper.send(json!({"id":1,"method":"hello","params":{"version":2}}));
    assert_eq!(helper.read()["ok"], true);
    let start = |id| json!({"id":id,"method":"start","params":{"file":"video.mp4","device_id":"test-id","subtitles":{"kind":"none"},"position":0}});
    helper.send(start(2));
    let reply = helper.read();
    assert_eq!(reply["error"]["code"], "config_error");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("http_port")
    );
    fs::write(
        &path,
        "http_port = 8010\n[cli.playback]\nauto_resume = false\n",
    )
    .unwrap();
    helper.send(start(3));
    // Validation proceeds to device selection without discovering real TVs.
    assert_eq!(helper.read()["error"]["code"], "invalid_device");
    fs::remove_file(&path).unwrap();
    helper.send(start(4));
    assert_eq!(helper.read()["error"]["code"], "invalid_device");
    helper.send(json!({"id":5,"method":"shutdown"}));
    assert_eq!(helper.read()["ok"], true);
    helper.wait();
}

#[test]
fn gui_preferences_use_shared_config_and_save_only_state() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let config = state.with_extension("config").join("werp/config.toml");
    let saved_state = state.join("werp/gui.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let contents = "# Keep this comment and formatting\nhttp_port = 8010\n[cli.playback]\nauto_resume = false\n[gui.conversion]\nauto_close = false\nauto_clsoe = true\n";
    fs::write(&config, contents).unwrap();
    // Legacy files are deliberately ignored, with no migration or fallback.
    fs::write(
        config.with_file_name("gui.toml"),
        "last_device_id='legacy'\n",
    )
    .unwrap();
    let mut helper = Helper::new(None, &state);
    helper.send(json!({"id":1,"method":"hello","params":{"version":2}}));
    assert_eq!(helper.read()["ok"], true);
    helper.send(json!({"id":2,"method":"get_gui_preferences","params":{}}));
    let initial = helper.read();
    assert_eq!(initial["result"]["last_device_id"], "");
    assert_eq!(initial["result"]["conversion_auto_close"], false);
    assert!(
        initial["result"]["warnings"][0]
            .as_str()
            .unwrap()
            .contains("gui.conversion.auto_clsoe")
    );
    helper.send(json!({"id":3,"method":"set_gui_last_device","params":{"device_id":"bedroom"}}));
    assert_eq!(helper.read()["result"]["last_device_id"], "bedroom");
    assert_eq!(fs::read_to_string(&config).unwrap(), contents);
    assert_eq!(
        fs::read_to_string(&saved_state).unwrap(),
        "last_device_id = \"bedroom\"\n"
    );
    assert_eq!(
        fs::metadata(&saved_state).unwrap().permissions().mode() & 0o777,
        0o600
    );
    helper.send(json!({"id":4,"method":"shutdown"}));
    assert_eq!(helper.read()["ok"], true);
    helper.wait();

    let mut helper = Helper::new(None, &state);
    helper.send(json!({"id":1,"method":"hello","params":{"version":2}}));
    assert_eq!(helper.read()["ok"], true);
    helper.send(json!({"id":2,"method":"get_gui_preferences","params":{"path":null}}));
    assert_eq!(helper.read()["result"]["last_device_id"], "bedroom");
    let alternate = dir.path().join("alternate.toml");
    fs::write(&alternate, "[gui.conversion]\nauto_close=true\n").unwrap();
    helper.send(json!({"id":3,"method":"get_gui_preferences","params":{"path":alternate}}));
    let reply = helper.read();
    assert_eq!(reply["result"]["conversion_auto_close"], true);
    assert_eq!(reply["result"]["last_device_id"], "bedroom");
    fs::write(&alternate, "[gui.conversion]\nauto_close='invalid'\n").unwrap();
    helper.send(json!({"id":4,"method":"get_gui_preferences","params":{"path":alternate}}));
    assert_eq!(helper.read()["error"]["code"], "config_error");
    helper.send(json!({"id":5,"method":"shutdown"}));
    assert_eq!(helper.read()["ok"], true);
    helper.wait();
}

#[test]
fn missing_config_defaults_and_bad_state_is_nonfatal_and_repairable() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let saved_state = state.join("werp/gui.toml");
    let mut helper = Helper::new(None, &state);
    helper.send(json!({"id":1,"method":"hello","params":{"version":2}}));
    assert_eq!(helper.read()["ok"], true);
    helper.send(json!({"id":2,"method":"get_gui_preferences","params":{"path":null}}));
    let reply = helper.read();
    assert_eq!(reply["result"]["conversion_auto_close"], true);
    assert_eq!(reply["result"]["warnings"], json!([]));
    assert!(!saved_state.exists());
    fs::create_dir_all(saved_state.parent().unwrap()).unwrap();
    fs::write(&saved_state, "last_device_id = [").unwrap();
    helper.send(json!({"id":3,"method":"get_gui_preferences","params":{}}));
    let reply = helper.read();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["result"]["last_device_id"], "");
    assert_eq!(reply["result"]["warnings"].as_array().unwrap().len(), 1);
    helper.send(json!({"id":4,"method":"set_gui_last_device","params":{"device_id":"tv"}}));
    assert_eq!(helper.read()["ok"], true);
    helper.send(json!({"id":5,"method":"get_gui_preferences","params":{}}));
    assert_eq!(helper.read()["result"]["last_device_id"], "tv");
    helper.send(json!({"id":6,"method":"get_gui_preferences","params":{"path":dir.path().join("missing.toml")}}));
    assert_eq!(helper.read()["error"]["code"], "config_error");
    helper.send(json!({"id":7,"method":"shutdown"}));
    assert_eq!(helper.read()["ok"], true);
    helper.wait();
}
#[test]
fn inspection_uses_shared_subtitle_preferences_without_hiding_manual_choices() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let mut metadata: Value =
        serde_json::from_str(include_str!("../../werp-core/tests/fixtures/h264.json")).unwrap();
    for (index, language) in [(2, "eng"), (3, "dut")] {
        metadata["streams"].as_array_mut().unwrap().push(json!({
            "index":index,"codec_type":"subtitle","codec_name":"subrip","tags":{"language":language}
        }));
    }
    fs::write(dir.path().join("probe.json"), metadata.to_string()).unwrap();
    let file = dir.path().join("video.mp4");
    fs::write(&file, "source").unwrap();
    fs::write(
        file.with_extension("srt"),
        "1\n00:00:00,000 --> 00:00:01,000\nCaption\n",
    )
    .unwrap();
    let state = dir.path().join("state");
    let config = state.with_extension("config").join("werp/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let mut helper = Helper::new(Some(&probe), &state);
    helper.send(json!({"id":1,"method":"hello","params":{"version":2}}));
    assert_eq!(helper.read()["ok"], true);
    for (settings, expected) in [
        ("", json!({"kind":"embedded","index":2})),
        (
            "[subtitles]\nlanguages=['Dutch','English']\n",
            json!({"kind":"embedded","index":3}),
        ),
        (
            "[subtitles]\nauto_load=false\nlanguages=['nl']\n",
            json!({"kind":"none"}),
        ),
        (
            "[subtitles]\nlanguages=[]\n",
            json!({"kind":"external","path":file.with_extension("srt")}),
        ),
        (
            "[subtitles]\nlanguages=['nl']\nfuture_option=true\n",
            json!({"kind":"embedded","index":3}),
        ),
    ] {
        fs::write(&config, settings).unwrap();
        helper.send(json!({"id":2,"method":"inspect","params":{"file":file}}));
        let reply = helper.read();
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(
            reply["result"]["suggested_subtitles"], expected,
            "{settings}"
        );
        assert_eq!(reply["result"]["subtitles"].as_array().unwrap().len(), 3);
        assert!(reply["result"]["subtitle_warning"].is_null(), "{reply}");
    }
    for invalid in [
        "[subtitles]\nauto_load='yes'",
        "[subtitles]\nlanguages=['unsupported']",
        "invalid = [",
    ] {
        fs::write(&config, invalid).unwrap();
        helper.send(json!({"id":3,"method":"inspect","params":{"file":file}}));
        let reply = helper.read();
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(
            reply["result"]["suggested_subtitles"],
            json!({"kind":"none"})
        );
        assert_eq!(reply["result"]["subtitles"].as_array().unwrap().len(), 3);
        assert!(
            reply["result"]["subtitle_warning"]
                .as_str()
                .unwrap()
                .contains("config.toml")
        );
    }
    assert!(!state.exists(), "inspection must not write state");
    helper.send(json!({"id":4,"method":"shutdown"}));
    assert_eq!(helper.read()["ok"], true);
    helper.wait();
}

#[test]
fn handshake_inspection_framing_validation_and_explicit_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let mut metadata: Value =
        serde_json::from_str(include_str!("../../werp-core/tests/fixtures/h264.json")).unwrap();
    metadata["streams"].as_array_mut().unwrap().push(
        json!({"index":2,"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"eng"}}),
    );
    fs::write(dir.path().join("probe.json"), metadata.to_string()).unwrap();
    let file = dir.path().join("video with\nnewline.mp4");
    fs::write(&file, "source").unwrap();
    fs::write(
        file.with_extension("srt"),
        "1\n00:00:00,000 --> 00:00:01,000\nCaption\n",
    )
    .unwrap();
    fs::write(
        file.with_extension("vtt"),
        "WEBVTT\n\n00:00.000 --> 00:01.000\nCaption\n",
    )
    .unwrap();
    let subtitle_target = dir.path().join("subtitle-without-extension");
    fs::write(&subtitle_target, "caption fixture").unwrap();
    std::os::unix::fs::symlink(&subtitle_target, file.with_extension("ssa")).unwrap();
    let state = dir.path().join("state");
    let config = state.with_extension("config").join("werp");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("config.toml"),
        "[cli.playback]\nauto_resume=false\n",
    )
    .unwrap();
    let mut helper = Helper::new(Some(&probe), &state);
    helper.send(json!({"id":1,"method":"hello","params":{"version":99}}));
    assert_eq!(helper.read()["error"]["code"], "protocol_version");
    helper.send(json!({"id":2,"method":"hello","params":{"version":2}}));
    assert_eq!(helper.read()["ok"], true);
    helper.send(json!({"id":3,"method":"inspect","params":{"file":file}}));
    // An unrelated completion must not discard a partially received frame.
    helper.write("{\"id\":4,\"method\":\"ins");
    let reply = helper.read();
    assert_eq!(reply["id"], 3);
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["result"]["subtitles"].as_array().unwrap().len(), 4);
    assert!(
        reply["result"]["subtitles"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |choice| choice["path"] == subtitle_target.to_string_lossy().as_ref()
                    && choice["codec"].is_null()
            )
    );
    assert_eq!(
        reply["result"]["suggested_subtitles"],
        json!({"kind":"embedded","index":2})
    );
    assert!(reply["result"]["resume_position"].is_null());
    assert!(
        !state.exists(),
        "inspection must not create checkpoint state"
    );
    helper.write("pect\",\"params\":{\"file\":\"/nonexistent/werp-video\"}}\n");
    assert_eq!(helper.read()["id"], 4);
    metadata["streams"].as_array_mut().unwrap().pop();
    fs::write(dir.path().join("probe.json"), metadata.to_string()).unwrap();
    helper.send(json!({"id":8,"method":"inspect","params":{"file":file}}));
    assert_eq!(
        helper.read()["result"]["suggested_subtitles"],
        json!({"kind":"external","path":file.with_extension("srt")})
    );
    fs::write(file.with_extension("SRT"), "duplicate").unwrap();
    helper.send(json!({"id":9,"method":"inspect","params":{"file":file}}));
    let ambiguous = helper.read();
    assert_eq!(ambiguous["ok"], true);
    assert_eq!(
        ambiguous["result"]["suggested_subtitles"],
        json!({"kind":"none"})
    );
    assert!(ambiguous["result"]["subtitle_warning"].is_string());
    fs::remove_file(file.with_extension("srt")).unwrap();
    fs::remove_file(file.with_extension("SRT")).unwrap();
    helper.send(json!({"id":10,"method":"inspect","params":{"file":file}}));
    assert_eq!(
        helper.read()["result"]["suggested_subtitles"],
        json!({"kind":"none"})
    );
    helper.send(json!({"id":5,"method":"pause","params":{"session_id":1}}));
    assert_eq!(helper.read()["error"]["code"], "invalid_session");
    // Inspection ignores malformed config; playback now validates shared settings.
    fs::remove_file(config.join("config.toml")).unwrap();
    helper.send(json!({"id":6,"method":"start","params":{"file":file,"device_id":"missing","subtitles":{"kind":"none"},"position":0}}));
    assert_eq!(helper.read()["error"]["code"], "invalid_device");
    helper.send(json!({"id":7,"method":"shutdown"}));
    assert_eq!(helper.read()["ok"], true);
    helper.wait();
}
#[test]
fn eof_cancels_and_reaps_an_active_probe() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\necho $$ > \"$0.pid\"\nexec sleep 60\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let file = dir.path().join("video");
    fs::write(&file, "source").unwrap();
    let mut helper = Helper::new(Some(&probe), &dir.path().join("state"));
    helper.send(json!({"id":1,"method":"hello","params":{"version":2}}));
    assert_eq!(helper.read()["ok"], true);
    helper.send(json!({"id":2,"method":"inspect","params":{"file":file}}));
    let deadline = std::time::Instant::now();
    while !dir.path().join("probe.pid").exists() {
        assert!(deadline.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = fs::read_to_string(dir.path().join("probe.pid")).unwrap();
    drop(helper.child.stdin.take());
    helper.wait();
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
}

#[test]
fn conversion_reports_noop_and_failure_without_device_or_resume_state() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        dir.path().join("probe.json"),
        include_str!("../../werp-core/tests/fixtures/h264.json"),
    )
    .unwrap();
    let file = dir.path().join("video.mp4");
    fs::write(&file, "fixture").unwrap();
    let state = dir.path().join("state");
    let mut helper = Helper::with_ffmpeg(Some(&probe), Some(&dir.path().join("no-ffmpeg")), &state);
    helper.send(json!({"id":1,"method":"hello","params":{"version":2}}));
    assert_eq!(helper.read()["ok"], true);
    for (id, source, phase) in [
        (2, file.clone(), "completed"),
        (3, dir.path().join("missing"), "failed"),
    ] {
        helper.send(json!({"id":id,"method":"convert","params":{"file":source}}));
        let ack = helper.read();
        assert_eq!(ack["id"], id);
        assert_eq!(ack["ok"], true);
        let operation = ack["result"]["operation_id"].clone();
        loop {
            let event = helper.read();
            assert_eq!(event["operation_id"], operation);
            if event["event"] == "conversion_ended" {
                assert_eq!(event["state"]["phase"], phase);
                if phase == "completed" {
                    assert_eq!(event["state"]["already_compatible"], true);
                    assert_eq!(event["state"]["output"], json!(file));
                } else {
                    assert!(event["state"]["error"].is_string());
                }
                break;
            }
            assert_eq!(event["event"], "conversion_state");
        }
    }
    assert!(!state.exists());
    helper.send(json!({"id":4,"method":"shutdown"}));
    assert_eq!(helper.read()["ok"], true);
    helper.wait();
}

#[test]
fn conversion_cancel_shutdown_and_eof_reap_encoder_and_remove_partial_output() {
    for action in ["cancel_conversion", "shutdown", "eof"] {
        let dir = tempfile::tempdir().unwrap();
        let probe = dir.path().join("probe");
        fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
        let mut metadata: Value =
            serde_json::from_str(include_str!("../../werp-core/tests/fixtures/h264.json")).unwrap();
        metadata["streams"][0]["codec_name"] = json!("vp9");
        fs::write(dir.path().join("probe.json"), metadata.to_string()).unwrap();
        let ffmpeg = dir.path().join("ffmpeg");
        fs::write(
            &ffmpeg,
            r#"#!/bin/sh
case "$*" in *-encoders*) printf ' V libx264\n A aac\n'; exit 0;; esac
for last do :; done
printf 'incomplete' > "$last"
echo $$ > "$0.pid"
exec sleep 60
"#,
        )
        .unwrap();
        fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o700)).unwrap();
        let file = dir.path().join("video.mp4");
        fs::write(&file, "source").unwrap();
        let state = dir.path().join("state");
        let mut helper = Helper::with_ffmpeg(Some(&probe), Some(&ffmpeg), &state);
        helper.send(json!({"id":1,"method":"hello","params":{"version":2}}));
        assert_eq!(helper.read()["ok"], true);
        helper.send(json!({"id":2,"method":"convert","params":{"file":file}}));
        let ack = helper.read();
        assert_eq!(ack["id"], 2);
        assert_eq!(ack["ok"], true);
        let operation = ack["result"]["operation_id"].clone();
        let deadline = std::time::Instant::now();
        let pid_file = dir.path().join("ffmpeg.pid");
        while !pid_file.exists() {
            assert!(deadline.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid = fs::read_to_string(pid_file).unwrap();
        helper.send(json!({"id":3,"method":"convert","params":{"file":file}}));
        loop {
            let reply = helper.read();
            if reply["id"] == 3 {
                assert_eq!(reply["error"]["code"], "busy");
                break;
            }
        }
        if action == "eof" {
            drop(helper.child.stdin.take());
        } else {
            if action == "shutdown" {
                helper.send(json!({"id":4,"method":action}));
            } else {
                helper.send(json!({"id":4,"method":action,"params":{"operation_id":operation}}));
            }
            loop {
                let reply = helper.read();
                if reply["id"] == 4 {
                    assert_eq!(reply["ok"], true);
                    break;
                }
            }
            if action == "cancel_conversion" {
                let ended = helper.read();
                assert_eq!(ended["event"], "conversion_ended");
                assert_eq!(ended["state"]["phase"], "cancelled");
                helper.send(json!({"id":5,"method":"shutdown"}));
                assert_eq!(helper.read()["ok"], true);
            }
        }
        helper.wait();
        assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
        assert_eq!(fs::read_to_string(file).unwrap(), "source");
        assert!(!state.exists());
        assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("session-") || name.contains(".werp-")
        }));
    }
}

#[test]
fn convert_uses_baseline_without_reading_subtitle_or_device_settings() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        dir.path().join("probe.json"),
        include_str!("../../werp-core/tests/fixtures/hevc.json"),
    )
    .unwrap();
    let file = dir.path().join("video.mp4");
    fs::write(&file, "source").unwrap();
    // The fixture identifies Matroska; change only the container to test a no-op
    // without starting an encoder or needing a receiver.
    let mut metadata: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("probe.json")).unwrap()).unwrap();
    metadata["format"]["format_name"] = json!("mp4");
    fs::write(dir.path().join("probe.json"), metadata.to_string()).unwrap();
    let state = dir.path().join("state");
    let config = state.with_extension("config").join("werp/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let mut helper = Helper::with_ffmpeg(Some(&probe), Some(&dir.path().join("no-ffmpeg")), &state);
    helper.send(json!({"id":1,"method":"hello","params":{"version":2}}));
    assert_eq!(helper.read()["ok"], true);
    fs::write(&config, "[subtitles]\nlanguages=['unsupported']\n").unwrap();
    fs::write(config.with_file_name("devices.toml"), "invalid TOML").unwrap();
    helper.send(json!({"id":10,"method":"preview_conversion","params":{"file":file}}));
    let preview = helper.read();
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["result"]["source"]["streams"][0]["codec"], "hevc");
    assert_eq!(
        preview["result"]["planned_target"]["streams"][0]["codec"],
        "h264"
    );
    assert_eq!(
        preview["result"]["planned_target"]["streams"][1]["channels"],
        2
    );
    assert!(!preview["result"]["output"].is_string());
    for (id, method) in [(11, "preview_conversion"), (12, "convert")] {
        helper.send(
            json!({"id":id,"method":method,"params":{"file":file,"device_id":"not-discovered"}}),
        );
        let error = helper.read();
        assert_eq!(error["ok"], false);
        assert_eq!(error["error"]["code"], "invalid_device");
    }
    helper.send(json!({"id":2,"method":"convert","params":{"file":file}}));
    assert_eq!(helper.read()["ok"], true);
    loop {
        let event = helper.read();
        if event["event"] == "conversion_ended" {
            // HEVC requires conversion without a target receiver; missing FFmpeg
            // proves it did not skip conversion using unrelated model rules.
            assert_eq!(event["state"]["phase"], "failed", "{event}");
            assert!(
                event["state"]["target_description"]
                    .as_str()
                    .unwrap()
                    .contains("H.264")
            );
            break;
        }
    }
    helper.send(json!({"id":5,"method":"shutdown"}));
    assert_eq!(helper.read()["ok"], true);
    helper.wait();
    assert!(!state.exists());
}
