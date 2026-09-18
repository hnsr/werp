#![cfg(unix)]
//! Complete sessions against a fake TLS receiver that fetches real HTTP resources.
//! These tests never discover or connect to devices on the LAN.
use std::{
    fs,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use procast_core::{
    CancellationToken, ProcastError,
    media::DirectPlayPolicy,
    session::{self, CastRequest, Phase, SessionState, Target},
};
use prost::Message;
use rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};
use tokio_rustls::TlsAcceptor;

#[derive(Clone, PartialEq, Message)]
struct Envelope {
    #[prost(int32, required, tag = "1")]
    protocol_version: i32,
    #[prost(string, required, tag = "2")]
    source_id: String,
    #[prost(string, required, tag = "3")]
    destination_id: String,
    #[prost(string, required, tag = "4")]
    namespace: String,
    #[prost(int32, required, tag = "5")]
    payload_type: i32,
    #[prost(string, optional, tag = "6")]
    payload_utf8: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Complete,
    Transient,
    Reject,
    Disconnect,
    Hold,
    HoldLoad,
    Takeover,
    RejectTracks,
    MissingSubtitles,
    BroadcastFinished,
    BroadcastError,
    BroadcastDuringPoll,
    BroadcastThenDisconnect,
    ForeignFinished,
    ZeroFinished,
    CloseAppDuringPoll,
    ForeignClose,
    CloseAppStillRunning,
    BroadcastCancelled,
    MediaErrorAppGone,
    MediaErrorAppRunning,
}

#[derive(Default, Debug)]
struct Transcript {
    loads: usize,
    stops: Vec<i64>,
    video: Vec<u8>,
    subtitles: Option<Vec<u8>>,
    url: String,
    polls: usize,
    track_commands: usize,
    receiver_polls: usize,
}

fn status(url: &str, state: &str, tracks: bool) -> Value {
    json!({"mediaSessionId":7,"playerState":state,"currentTime":1.0,
        "media":{"contentId":url,"contentType":"video/mp4"},
        "activeTrackIds": if tracks { vec![1] } else { vec![] }})
}

async fn send(stream: &mut (impl AsyncWrite + Unpin), message: &Envelope) -> std::io::Result<()> {
    let bytes = message.encode_to_vec();
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await
}

