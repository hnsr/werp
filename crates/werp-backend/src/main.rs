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
use werp_core::{
    CancellationToken, conversion, discovery, media, resume,
    session::{self, CastRequest, Control, Phase, SessionState, Target},
    subtitles,
};

mod gui_state;

const VERSION: u32 = 2;
const MAX_LINE: u64 = 64 * 1024;
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "ffprobe")]
    ffprobe: PathBuf,
    #[arg(long, default_value = "ffmpeg")]
    ffmpeg: PathBuf,
    #[arg(long)]
    no_inhibit_sleep: bool,
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
    GetSubtitleDelay {
        file: PathBuf,
        subtitles: Subtitle,
    },
    Discover,
    GetGuiPreferences {
        path: Option<PathBuf>,
    },
    SetGuiLastDevice {
        device_id: String,
    },
    PreviewConversion {
        file: PathBuf,
        device_id: Option<String>,
    },
    Convert {
        file: PathBuf,
        device_id: Option<String>,
    },
    CancelConversion {
        operation_id: u64,
    },
    Start {
        file: PathBuf,
        device_id: String,
        subtitles: Subtitle,
        #[serde(default)]
        subtitle_delay_ms: i32,
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
    SetSubtitles {
        session_id: u64,
        subtitles: Subtitle,
        subtitle_delay_ms: i32,
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
impl Subtitle {
    fn request(self) -> subtitles::Request {
        match self {
            Self::None => subtitles::Request::Off,
            Self::Embedded { index } => subtitles::Request::Embedded(index),
            Self::External { path } => subtitles::Request::External(path),
        }
    }
}
struct Active {
    id: u64,
    cancel: CancellationToken,
    controls: session::SessionController,
    updates: watch::Receiver<SessionState>,
    stops: Vec<u64>,
}
struct Converting {
    id: u64,
    cancel: CancellationToken,
    updates: watch::Receiver<conversion::State>,
    stops: Vec<u64>,
}
enum QueryResult {
    Devices(Vec<discovery::Device>),
    Data(Value),
}
// Conversion needs the model identity, not a connection to its address.
fn conversion_model<'a>(
    device_id: Option<&str>,
    devices: &'a HashMap<String, discovery::Device>,
) -> Result<Option<&'a str>, (&'static str, String)> {
    let Some(id) = device_id else {
        return Ok(None);
    };
    let device = devices.get(id).ok_or_else(|| {
        (
            "invalid_device",
            "select a discovered device or broad compatibility".into(),
        )
    })?;
    if device.capabilities.is_some_and(|bits| bits & 1 == 0) {
        return Err(("invalid_device", "device cannot play video".into()));
    }
    Ok(Some(&device.model))
}

