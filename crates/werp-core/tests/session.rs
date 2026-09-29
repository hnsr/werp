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

use prost::Message;
use rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};
use tokio_rustls::TlsAcceptor;
use werp_core::{
    CancellationToken, WerpError,
    playback::{Mode as PlaybackMode, Profile},
    session::{self, CastRequest, Phase, SessionState, Target},
};

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
    BufferingAppGone,
    EmptyAppGone,
    BufferingAppRunning,
}

#[derive(Default, Debug)]
struct Transcript {
    loads: usize,
    stops: Vec<i64>,
    video: Vec<u8>,
    content_type: String,
    subtitles: Option<Vec<u8>>,
    url: String,
    polls: usize,
    track_commands: usize,
    receiver_polls: usize,
    start_position: f64,
    controls: Vec<String>,
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

async fn download(url: &str, content_type: &str) -> Vec<u8> {
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
    assert!(
        header.contains(&format!("content-type: {content_type}\r\n")),
        "{header}"
    );
    bytes[header_end..].to_vec()
}

async fn receiver(
    mode: Mode,
    loaded: Arc<AtomicBool>,
) -> (SocketAddr, tokio::task::JoinHandle<Transcript>) {
    receiver_at(mode, loaded, 1.0).await
}

async fn receiver_at(
    mode: Mode,
    loaded: Arc<AtomicBool>,
    position: f64,
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
        let mut position = position;
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
                    log.start_position = payload["currentTime"].as_f64().unwrap();
                    log.url = payload["media"]["contentId"].as_str().unwrap().into();
                    loaded.store(true, Ordering::Release);
                    if mode == Mode::Reject {
                        json!({"type":"LOAD_FAILED","detailedErrorCode":104})
                    } else {
                        log.content_type = payload["media"]["contentType"].as_str().unwrap().into();
                        log.video = download(&log.url, &log.content_type).await;
                        if let Some(url) = payload
                            .pointer("/media/tracks/0/trackContentId")
                            .and_then(Value::as_str)
                        {
                            assert_eq!(payload["activeTrackIds"], json!([1]));
                            if mode != Mode::MissingSubtitles {
                                log.subtitles = Some(download(url, "text/vtt").await);
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
                        let applications = if !matches!(
                            mode,
                            Mode::CloseAppDuringPoll
                                | Mode::MediaErrorAppGone
                                | Mode::BufferingAppGone
                                | Mode::EmptyAppGone
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
                        } else if mode == Mode::BufferingAppGone
                            || (mode == Mode::BufferingAppRunning && log.polls == 1)
                        {
                            json!({"type":"MEDIA_STATUS","status":[status(&log.url,"BUFFERING",tracks)]})
                        } else if mode == Mode::EmptyAppGone {
                            json!({"type":"MEDIA_STATUS","status":[]})
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
                                    | Mode::BufferingAppRunning
                            ) || (mode == Mode::Transient && log.polls >= 4)
                            {
                                "IDLE"
                            } else if mode == Mode::Transient && log.polls == 2 {
                                "BUFFERING"
                            } else if mode == Mode::Hold {
                                state
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
                "PAUSE" | "PLAY" | "SEEK" => {
                    assert_eq!(payload["mediaSessionId"], 7);
                    log.controls.push(payload["type"].as_str().unwrap().into());
                    match payload["type"].as_str().unwrap() {
                        "PAUSE" => state = "PAUSED",
                        "PLAY" => state = "PLAYING",
                        "SEEK" => position = payload["currentTime"].as_f64().unwrap(),
                        _ => unreachable!(),
                    }
                    json!({"type":"MEDIA_STATUS","status":[status(&log.url,state,tracks)]})
                }
                "STOP" => {
                    log.stops.push(payload["mediaSessionId"].as_i64().unwrap());
                    json!({"type":"MEDIA_STATUS","status":[status(&log.url,"IDLE",tracks)]})
                }
                other => panic!("unexpected request: {other}"),
            };
            if let Some(entries) = reply.get_mut("status").and_then(Value::as_array_mut) {
                for entry in entries {
                    entry["currentTime"] = json!(position);
                }
            }
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
        Mode::BufferingAppGone,
        Mode::EmptyAppGone,
        Mode::BufferingAppRunning,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("movie.mp4");
        fs::write(&video, "original bytes").unwrap();
        let subtitles = directory.path().join("captions.vtt");
        fs::write(&subtitles, "WEBVTT\n\n00:00.000 --> 00:02.000\nHello\n").unwrap();
        let (address, receiver) = receiver(mode, Arc::new(AtomicBool::new(false))).await;
        let mut request = CastRequest::new(video);
        request.inhibit_sleep = false;
        request.subtitles = werp_core::subtitles::Request::Off;
        request.save_position = false;
        request.target = Some(Target::Host(address));
        request.subtitles = werp_core::subtitles::Request::External(subtitles);
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
                if matches!(mode, Mode::ForeignClose | Mode::BufferingAppRunning) {
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
        request.inhibit_sleep = false;
        request.subtitles = werp_core::subtitles::Request::Off;
        request.save_position = false;
        request.target = Some(Target::Host(address));
        request.subtitles = werp_core::subtitles::Request::External(subs);
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
async fn force_direct_serves_original_formats_and_keeps_session_cleanup() {
    for (container, extension, mime, mode) in [
        (
            "mov,mp4,m4a,3gp,3g2,mj2",
            "mp4",
            "video/mp4",
            Mode::Complete,
        ),
        ("matroska,webm", "mkv", "video/x-matroska", Mode::Complete),
        ("matroska,webm", "webm", "video/webm", Mode::Complete),
        (
            "unrecognized",
            "bin",
            "application/octet-stream",
            Mode::Complete,
        ),
        ("mp4", "mp4", "video/mp4", Mode::Hold),
        ("mp4", "mp4", "video/mp4", Mode::Reject),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join(format!("original.{extension}"));
        let bytes = b"original media including HDR signalling";
        fs::write(&video, bytes).unwrap();
        let mut metadata: Value = serde_json::from_str(include_str!("fixtures/h264.json")).unwrap();
        metadata["format"]["format_name"] = container.into();
        metadata["streams"][0]["color_transfer"] = "smpte2084".into();
        metadata["streams"][0]["codec_name"] = "av1".into();
        metadata["streams"][0]["width"] = 3840.into();
        metadata["streams"][0]["height"] = 2160.into();
        metadata["streams"][0]["r_frame_rate"] = "120/1".into();
        metadata["streams"][1]["codec_name"] = "dts".into();
        let probe = fake_probe(dir.path(), &metadata.to_string());
        let (address, receiver) = receiver(mode, Arc::new(AtomicBool::new(false))).await;
        let mut request = CastRequest::new(video.clone());
        request.inhibit_sleep = false;
        request.save_position = false;
        request.target = Some(Target::Host(address));
        request.probe.executable = probe;
        request.ffmpeg = dir.path().join("missing-ffmpeg");
        // This path cannot be a cache directory. A forced trial must never touch it.
        request.cache.directory = Some(video.clone());
        let (progress, _) = watch::channel(SessionState::default());
        let error = session::run(request.clone(), progress, &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("tone mapping"));
        request.force_direct = true;
        let captions = dir.path().join("captions.vtt");
        fs::write(&captions, "WEBVTT\n\n00:00.000 --> 00:01.000\nHello\n").unwrap();
        request.subtitles = werp_core::subtitles::Request::External(captions);
        let token = CancellationToken::new();
        let (progress, mut updates) = watch::channel(SessionState::default());
        let cancel = async {
            if mode == Mode::Hold {
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
        let (result, ()) = tokio::time::timeout(Duration::from_secs(6), async {
            tokio::join!(session::run(request, progress, &token), cancel)
        })
        .await
        .unwrap();
        let log = tokio::time::timeout(Duration::from_secs(1), receiver)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(log.loads, 1, "no retry or conversion fallback");
        match mode {
            Mode::Complete => assert!(result.is_ok(), "{result:?}"),
            Mode::Hold => {
                assert!(matches!(result, Err(WerpError::Cancelled)));
                assert_eq!(log.stops, vec![7]);
            }
            Mode::Reject => assert!(result.is_err()),
            _ => unreachable!(),
        }
        if mode != Mode::Reject {
            assert_eq!(log.video, bytes);
            assert_eq!(log.content_type, mime);
            assert!(
                String::from_utf8(log.subtitles.unwrap())
                    .unwrap()
                    .contains("Hello")
            );
        }
        assert_eq!(fs::read(&video).unwrap(), bytes);
        assert!(
            !fs::read_dir(dir.path()).unwrap().any(|p| p
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".werp-"))
        );
        let host = log
            .url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        assert!(
            TcpStream::connect(host).await.is_err(),
            "server survived cleanup"
        );
    }
}

#[tokio::test]
async fn force_direct_rejects_burn_in_and_still_requires_a_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("movie.mp4");
    fs::write(&video, "original").unwrap();
    let mut metadata: Value = serde_json::from_str(include_str!("fixtures/h264.json")).unwrap();
    metadata["streams"].as_array_mut().unwrap().push(json!({
        "index": 2, "codec_type": "subtitle", "codec_name": "hdmv_pgs_subtitle"
    }));
    let mut request = CastRequest::new(video);
    request.force_direct = true;
    request.inhibit_sleep = false;
    request.save_position = false;
    request.target = Some(Target::Host("127.0.0.1:1".parse().unwrap()));
    request.probe.executable = fake_probe(dir.path(), &metadata.to_string());
    request.ffmpeg = dir.path().join("missing-ffmpeg");
    request.subtitles = werp_core::subtitles::Request::Embedded(2);
    let (progress, _) = watch::channel(SessionState::default());
    let error = session::run(request.clone(), progress, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, WerpError::Subtitles(_)));
    assert!(error.to_string().contains("--force-direct cannot burn in"));
    request.file = dir.path().into();
    let (progress, _) = watch::channel(SessionState::default());
    assert!(matches!(
        session::run(request, progress, &CancellationToken::new()).await,
        Err(WerpError::NotRegularFile(_))
    ));
}

#[tokio::test]
async fn experimental_formats_require_opt_in_and_serve_original_bytes() {
    for fixture in [
        include_str!("fixtures/h264.json"),
        include_str!("fixtures/hevc.json"),
        include_str!("fixtures/h264-ac3.json"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("surround.mp4");
        let bytes = b"original surround video bytes";
        fs::write(&video, bytes).unwrap();
        let mut metadata: Value = serde_json::from_str(fixture).unwrap();
        metadata["streams"][1]["channels"] = json!(6);
        metadata["streams"][0]["color_transfer"] = "smpte2084".into();
        let probe = fake_probe(directory.path(), &metadata.to_string());
        let (address, receiver) = receiver(Mode::Complete, Arc::new(AtomicBool::new(false))).await;
        let mut request = CastRequest::new(video.clone());
        request.inhibit_sleep = false;
        request.subtitles = werp_core::subtitles::Request::Off;
        request.save_position = false;
        request.mode = PlaybackMode::Direct;
        request.target = Some(Target::Host(address));
        request.probe.executable = probe;
        request.ffmpeg = directory.path().join("missing-ffmpeg");
        let (progress, updates) = watch::channel(SessionState::default());
        let error = session::run(request.clone(), progress, &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(error, WerpError::UnsupportedMedia(_)));
        assert_eq!(updates.borrow().phase, Phase::Failed);
        // The receiver is still waiting for its first connection after preflight rejection.
        request.profile = Profile::Experimental;
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
        fs::write(&subs, "WEBVTT\n\n00:00.000 --> 00:02.000\nHello Werp!\n").unwrap();
        let loaded = Arc::new(AtomicBool::new(false));
        let (address, receiver) = receiver(mode, loaded.clone()).await;
        let mut request = CastRequest::new(file);
        request.inhibit_sleep = false;
        request.subtitles = werp_core::subtitles::Request::Off;
        request.save_position = false;
        request.target = Some(Target::Host(address));
        request.probe.executable = fake_probe(directory.path(), include_str!("fixtures/h264.json"));
        request.ffmpeg = directory.path().join("missing-ffmpeg");
        request.subtitle_delay_ms = 750;
        // Complete exercises the no-subtitle route; all others use WebVTT.
        if mode != Mode::Complete {
            request.subtitles = werp_core::subtitles::Request::External(subs);
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
                assert!(matches!(result, Err(WerpError::Cancelled)), "{result:?}");
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
                        .contains("00:00:00.750 --> 00:00:02.750\nHello Werp!")
                );
            }
        }
    }
}

#[tokio::test]
async fn delay_that_removes_all_captions_still_plays_video() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("movie.mp4");
    fs::write(&file, b"selected video bytes").unwrap();
    let subs = directory.path().join("captions.vtt");
    let original = "WEBVTT\n\n00:00.000 --> 00:02.000\nHello Werp!\n";
    fs::write(&subs, original).unwrap();
    let (address, receiver) = receiver(Mode::Complete, Arc::new(AtomicBool::new(false))).await;
    let mut request = CastRequest::new(file);
    request.inhibit_sleep = false;
    request.save_position = false;
    request.subtitles = werp_core::subtitles::Request::External(subs.clone());
    request.subtitle_delay_ms = -2000;
    request.target = Some(Target::Host(address));
    request.probe.executable = fake_probe(directory.path(), include_str!("fixtures/h264.json"));
    let (progress, _) = watch::channel(SessionState::default());
    tokio::time::timeout(
        Duration::from_secs(5),
        session::run(request, progress, &CancellationToken::new()),
    )
    .await
    .unwrap()
    .unwrap();
    let log = receiver.await.unwrap();
    assert_eq!(log.loads, 1);
    assert!(log.subtitles.is_none());
    assert_eq!(fs::read_to_string(subs).unwrap(), original);
}

#[tokio::test]
async fn invalid_media_and_subtitles_fail_before_contacting_receiver() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("movie.mp4");
    fs::write(&file, "placeholder").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    for malformed_media in [true, false] {
        let mut request = CastRequest::new(file.clone());
        request.inhibit_sleep = false;
        request.subtitles = werp_core::subtitles::Request::Off;
        request.save_position = false;
        request.mode = PlaybackMode::Direct;
        request.target = Some(Target::Host(listener.local_addr().unwrap()));
        let data = if malformed_media {
            include_str!("fixtures/h264.json").replace("h264", "hevc")
        } else {
            include_str!("fixtures/h264.json").into()
        };
        request.probe.executable = fake_probe(dir.path(), &data);
        let sub = dir.path().join("invalid.vtt");
        fs::write(&sub, "WEBVTT\n").unwrap();
        request.subtitles = werp_core::subtitles::Request::External(sub);
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
    let mkv = dir.path().join("actual test.mkv");
    let mux = tokio::process::Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(&video)
        .args(["-c", "copy"])
        .arg(&mkv)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        mux.status.success(),
        "{}",
        String::from_utf8_lossy(&mux.stderr)
    );
    for (mode, extension, content) in [
        (
            None,
            "srt",
            "1\n00:00:00,000 --> 00:00:01,500\nHello Werp!\n",
        ),
        (
            None,
            "vtt",
            "WEBVTT\n\n00:00.000 --> 00:01.500\nHello Werp!\n",
        ),
        (
            Some(werp_core::transcode::TranscodeMode::AudioVideo),
            "srt",
            "1\n00:00:00,000 --> 00:00:01,500\nHello Werp!\n",
        ),
        (
            Some(werp_core::transcode::TranscodeMode::AudioVideo),
            "vtt",
            "WEBVTT\n\n00:00.000 --> 00:01.500\nHello Werp!\n",
        ),
        (
            Some(werp_core::transcode::TranscodeMode::AudioOnly),
            "srt",
            "1\n00:00:00,000 --> 00:00:01,500\nHello Werp!\n",
        ),
        (
            Some(werp_core::transcode::TranscodeMode::AudioOnly),
            "vtt",
            "WEBVTT\n\n00:00.000 --> 00:01.500\nHello Werp!\n",
        ),
        (
            Some(werp_core::transcode::TranscodeMode::Remux),
            "srt",
            "1\n00:00:00,000 --> 00:00:01,500\nHello Werp!\n",
        ),
        (
            Some(werp_core::transcode::TranscodeMode::Remux),
            "vtt",
            "WEBVTT\n\n00:00.000 --> 00:01.500\nHello Werp!\n",
        ),
    ] {
        let subs = dir.path().join(format!("captions.{extension}"));
        fs::write(&subs, content).unwrap();
        let (address, receiver) = receiver(Mode::Complete, Arc::new(AtomicBool::new(false))).await;
        let source = if mode == Some(werp_core::transcode::TranscodeMode::Remux) {
            &mkv
        } else {
            &video
        };
        let mut request = CastRequest::new(source.clone());
        request.inhibit_sleep = false;
        request.subtitles = werp_core::subtitles::Request::Off;
        request.save_position = false;
        request.target = Some(Target::Host(address));
        request.subtitles = werp_core::subtitles::Request::External(subs);
        let force_transcode = mode.is_some();
        request.mode = match mode {
            Some(werp_core::transcode::TranscodeMode::Remux) => PlaybackMode::Remux,
            Some(werp_core::transcode::TranscodeMode::AudioOnly) => PlaybackMode::Audio,
            Some(werp_core::transcode::TranscodeMode::AudioVideo) => PlaybackMode::Transcode,
            None => PlaybackMode::Direct,
        };
        request.cache.enabled = false;
        request.cache.directory = Some(dir.path().join("cache"));
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
                fs::read(source).unwrap(),
                "preparation must serve the output rather than the source"
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
            let info = werp_core::media::inspect(
                &received,
                &Default::default(),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
            werp_core::media::validate_direct_play(&info).unwrap();
        } else {
            assert_eq!(log.video, fs::read(source).unwrap());
        }
        let text = String::from_utf8(log.subtitles.unwrap()).unwrap();
        assert!(text.starts_with("WEBVTT"));
        assert!(
            text.contains("00:00.000 --> 00:01.500\nHello Werp!")
                || text.contains("00:00:00.000 --> 00:00:01.500\nHello Werp!"),
            "{text}"
        );
        assert_eq!(log.track_commands, 1);
    }
}

#[tokio::test]
async fn resume_survives_interruptions_obeys_overrides_and_clears_on_completion() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("movie.mp4");
    fs::write(&video, "original video").unwrap();
    let mut data: Value = serde_json::from_str(include_str!("fixtures/h264.json")).unwrap();
    data["format"]["duration"] = json!("120.0");
    let probe = fake_probe(dir.path(), &data.to_string());
    let state = dir.path().join("resume");
    for (mode, enabled, restart, expected_start, position) in [
        (Mode::Hold, true, false, 0.0, 40.0),
        (Mode::Reject, true, false, 35.0, 40.0),
        (Mode::Hold, true, true, 0.0, 60.0),
        (Mode::Complete, false, false, 0.0, 1.0),
        (Mode::Complete, true, false, 55.0, 60.0),
        (Mode::Disconnect, true, false, 0.0, 30.0),
        (Mode::Complete, true, false, 25.0, 30.0),
        (Mode::Complete, true, false, 0.0, 1.0),
    ] {
        let (address, receiver) =
            receiver_at(mode, Arc::new(AtomicBool::new(false)), position).await;
        let mut request = CastRequest::new(video.clone());
        request.target = Some(Target::Host(address));
        request.probe.executable = probe.clone();
        request.inhibit_sleep = false;
        request.subtitles = werp_core::subtitles::Request::Off;
        request.save_position = enabled;
        request.resume_directory = Some(state.clone());
        request.start_position = if enabled && !restart {
            let info =
                werp_core::media::inspect(&request.file, &request.probe, &CancellationToken::new())
                    .await
                    .unwrap();
            werp_core::resume::checkpoint(&info, Some(&state))
                .unwrap()
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let cancel = CancellationToken::new();
        let (progress, mut updates) = watch::channel(SessionState::default());
        let control = async {
            if mode == Mode::Hold {
                loop {
                    if updates.borrow_and_update().phase == Phase::Playing {
                        cancel.cancel();
                        break;
                    }
                    if updates.changed().await.is_err() {
                        break;
                    }
                }
            }
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(session::run(request, progress, &cancel), control)
        })
        .await
        .unwrap();
        if matches!(mode, Mode::Hold | Mode::Reject | Mode::Disconnect) {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        let log = receiver.await.unwrap();
        assert_eq!(
            log.start_position, expected_start,
            "{mode:?}, enabled={enabled}, restart={restart}"
        );
    }
    assert!(
        !fs::read_dir(state).unwrap().any(|p| p
            .unwrap()
            .path()
            .extension()
            .is_some_and(|s| s == "json"))
    );
}

#[tokio::test]
async fn controlled_session_pauses_plays_seeks_and_stops_with_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("video.mp4");
    fs::write(&file, b"video").unwrap();
    let (address, receiver) = receiver(Mode::Hold, Arc::new(AtomicBool::new(false))).await;
    let mut request = CastRequest::new(file);
    request.target = Some(Target::Host(address));
    request.inhibit_sleep = false;
    request.save_position = false;
    let mut data: Value = serde_json::from_str(include_str!("fixtures/h264.json")).unwrap();
    data["format"]["duration"] = json!("120.0");
    request.probe.executable = fake_probe(dir.path(), &data.to_string());
    let cancel = CancellationToken::new();
    let (progress, mut updates) = watch::channel(SessionState::default());
    let (controller, controls) = session::control_channel();
    let token = cancel.clone();
    let task =
        tokio::spawn(
            async move { session::run_controlled(request, progress, &token, controls).await },
        );
    tokio::time::timeout(Duration::from_secs(8), async {
        while updates.borrow().phase != Phase::Playing {
            updates.changed().await.unwrap();
        }
        controller.command(session::Control::Pause).await.unwrap();
        while updates.borrow().phase != Phase::Paused {
            updates.changed().await.unwrap();
        }
        for position in [f64::NAN, f64::INFINITY, -1.0, 999999.0] {
            assert!(
                controller
                    .command(session::Control::Seek(position))
                    .await
                    .is_err()
            );
        }
        controller
            .command(session::Control::Seek(30.0))
            .await
            .unwrap();
        while updates.borrow().position_seconds != Some(30.0) {
            updates.changed().await.unwrap();
        }
        controller.command(session::Control::Play).await.unwrap();
        while updates.borrow().phase != Phase::Playing {
            updates.changed().await.unwrap();
        }
        controller
            .command(session::Control::Seek(5.0))
            .await
            .unwrap();
        while updates.borrow().position_seconds != Some(5.0) {
            updates.changed().await.unwrap();
        }
        cancel.cancel();
        assert!(matches!(task.await.unwrap(), Err(WerpError::Cancelled)));
        assert_eq!(updates.borrow().phase, Phase::Cancelled);
    })
    .await
    .unwrap();
    assert!(controller.command(session::Control::Play).await.is_err());
    let log = receiver.await.unwrap();
    assert_eq!(log.controls, ["PAUSE", "SEEK", "PLAY", "SEEK"]);
    assert_eq!(log.stops, [7]);
    let host = log
        .url
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    assert!(TcpStream::connect(host).await.is_err());
}