async fn download(url: &str) -> Vec<u8> {
    let (host, path) = url
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    let mut socket = TcpStream::connect(host).await.unwrap();
    socket.write_all(format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nOrigin: https://www.gstatic.com\r\n\r\n").as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    socket.read_to_end(&mut bytes).await.unwrap();
    let header_end = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let header = String::from_utf8_lossy(&bytes[..header_end]);
    assert!(header.starts_with("HTTP/1.1 200"), "{header}");
    assert!(header.contains("access-control-allow-origin: *"));
    bytes[header_end..].to_vec()
}

async fn receiver(
    mode: Mode,
    loaded: Arc<AtomicBool>,
) -> (SocketAddr, tokio::task::JoinHandle<Transcript>) {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.cert.der().clone()],
                PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
            )
            .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut stream = TlsAcceptor::from(Arc::new(config))
            .accept(socket)
            .await
            .unwrap();
        let mut log = Transcript::default();
        let mut tracks = false;
        let mut state = "PLAYING";
        while let Ok(length) = stream.read_u32().await {
            assert!(length <= 65536);
            let mut bytes = vec![0; length as usize];
            if stream.read_exact(&mut bytes).await.is_err() {
                break;
            }
            let message = Envelope::decode(bytes.as_slice()).unwrap();
            let payload: Value =
                serde_json::from_str(message.payload_utf8.as_deref().unwrap()).unwrap();
            let mut reply = match payload["type"].as_str().unwrap() {
                "CONNECT" | "CLOSE" => continue,
                "PING" => json!({"type":"PONG"}),
                "LAUNCH" => {
                    json!({"type":"RECEIVER_STATUS","status":{"applications":[{"appId":"CC1AD845","sessionId":"app-session","transportId":"app-transport"}]}})
                }
                "LOAD" => {
                    log.loads += 1;
                    log.url = payload["media"]["contentId"].as_str().unwrap().into();
                    loaded.store(true, Ordering::Release);
                    if mode == Mode::Reject {
                        json!({"type":"LOAD_FAILED","detailedErrorCode":104})
                    } else {
                        log.video = download(&log.url).await;
                        if let Some(url) = payload
                            .pointer("/media/tracks/0/trackContentId")
                            .and_then(Value::as_str)
                        {
                            assert_eq!(payload["activeTrackIds"], json!([1]));
                            if mode != Mode::MissingSubtitles {
                                log.subtitles = Some(download(url).await);
                            }
                            tracks = true;
                        } else {
                            assert!(payload["media"].get("tracks").is_none());
                        }
                        if mode == Mode::HoldLoad {
                            continue;
                        }
                        if mode == Mode::Transient {
                            json!({"type":"MEDIA_STATUS","status":[{"mediaSessionId":0,"playerState":"IDLE"}]})
                        } else {
                            json!({"type":"MEDIA_STATUS","status":[status(&log.url,state,tracks)]})
                        }
                    }
                }
                "GET_STATUS" => {
                    if message.namespace == "urn:x-cast:com.google.cast.receiver" {
                        log.receiver_polls += 1;
                        let applications = if matches!(
                            mode,
                            Mode::CloseAppStillRunning | Mode::MediaErrorAppRunning
                        ) {
                            json!([{"appId":"CC1AD845","sessionId":"app-session","transportId":"app-transport"}])
                        } else {
                            json!([])
                        };
                        json!({"type":"RECEIVER_STATUS","status":{"applications":applications,"volume":{"level":1.0,"muted":false}}})
                    } else {
                        log.polls += 1;
                        if matches!(mode, Mode::MediaErrorAppGone | Mode::MediaErrorAppRunning)
                            && log.polls == 1
                        {
                            json!({"type":"INVALID_REQUEST","reason":"media channel unavailable"})
                        } else if mode == Mode::Takeover {
                            let mut foreign = status("another-sender", "PLAYING", false);
                            foreign["mediaSessionId"] = json!(99);
                            json!({"type":"MEDIA_STATUS","status":[foreign]})
                        } else if matches!(
                            mode,
                            Mode::BroadcastFinished
                                | Mode::BroadcastError
                                | Mode::BroadcastDuringPoll
                                | Mode::BroadcastThenDisconnect
                        ) || (mode == Mode::Transient && log.polls == 1)
                        {
                            json!({"type":"MEDIA_STATUS","status":[]})
                        } else {
                            state = if matches!(
                                mode,
                                Mode::Complete
                                    | Mode::MissingSubtitles
                                    | Mode::ForeignFinished
                                    | Mode::ZeroFinished
                                    | Mode::ForeignClose
                            ) || (mode == Mode::Transient && log.polls >= 4)
                            {
                                "IDLE"
                            } else if mode == Mode::Transient && log.polls == 2 {
                                "BUFFERING"
                            } else {
                                "PLAYING"
                            };
                            let mut entry = status(&log.url, state, tracks);
                            if state == "IDLE" {
                                entry["idleReason"] = json!("FINISHED");
                            }
                            json!({"type":"MEDIA_STATUS","status":[entry]})
                        }
                    }
                }
                "EDIT_TRACKS_INFO" => {
                    log.track_commands += 1;
                    if mode == Mode::RejectTracks {
                        json!({"type":"INVALID_REQUEST","reason":"text track rejected"})
                    } else {
                        json!({"type":"MEDIA_STATUS","status":[status(&log.url,state,tracks)]})
                    }
                }
                "STOP" => {
                    log.stops.push(payload["mediaSessionId"].as_i64().unwrap());
                    json!({"type":"MEDIA_STATUS","status":[status(&log.url,"IDLE",tracks)]})
                }
                other => panic!("unexpected request: {other}"),
            };
            if let Some(id) = payload.get("requestId") {
                reply["requestId"] = id.clone();
            }
            let response = Envelope {
                protocol_version: 0,
                source_id: message.destination_id,
                destination_id: message.source_id,
                namespace: message.namespace.clone(),
                payload_type: 0,
                payload_utf8: Some(reply.to_string()),
            };
            // Exercise a FINISHED broadcast while GET_STATUS never replies.
            let pending_poll = matches!(mode, Mode::BroadcastDuringPoll | Mode::CloseAppDuringPoll)
                && payload["type"] == "GET_STATUS"
                && message.namespace == "urn:x-cast:com.google.cast.media";
            if !pending_poll && send(&mut stream, &response).await.is_err() {
                break;
            }
            let after_tracks = payload["type"] == "EDIT_TRACKS_INFO"
                && matches!(
                    mode,
                    Mode::BroadcastFinished
                        | Mode::BroadcastError
                        | Mode::BroadcastThenDisconnect
                        | Mode::ForeignFinished
                        | Mode::ZeroFinished
                        | Mode::BroadcastCancelled
                );
            if after_tracks || (pending_poll && mode == Mode::BroadcastDuringPoll) {
                let mut entry = json!({"mediaSessionId":7,"playerState":"IDLE","idleReason":"FINISHED","currentTime":0});
                if mode == Mode::ForeignFinished {
                    entry["media"] = json!({"contentId":"another-senders-file"});
                } else if mode == Mode::ZeroFinished {
                    entry["mediaSessionId"] = json!(0);
                } else if mode == Mode::BroadcastError {
                    entry["idleReason"] = json!("ERROR");
                } else if mode == Mode::BroadcastCancelled {
                    entry["idleReason"] = json!("CANCELLED");
                }
                let event = Envelope {
                    payload_utf8: Some(
                        json!({"type":"MEDIA_STATUS","requestId":0,"status":[entry]}).to_string(),
                    ),
                    ..response.clone()
                };
                if send(&mut stream, &event).await.is_err() {
                    break;
                }
                if mode == Mode::BroadcastThenDisconnect {
                    break;
                }
            }
            if (pending_poll && mode == Mode::CloseAppDuringPoll)
                || (payload["type"] == "EDIT_TRACKS_INFO"
                    && matches!(mode, Mode::ForeignClose | Mode::CloseAppStillRunning))
            {
                let close = Envelope {
                    source_id: if mode == Mode::ForeignClose {
                        "other-app"
                    } else {
                        "app-transport"
                    }
                    .into(),
                    namespace: "urn:x-cast:com.google.cast.tp.connection".into(),
                    payload_utf8: Some(json!({"type":"CLOSE"}).to_string()),
                    ..response.clone()
                };
                if send(&mut stream, &close).await.is_err() {
                    break;
                }
            }
            if mode == Mode::Disconnect && log.loads > 0 {
                break;
            }
        }
        log
    });
    (address, task)
}

