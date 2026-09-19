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
        let mut command = Command::new(env!("CARGO_BIN_EXE_yeet-backend"));
        command
            .arg("--no-inhibit-sleep")
            .env("XDG_STATE_HOME", state)
            .env("XDG_CONFIG_HOME", state.with_extension("config"));
        if let Some(probe) = probe {
            command.arg("--ffprobe").arg(probe);
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
    assert!(reply["result"]["resume_position"].is_null());
    assert!(
        !state.exists(),
        "inspection must not create checkpoint state"
    );
    helper.write("pect\",\"params\":{\"file\":\"/nonexistent/yeet-video\"}}\n");
    assert_eq!(helper.read()["id"], 4);
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
