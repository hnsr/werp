//! Private frontend helper. stdout is exclusively protocol JSON; stderr is logging.
use clap::Parser;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, watch},
    task::JoinSet,
};
use yeet_core::{
    CancellationToken, discovery, media, resume,
    session::{self, CastRequest, Control, Phase, SessionState, Target},
    subtitles,
};

const VERSION: u32 = 1;
const MAX_LINE: u64 = 64 * 1024;
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "ffprobe")]
    ffprobe: PathBuf,
    #[arg(long, default_value = "ffmpeg")]
    ffmpeg: PathBuf,
    #[arg(long)]
    no_inhibit_sleep: bool,
    #[arg(long, default_value_t = 0)]
    http_port: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: u64,
    #[serde(flatten)]
    command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
enum Command {
    Hello {
        version: u32,
    },
    Inspect {
        file: PathBuf,
    },
    Discover,
    Start {
        file: PathBuf,
        device_id: String,
        subtitles: Subtitle,
        position: f64,
    },
    Pause {
        session_id: u64,
    },
    Play {
        session_id: u64,
    },
    Seek {
        session_id: u64,
        position: f64,
    },
    Stop {
        session_id: u64,
    },
    Shutdown,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Subtitle {
    None,
    Embedded { index: u32 },
    External { path: PathBuf },
}
struct Active {
    id: u64,
    cancel: CancellationToken,
    controls: session::SessionController,
    updates: watch::Receiver<SessionState>,
    stops: Vec<u64>,
}
enum QueryResult {
    Devices(Vec<discovery::Device>),
    Data(Value),
}
fn ok(id: u64, result: Value) -> Value {
    json!({"id":id,"ok":true,"result":result})
}
fn error(id: Option<u64>, code: &str, message: impl ToString) -> Value {
    json!({"id":id,"ok":false,"error":{"code":code,"message":message.to_string()}})
}
async fn send(output: &mpsc::Sender<Value>, message: Value) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(3), output.send(message))
        .await
        .map_err(|_| "frontend stopped reading protocol messages".to_string())?
        .map_err(|_| "frontend output closed".into())
}
async fn run(args: Args) -> Result<(), String> {
    let lifetime = CancellationToken::new();
    let (output, mut messages) = mpsc::channel::<Value>(32);
    let disconnected = lifetime.clone();
    let mut writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = messages.recv().await {
            let mut line = serde_json::to_vec(&message).unwrap();
            line.push(b'\n');
            if stdout.write_all(&line).await.is_err() || stdout.flush().await.is_err() {
                disconnected.cancel();
                break;
            }
        }
    });
    let probe = media::ProbeOptions {
        executable: args.ffprobe,
        ..Default::default()
    };
    let (input_tx, mut input_rx) = mpsc::channel(16);
    let reader = tokio::spawn(async move {
        let mut input = BufReader::new(tokio::io::stdin());
        loop {
            let mut line = Vec::new();
            let result = (&mut input)
                .take(MAX_LINE + 1)
                .read_until(b'\n', &mut line)
                .await;
            let terminal = result.as_ref().map_or(true, |count| {
                *count == 0 || *count as u64 > MAX_LINE || !line.ends_with(b"\n")
            });
            if input_tx
                .send(result.map(|_| line).map_err(|e| e.to_string()))
                .await
                .is_err()
                || terminal
            {
                break;
            }
        }
    });
    let mut queries = JoinSet::new();
    let mut sessions = JoinSet::<(u64, Result<(), yeet_core::YeetError>)>::new();
    let mut pending = std::collections::HashSet::new();
    let mut devices = HashMap::<String, discovery::Device>::new();
    let mut active: Option<Active> = None;
    let mut next_session = 1u64;
    let mut hello = false;
    let mut shutdown_id = None;
    let result = async {
        loop {
            tokio::select! {
                biased;
                _ = lifetime.cancelled() => break,
                completed = sessions.join_next(), if !sessions.is_empty() => {
                    let (session_id, outcome) = completed.map(|r| r.map_err(|e| e.to_string())).transpose()?.unwrap();
                    let ended = active.take().unwrap();
                    let phase = ended.updates.borrow().phase;
                    send(&output, json!({"event":"state","session_id":session_id,"state":*ended.updates.borrow()})).await?;
                    for id in ended.stops { send(&output, ok(id,json!({}))).await?; pending.remove(&id); }
                    let failure = outcome.err().filter(|e| !matches!(e, yeet_core::YeetError::Cancelled)).map(|e| e.to_string());
                    send(&output, json!({"event":"ended","session_id":session_id,"phase":phase,"error":failure})).await?;
                }
                completed = queries.join_next(), if !queries.is_empty() => {
                    let (id, result) = completed.map(|r| r.map_err(|e| e.to_string())).transpose()?.unwrap();
                    let reply = match result {
                        Ok(QueryResult::Devices(found)) => {
                            devices = found.iter().map(|d| (d.id.clone(),d.clone())).collect();
                            ok(id,json!({"devices":found}))
                        }
                        Ok(QueryResult::Data(data)) => ok(id,data),
                        Err(message) => error(Some(id),"operation_failed",message),
                    };
                    pending.remove(&id);
                    send(&output,reply).await?;
                }
                changed = async {
                    match active.as_mut() {
                        Some(session) => session.updates.changed().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if changed.is_ok() && let Some(session) = active.as_mut() {
                        let state = session.updates.borrow_and_update().clone();
                        // Coalesced snapshots never stall the media session.
                        let _ = output.try_send(json!({"event":"state","session_id":session.id,"state":state}));
                    }
                }
                read = input_rx.recv() => {
                    let Some(line) = read else { break; };
                    let line = line?;
                    let count = line.len();
                    if count == 0 { break; }
                    if count as u64 > MAX_LINE || !line.ends_with(b"\n") {
                        send(&output,error(None,"invalid_request","request exceeds 64 KiB or has no newline")).await?;
                        break;
                    }
                    let raw: Value = match serde_json::from_slice(&line) {
                        Ok(raw) => raw,
                        Err(e) => { send(&output,error(None,"invalid_request",e)).await?; continue; }
                    };
                    let id = raw.get("id").and_then(Value::as_u64);
                    let request: Request = match serde_json::from_value(raw) {
                        Ok(request) => request,
                        Err(e) => { send(&output,error(id,"invalid_request",e)).await?; continue; }
                    };
                    let id = request.id;
                    if id > 9_007_199_254_740_991 || pending.contains(&id) {
                        send(&output,error(Some(id),"invalid_request","request id is out of range or already pending")).await?; continue;
                    }
                    if !hello {
                        match request.command {
                            Command::Hello { version: VERSION } => {
                                hello = true;
                                send(&output,ok(id,json!({"version":VERSION,"application":"yeet-backend"}))).await?;
                            }
                            _ => send(&output,error(Some(id),"protocol_version","send hello with version 1 first")).await?,
                        }
                        continue;
                    }
                    match request.command {
                        Command::Hello { .. } => send(&output,error(Some(id),"invalid_request","already connected")).await?,
                        Command::Shutdown => { shutdown_id = Some(id); break; }
                        Command::Inspect { file } => {
                            if queries.len() >= 16 { send(&output,error(Some(id),"busy","too many queries")).await?; continue; }
                            pending.insert(id);
                            let probe = probe.clone(); let cancel = lifetime.clone();
                            queries.spawn(async move {
                                let result = async {
                                    let info = media::inspect(&file,&probe,&cancel).await.map_err(|e| e.to_string())?;
                                    let choices = tokio::select! {
                                        _ = cancel.cancelled() => return Err("cancelled".to_string()),
                                        choices = subtitles::choices(&info,&file) => choices.map_err(|e| e.to_string())?,
                                    };
                                    let (checkpoint, warning) = match resume::checkpoint(&info,None) {
                                        Ok(value) => (value,None),
                                        Err(error) => (None,Some(error.to_string())),
                                    };
                                    Ok(QueryResult::Data(json!({"media":info,"subtitles":choices,"resume_position":checkpoint,"resume_warning":warning})))
                                }.await;
                                (id,result)
                            });
                        }
                        Command::Discover => {
                            if queries.len() >= 16 { send(&output,error(Some(id),"busy","too many queries")).await?; continue; }
                            pending.insert(id); let cancel = lifetime.clone();
                            queries.spawn(async move { (id,discovery::discover(Duration::from_secs(5),&cancel).await.map(QueryResult::Devices).map_err(|e| e.to_string())) });
                        }
                        Command::Start { file, device_id, subtitles, position } => {
                            if active.is_some() { send(&output,error(Some(id),"busy","a session is already active")).await?; continue; }
                            let Some(device) = devices.get(&device_id).cloned() else {
                                send(&output,error(Some(id),"invalid_device","select a discovered device")).await?; continue;
                            };
                            if device.addresses.is_empty() || device.capabilities.is_some_and(|c| c & 1 == 0) {
                                send(&output,error(Some(id),"invalid_device","device cannot play video or has no IPv4 address")).await?; continue;
                            }
                            if !position.is_finite() || position < 0.0 {
                                send(&output,error(Some(id),"invalid_position","position must be nonnegative and finite")).await?; continue;
                            }
                            let mut request = CastRequest::new(file);
                            request.target = Some(Target::Device(device));
                            request.start_position = position;
                            request.probe = probe.clone(); request.ffmpeg = args.ffmpeg.clone();
                            request.inhibit_sleep = !args.no_inhibit_sleep;
                            request.http_port = args.http_port;
                            request.subtitles = match subtitles {
                                Subtitle::None => subtitles::Request::Off,
                                Subtitle::Embedded { index } => subtitles::Request::Embedded(index),
                                Subtitle::External { path } => subtitles::Request::External(path),
                            };
                            let session_id = next_session; next_session += 1;
                            let cancel = lifetime.child_token();
                            let (progress, updates) = watch::channel(SessionState::default());
                            let (controls, receiver) = session::control_channel();
                            active = Some(Active { id:session_id, cancel:cancel.clone(), controls, updates, stops:vec![] });
                            send(&output,ok(id,json!({"session_id":session_id}))).await?;
                            sessions.spawn(async move { (session_id,session::run_controlled(request,progress,&cancel,receiver).await) });
                        }
                        command => {
                            let (session_id, control) = match command {
                                Command::Pause { session_id } => (session_id,Some(Control::Pause)),
                                Command::Play { session_id } => (session_id,Some(Control::Play)),
                                Command::Seek { session_id, position } => (session_id,Some(Control::Seek(position))),
                                Command::Stop { session_id } => (session_id,None),
                                _ => unreachable!(),
                            };
                            let Some(session) = active.as_mut().filter(|s| s.id == session_id) else {
                                send(&output,error(Some(id),"invalid_session","session is no longer active")).await?; continue;
                            };
                            if let Some(control) = control {
                                if !matches!(session.updates.borrow().phase, Phase::Playing | Phase::Paused | Phase::Buffering) {
                                    send(&output,error(Some(id),"not_playing","playback controls are unavailable during preparation")).await?; continue;
                                }
                                if queries.len() >= 16 { send(&output,error(Some(id),"busy","too many commands")).await?; continue; }
                                let controls = session.controls.clone(); pending.insert(id);
                                queries.spawn(async move { (id,controls.command(control).await.map(|()| QueryResult::Data(json!({})))) });
                            } else {
                                if session.stops.len() >= 16 { send(&output,error(Some(id),"busy","stop is already pending")).await?; continue; }
                                pending.insert(id); session.stops.push(id); session.cancel.cancel();
                            }
                        }
                    }
                }
            }
        }
        Ok::<(),String>(())
    }.await;
    lifetime.cancel();
    reader.abort();
    while sessions.join_next().await.is_some() {}
    while queries.join_next().await.is_some() {}
    if let Some(id) = shutdown_id {
        let _ = send(&output, ok(id, json!({}))).await;
    }
    drop(output);
    if tokio::time::timeout(Duration::from_secs(3), &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
    result
}
fn main() {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .without_time()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(run(args));
    // stdin's platform blocking reader can still await input after an explicit
    // shutdown request. All media/query tasks have already been joined above.
    runtime.shutdown_timeout(Duration::from_millis(100));
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