#[tokio::test]
async fn receiver_stop_finishes_without_stopping_other_apps_or_hiding_errors() {
    for mode in [
        Mode::CloseAppDuringPoll,
        Mode::ForeignClose,
        Mode::CloseAppStillRunning,
        Mode::BroadcastCancelled,
        Mode::MediaErrorAppGone,
        Mode::MediaErrorAppRunning,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("movie.mp4");
        fs::write(&video, "original bytes").unwrap();
        let subtitles = directory.path().join("captions.vtt");
        fs::write(&subtitles, "WEBVTT\n\n00:00.000 --> 00:02.000\nHello\n").unwrap();
        let (address, receiver) = receiver(mode, Arc::new(AtomicBool::new(false))).await;
        let mut request = CastRequest::new(video);
        request.target = Target::Host(address);
        request.subtitles = Some(subtitles);
        request.probe.executable = fake_probe(directory.path(), include_str!("fixtures/h264.json"));
        let (progress, updates) = watch::channel(SessionState::default());
        let result = tokio::time::timeout(
            Duration::from_secs(4),
            session::run(request, progress, &CancellationToken::new()),
        )
        .await
        .expect("app stop waited for a media request timeout");
        let log = tokio::time::timeout(Duration::from_secs(1), receiver)
            .await
            .unwrap()
            .unwrap();
        let should_fail = matches!(
            mode,
            Mode::CloseAppStillRunning | Mode::MediaErrorAppRunning
        );
        if should_fail {
            assert!(
                result.is_err(),
                "must not mask errors when the app remains active"
            );
            assert_eq!(updates.borrow().phase, Phase::Failed);
        } else {
            assert!(result.is_ok(), "{mode:?}: {result:?}");
            assert_eq!(
                updates.borrow().phase,
                if mode == Mode::ForeignClose {
                    Phase::Completed
                } else {
                    Phase::Stopped
                }
            );
            assert!(
                log.stops.is_empty(),
                "must not send a redundant STOP: {mode:?}"
            );
        }
        if matches!(mode, Mode::ForeignClose | Mode::BroadcastCancelled) {
            assert_eq!(log.receiver_polls, 0);
        } else {
            assert_eq!(log.receiver_polls, 1);
        }
        let host = log
            .url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        assert!(
            TcpStream::connect(host).await.is_err(),
            "HTTP server survived receiver stop"
        );
    }
}

