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
        let mut command = Command::new(env!("CARGO_BIN_EXE_yeet-backend"));
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
fn handshake_inspection_framing_validation_and_explicit_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let mut metadata: Value =
        serde_json::from_str(include_str!("../../yeet-core/tests/fixtures/h264.json")).unwrap();
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
    let config = state.with_extension("config").join("yeet");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("config.toml"), "invalid = [").unwrap();
    let mut helper = Helper::new(Some(&probe), &state);
    helper.send(json!({"id":1,"method":"hello","params":{"version":99}}));
    assert_eq!(helper.read()["error"]["code"], "protocol_version");
    helper.send(json!({"id":2,"method":"hello","params":{"version":1}}));
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
    helper.write("pect\",\"params\":{\"file\":\"/nonexistent/yeet-video\"}}\n");
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
    helper.send(json!({"id":1,"method":"hello","params":{"version":1}}));
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
        include_str!("../../yeet-core/tests/fixtures/h264.json"),
    )
    .unwrap();
    let file = dir.path().join("video.mp4");
    fs::write(&file, "fixture").unwrap();
    let state = dir.path().join("state");
    let mut helper = Helper::with_ffmpeg(Some(&probe), Some(&dir.path().join("no-ffmpeg")), &state);
    helper.send(json!({"id":1,"method":"hello","params":{"version":1}}));
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
            serde_json::from_str(include_str!("../../yeet-core/tests/fixtures/h264.json")).unwrap();
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
        helper.send(json!({"id":1,"method":"hello","params":{"version":1}}));
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
            name.starts_with("session-") || name.contains(".yeet-")
        }));
    }
}