fn conversion_policy(
    device_id: Option<&str>,
    devices: &HashMap<String, discovery::Device>,
) -> Result<media::DirectPlayPolicy, (&'static str, String)> {
    let Some(model) = conversion_model(device_id, devices)? else {
        return Ok(media::DirectPlayPolicy::Conservative);
    };
    let database =
        werp_core::devices::load(None).map_err(|e| ("operation_failed", e.to_string()))?;
    Ok(database.policy(Some(model)))
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
// Read-only recommendation using shared preferences; frontend choices stay explicit.
async fn suggested_subtitles(
    info: &media::MediaInfo,
    file: &std::path::Path,
) -> Result<Value, String> {
    let settings = werp_core::config::load(None).map_err(|error| error.to_string())?;
    for warning in &settings.warnings {
        eprintln!("Warning: {warning}");
    }
    let selection = subtitles::select(&subtitles::Request::Auto, info, file, &settings.subtitles)
        .await
        .map_err(|e| e.to_string())?;
    Ok(match selection {
        None => json!({"kind":"none"}),
        Some(subtitles::Selection::Text { index, .. } | subtitles::Selection::Bitmap { index }) => {
            json!({"kind":"embedded","index":index})
        }
        Some(subtitles::Selection::External(path)) => {
            let path = tokio::fs::canonicalize(path)
                .await
                .map_err(|e| e.to_string())?;
            json!({"kind":"external","path":path})
        }
    })
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
    let mut sessions = JoinSet::<(u64, Result<(), werp_core::WerpError>)>::new();
    let mut conversions = JoinSet::<u64>::new();
    let mut converting: Option<Converting> = None;
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
                    let failure = outcome.err().filter(|e| !matches!(e, werp_core::WerpError::Cancelled)).map(|e| e.to_string());
                    send(&output, json!({"event":"ended","session_id":session_id,"phase":phase,"error":failure})).await?;
                }
                completed = conversions.join_next(), if !conversions.is_empty() => {
                    let operation_id = completed.unwrap().map_err(|e| e.to_string())?;
                    let ended = converting.take().unwrap();
                    for id in ended.stops { send(&output,ok(id,json!({}))).await?; pending.remove(&id); }
                    send(&output,json!({"event":"conversion_ended","operation_id":operation_id,"state":*ended.updates.borrow()})).await?;
                }
                changed = async {
                    match converting.as_mut() {
                        Some(operation) => operation.updates.changed().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if changed.is_ok() && let Some(operation) = converting.as_mut() {
                        let state = operation.updates.borrow_and_update().clone();
                        let _ = output.try_send(json!({"event":"conversion_state","operation_id":operation.id,"state":state}));
                    }
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
                                send(&output,ok(id,json!({"version":VERSION,"application":"werp-backend"}))).await?;
                            }
                            _ => send(&output,error(Some(id),"protocol_version","send hello with version 2 first")).await?,
                        }
                        continue;
                    }
                    match request.command {
                        Command::Hello { .. } => send(&output,error(Some(id),"invalid_request","already connected")).await?,
                        Command::Shutdown => { shutdown_id = Some(id); break; }
                        Command::GetGuiPreferences { path } => {
                            let reply = match werp_core::config::load(path.as_deref()) {
                                Ok(settings) => {
                                    let mut warnings = settings.warnings;
                                    let state = gui_state::load().unwrap_or_else(|message| {
                                        warnings.push(message);
                                        Default::default()
                                    });
                                    ok(id,json!({"last_device_id":state.last_device_id,
                                        "conversion_auto_close":settings.gui.conversion.auto_close,
                                        "warnings":warnings}))
                                }
                                Err(message) => error(Some(id),"config_error",message),
                            };
                            send(&output,reply).await?;
                        }
                        Command::SetGuiLastDevice { device_id } => {
                            let state = gui_state::GuiState { last_device_id: device_id };
                            let reply = match gui_state::save(&state) {
                                Ok(()) => ok(id,json!({"last_device_id":state.last_device_id})),
                                Err(message) => error(Some(id),"state_error",message),
                            };
                            send(&output,reply).await?;
                        }
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
                                    let suggestion = tokio::select! {
                                        _ = cancel.cancelled() => return Err("cancelled".to_string()),
                                        suggestion = suggested_subtitles(&info,&file) => suggestion,
                                    };
                                    let (suggested_subtitles, subtitle_warning) = match suggestion {
                                        Ok(value) => (value,None),
                                        Err(error) => (json!({"kind":"none"}),Some(error)),
                                    };
                                    let (checkpoint, warning) = match resume::checkpoint(&info,None) {
                                        Ok(value) => (value,None),
                                        Err(error) => (None,Some(error.to_string())),
                                    };
                                    Ok(QueryResult::Data(json!({"media":info,"subtitles":choices,"resume_position":checkpoint,"resume_warning":warning,"suggested_subtitles":suggested_subtitles,"subtitle_warning":subtitle_warning})))
                                }.await;
                                (id,result)
                            });
                        }
                        Command::GetSubtitleDelay { file, subtitles } => {
                            if queries.len() >= 16 { send(&output,error(Some(id),"busy","too many queries")).await?; continue; }
                            pending.insert(id); let probe = probe.clone(); let cancel = lifetime.clone();
                            queries.spawn(async move {
                                let result = async {
                                    let info = media::inspect(&file, &probe, &cancel).await.map_err(|e| e.to_string())?;
                                    let subtitle = subtitles.request();
                                    let delay = tokio::select! {
                                        _ = cancel.cancelled() => return Err("cancelled".into()),
                                        delay = werp_core::subtitle_offsets::load(&info, &subtitle, None) => delay.map_err(|e| e.to_string())?,
                                    };
                                    Ok(QueryResult::Data(json!({"subtitle_delay_ms": delay})))
                                }.await;
                                (id, result)
                            });
                        }
                        Command::Discover => {
                            if queries.len() >= 16 { send(&output,error(Some(id),"busy","too many queries")).await?; continue; }
                            pending.insert(id); let cancel = lifetime.clone();
                            queries.spawn(async move { (id,discovery::discover(Duration::from_secs(5),&cancel).await.map(QueryResult::Devices).map_err(|e| e.to_string())) });
                        }
                        Command::PreviewConversion { file, device_id } => {
                            if queries.len() >= 16 { send(&output,error(Some(id),"busy","too many queries")).await?; continue; }
                            let policy = match conversion_policy(device_id.as_deref(), &devices) {
                                Ok(policy) => policy,
                                Err((code,message)) => { send(&output,error(Some(id),code,message)).await?; continue; }
                            };
                            pending.insert(id);
                            let probe = probe.clone(); let cancel = lifetime.clone();
                            queries.spawn(async move {
                                let result = conversion::preview(&file,&probe,policy,&cancel).await
                                    .map(|state| QueryResult::Data(json!(state))).map_err(|e| e.to_string());
                                (id,result)
                            });
                        }
                        Command::Convert { file, device_id } => {
                            if active.is_some() || converting.is_some() { send(&output,error(Some(id),"busy","an operation is already active")).await?; continue; }
                            let policy = match conversion_policy(device_id.as_deref(), &devices) {
                                Ok(policy) => policy,
                                Err((code,message)) => { send(&output,error(Some(id),code,message)).await?; continue; }
                            };
                            let operation_id = next_session; next_session += 1;
                            let cancel = lifetime.child_token();
                            let (progress, updates) = watch::channel(conversion::State::default());
                            let request = conversion::Request { file, probe:probe.clone(), ffmpeg:args.ffmpeg.clone(), inhibit_sleep:!args.no_inhibit_sleep, cache:Default::default(), policy };
                            converting = Some(Converting { id:operation_id, cancel:cancel.clone(), updates, stops:vec![] });
                            send(&output,ok(id,json!({"operation_id":operation_id}))).await?;
                            conversions.spawn(async move { conversion::run(request,progress,&cancel).await; operation_id });
                        }
                        Command::CancelConversion { operation_id } => {
                            let Some(operation) = converting.as_mut().filter(|o| o.id == operation_id) else {
                                send(&output,error(Some(id),"invalid_operation","conversion is no longer active")).await?; continue;
                            };
                            if operation.stops.len() >= 16 { send(&output,error(Some(id),"busy","cancellation is already pending")).await?; continue; }
                            pending.insert(id); operation.stops.push(id); operation.cancel.cancel();
                        }
                        Command::Start { file, device_id, subtitles, subtitle_delay_ms, position } => {
                            if active.is_some() || converting.is_some() { send(&output,error(Some(id),"busy","a session is already active")).await?; continue; }
                            let settings = match werp_core::config::load(None) {
                                Ok(settings) => settings,
                                Err(error) => { send(&output,self::error(Some(id),"config_error",error)).await?; continue; }
                            };
                            for warning in &settings.warnings { eprintln!("Warning: {warning}"); }
                            let Some(device) = devices.get(&device_id).cloned() else {
                                send(&output,error(Some(id),"invalid_device","select a discovered device")).await?; continue;
                            };
                            if device.addresses.is_empty() || device.capabilities.is_some_and(|c| c & 1 == 0) {
                                send(&output,error(Some(id),"invalid_device","device cannot play video or has no IPv4 address")).await?; continue;
                            }
                            if !position.is_finite() || position < 0.0 {
                                send(&output,error(Some(id),"invalid_position","position must be nonnegative and finite")).await?; continue;
                            }
                            let device_database = match werp_core::devices::load(None) {
                                Ok(settings) => settings,
                                Err(error) => { send(&output,self::error(Some(id),"operation_failed",error)).await?; continue; }
                            };
                            let mut request = CastRequest::new(file);
                            request.device_database = device_database;
                            request.target = Some(Target::Device(device));
                            request.start_position = position;
                            request.subtitle_delay_ms = subtitle_delay_ms;
                            request.probe = probe.clone(); request.ffmpeg = args.ffmpeg.clone();
                            request.inhibit_sleep = !args.no_inhibit_sleep;
                            request.http_port = settings.http_port;
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
                                Command::SetSubtitles { session_id, subtitles, subtitle_delay_ms } => (session_id,Some(Control::SetSubtitles { subtitles: subtitles.request(), delay_ms: subtitle_delay_ms })),
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
    while conversions.join_next().await.is_some() {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_uses_discovered_model_and_does_not_require_a_reachable_address() {
        let mut devices = HashMap::new();
        let mut device = discovery::Device {
            id: "test-id".into(),
            name: "Editable name".into(),
            model: "DIW7022".into(),
            addresses: vec![],
            port: 8009,
            capabilities: Some(1),
        };
        devices.insert(device.id.clone(), device.clone());
        assert_eq!(conversion_model(None, &devices).unwrap(), None);
        assert_eq!(
            conversion_model(Some("test-id"), &devices).unwrap(),
            Some("DIW7022")
        );
        for id in ["", "Editable name", "missing"] {
            assert_eq!(
                conversion_model(Some(id), &devices).unwrap_err().0,
                "invalid_device"
            );
        }
        device.capabilities = Some(4);
        devices.insert(device.id.clone(), device.clone());
        assert_eq!(
            conversion_model(Some("test-id"), &devices).unwrap_err().0,
            "invalid_device"
        );
        device.capabilities = None;
        devices.insert(device.id.clone(), device);
        assert!(conversion_model(Some("test-id"), &devices).is_ok());
    }

    #[test]
    fn start_accepts_signed_subtitle_delay_and_defaults_to_zero() {
        for delay in [None, Some(-1500), Some(750)] {
            let mut params = json!({"file":"video.mkv","device_id":"test","subtitles":{"kind":"none"},"position":0});
            if let Some(delay) = delay {
                params["subtitle_delay_ms"] = json!(delay);
            }
            let request: Request =
                serde_json::from_value(json!({"id":1,"method":"start","params":params})).unwrap();
            let Command::Start {
                subtitle_delay_ms, ..
            } = request.command
            else {
                panic!("expected start")
            };
            assert_eq!(subtitle_delay_ms, delay.unwrap_or(0));
        }
    }
}