#[tokio::test]
async fn terminal_broadcasts_complete_promptly_and_do_not_confuse_other_sessions() {
    for mode in [
        Mode::BroadcastFinished,
        Mode::BroadcastError,
        Mode::BroadcastDuringPoll,
        Mode::BroadcastThenDisconnect,
        Mode::ForeignFinished,
        Mode::ZeroFinished,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("movie.mp4");
        fs::write(&video, "selected video bytes").unwrap();
        let subs = directory.path().join("captions.vtt");
        fs::write(&subs, "WEBVTT\n\n00:00.000 --> 00:02.000\nHello\n").unwrap();
        let (address, receiver) = receiver(mode, Arc::new(AtomicBool::new(false))).await;
        let mut request = CastRequest::new(video);
        request.target = Target::Host(address);
        request.subtitles = Some(subs);
        request.probe.executable = fake_probe(directory.path(), include_str!("fixtures/h264.json"));
        let (progress, updates) = watch::channel(SessionState::default());
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            session::run(request, progress, &CancellationToken::new()),
        )
        .await
        .expect("terminal broadcast was ignored or waited for a request timeout");
        if mode == Mode::BroadcastError {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("playback ended: ERROR")
            );
            assert_eq!(updates.borrow().phase, Phase::Failed);
        } else {
            assert!(result.is_ok(), "{mode:?}: {result:?}");
            assert_eq!(updates.borrow().phase, Phase::Completed);
        }
        let log = tokio::time::timeout(Duration::from_secs(1), receiver)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(log.loads, 1);
        assert!(log.stops.is_empty(), "{mode:?}");
        if matches!(
            mode,
            Mode::ForeignFinished | Mode::ZeroFinished | Mode::BroadcastDuringPoll
        ) {
            assert_eq!(
                log.polls, 1,
                "must still poll after an unowned terminal event"
            );
        } else if mode != Mode::BroadcastError {
            assert_eq!(log.polls, 0, "should finish before the next poll");
        }
        let host = log
            .url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        assert!(
            TcpStream::connect(host).await.is_err(),
            "server survived completion"
        );
    }
}

fn fake_probe(directory: &Path, metadata: &str) -> PathBuf {
    let program = directory.join("fake ffprobe");
    fs::write(&program, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
    fs::write(directory.join("fake ffprobe.json"), metadata).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    program
}

#[tokio::test]
async fn experimental_surround_requires_opt_in_and_serves_original_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let video = directory.path().join("surround.mp4");
    let bytes = b"original surround video bytes";
    fs::write(&video, bytes).unwrap();
    let mut metadata: Value = serde_json::from_str(include_str!("fixtures/h264.json")).unwrap();
    metadata["streams"][1]["channels"] = json!(6);
    let probe = fake_probe(directory.path(), &metadata.to_string());
    let (address, receiver) = receiver(Mode::Complete, Arc::new(AtomicBool::new(false))).await;
    let mut request = CastRequest::new(video.clone());
    request.target = Target::Host(address);
    request.probe.executable = probe;
    request.ffmpeg = directory.path().join("missing-ffmpeg");
    let (progress, updates) = watch::channel(SessionState::default());
    let error = session::run(request.clone(), progress, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, ProcastError::UnsupportedMedia(_)));
    assert_eq!(updates.borrow().phase, Phase::Failed);
    // The receiver is still waiting for its first connection after preflight rejection.
    request.direct_play_policy = DirectPlayPolicy::Experimental;
    let (progress, updates) = watch::channel(SessionState::default());
    tokio::time::timeout(
        Duration::from_secs(5),
        session::run(request, progress, &CancellationToken::new()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(updates.borrow().phase, Phase::Completed);
    let log = tokio::time::timeout(Duration::from_secs(1), receiver)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(log.loads, 1);
    assert_eq!(log.video, bytes);
    assert_eq!(fs::read(&video).unwrap(), bytes);
    let host = log
        .url
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    assert!(
        TcpStream::connect(host).await.is_err(),
        "server survived completion"
    );
}

#[tokio::test]
async fn session_covers_completion_transient_idle_errors_takeover_and_cancellation() {
    for mode in [
        Mode::Complete,
        Mode::Transient,
        Mode::Reject,
        Mode::Disconnect,
        Mode::Hold,
        Mode::HoldLoad,
        Mode::Takeover,
        Mode::RejectTracks,
        Mode::MissingSubtitles,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("movie ünicode.mp4");
        fs::write(&file, b"selected video bytes").unwrap();
        let subs = directory.path().join("captions.vtt");
        fs::write(&subs, "WEBVTT\n\n00:00.000 --> 00:02.000\nHello Procast!\n").unwrap();
        let loaded = Arc::new(AtomicBool::new(false));
        let (address, receiver) = receiver(mode, loaded.clone()).await;
        let mut request = CastRequest::new(file);
        request.target = Target::Host(address);
        request.probe.executable = fake_probe(directory.path(), include_str!("fixtures/h264.json"));
        request.ffmpeg = directory.path().join("missing-ffmpeg");
        // Complete exercises the no-subtitle route; all others use WebVTT.
        if mode != Mode::Complete {
            request.subtitles = Some(subs);
        }
        let token = CancellationToken::new();
        let (progress, mut updates) = watch::channel(SessionState::default());
        let controller = async {
            if mode == Mode::HoldLoad {
                while !loaded.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                // Allow the simulated receiver to finish its HTTP fetch before cancellation.
                tokio::time::sleep(Duration::from_millis(30)).await;
                token.cancel();
            } else if mode == Mode::Hold {
                loop {
                    if updates.borrow_and_update().phase == Phase::Playing {
                        token.cancel();
                        break;
                    }
                    if updates.changed().await.is_err() {
                        break;
                    }
                }
            }
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(12), async {
            tokio::join!(session::run(request, progress, &token), controller)
        })
        .await
        .expect("session did not terminate");
        let final_phase = updates.borrow().phase;
        let log = tokio::time::timeout(Duration::from_secs(3), receiver)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(log.loads, 1, "{mode:?}: {log:?}");
        match mode {
            Mode::Complete | Mode::Transient => {
                assert!(result.is_ok(), "{mode:?}: {result:?}");
                assert_eq!(final_phase, Phase::Completed);
                assert!(
                    log.stops.is_empty(),
                    "natural completion must not send STOP"
                );
                if mode == Mode::Transient {
                    assert!(log.polls >= 4);
                }
            }
            Mode::Hold | Mode::HoldLoad => {
                assert!(matches!(result, Err(ProcastError::Cancelled)), "{result:?}");
                assert_eq!(final_phase, Phase::Cancelled);
                assert_eq!(log.stops, vec![7]);
            }
            _ => {
                assert!(result.is_err(), "{mode:?}");
                assert_eq!(final_phase, Phase::Failed);
                if mode == Mode::Takeover {
                    assert!(log.stops.is_empty());
                }
                if mode == Mode::Reject {
                    assert!(
                        result
                            .unwrap_err()
                            .to_string()
                            .contains("no successful video download")
                    );
                }
            }
        }
        let host = log
            .url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        assert!(
            TcpStream::connect(host).await.is_err(),
            "HTTP server survived {mode:?}"
        );
        if mode != Mode::Reject {
            assert_eq!(log.video, b"selected video bytes");
            if !matches!(mode, Mode::Complete | Mode::MissingSubtitles) {
                assert!(
                    String::from_utf8_lossy(log.subtitles.as_ref().unwrap())
                        .contains("Hello Procast!")
                );
            }
        }
    }
}

#[tokio::test]
async fn invalid_media_and_subtitles_fail_before_contacting_receiver() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("movie.mp4");
    fs::write(&file, "placeholder").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    for malformed_media in [true, false] {
        let mut request = CastRequest::new(file.clone());
        request.target = Target::Host(listener.local_addr().unwrap());
        let data = if malformed_media {
            include_str!("fixtures/h264.json").replace("h264", "hevc")
        } else {
            include_str!("fixtures/h264.json").into()
        };
        request.probe.executable = fake_probe(dir.path(), &data);
        let sub = dir.path().join("invalid.vtt");
        fs::write(&sub, "WEBVTT\n").unwrap();
        request.subtitles = Some(sub);
        let (progress, updates) = watch::channel(SessionState::default());
        let result = session::run(request, progress, &CancellationToken::new()).await;
        assert!(result.is_err());
        assert_eq!(updates.borrow().phase, Phase::Failed);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
#[ignore = "requires real ffmpeg/ffprobe with libx264 and AAC; uses only a loopback receiver"]
async fn real_media_and_srt_or_vtt_complete_the_entire_session() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("actual test.mp4");
    let output = tokio::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=size=160x90:rate=15",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-profile:v",
            "baseline",
            "-level:v",
            "3.0",
            "-c:a",
            "aac",
            "-movflags",
            "+faststart",
        ])
        .arg(&video)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (force_transcode, extension, content) in [
        (
            false,
            "srt",
            "1\n00:00:00,000 --> 00:00:01,500\nHello Procast!\n",
        ),
        (
            false,
            "vtt",
            "WEBVTT\n\n00:00.000 --> 00:01.500\nHello Procast!\n",
        ),
        (
            true,
            "srt",
            "1\n00:00:00,000 --> 00:00:01,500\nHello Procast!\n",
        ),
        (
            true,
            "vtt",
            "WEBVTT\n\n00:00.000 --> 00:01.500\nHello Procast!\n",
        ),
    ] {
        let subs = dir.path().join(format!("captions.{extension}"));
        fs::write(&subs, content).unwrap();
        let (address, receiver) = receiver(Mode::Complete, Arc::new(AtomicBool::new(false))).await;
        let mut request = CastRequest::new(video.clone());
        request.target = Target::Host(address);
        request.subtitles = Some(subs);
        request.force_transcode = force_transcode;
        request.transcode.directory = Some(dir.path().join("cache"));
        let (progress, _) = watch::channel(SessionState::default());
        tokio::time::timeout(
            Duration::from_secs(10),
            session::run(request, progress, &CancellationToken::new()),
        )
        .await
        .unwrap()
        .unwrap();
        let log = receiver.await.unwrap();
        if force_transcode {
            assert_ne!(
                log.video,
                fs::read(&video).unwrap(),
                "force must re-encode even compatible input"
            );
            assert!(
                fs::read_dir(dir.path().join("cache"))
                    .unwrap()
                    .next()
                    .is_none(),
                "prepared media survived completion"
            );
            let received = dir.path().join("received.mp4");
            fs::write(&received, &log.video).unwrap();
            let info = procast_core::media::inspect(
                &received,
                &Default::default(),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
            procast_core::media::validate_direct_play(&info).unwrap();
        } else {
            assert_eq!(log.video, fs::read(&video).unwrap());
        }
        let text = String::from_utf8(log.subtitles.unwrap()).unwrap();
        assert!(text.starts_with("WEBVTT"));
        assert!(text.contains("Hello Procast!"));
        assert_eq!(log.track_commands, 1);
    }
}
