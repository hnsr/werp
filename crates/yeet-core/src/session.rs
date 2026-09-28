//! One foreground casting session, independent of terminal or GUI presentation.
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::atomic::Ordering,
    time::Duration,
};

use serde::Serialize;
use serde_json::json;
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};

use crate::{
    CancellationToken, YeetError,
    cache::{self, CacheOptions},
    cast::CastSession,
    discovery,
    media::{self, ProbeOptions},
    playback::{self, Mode, Profile},
    serve::{MediaServer, address_toward},
    subtitles,
    transcode::{self, TranscodeMode, TranscodeOptions},
};

#[derive(Debug, Clone)]
pub enum Target {
    Device(discovery::Device),
    Host(SocketAddr),
}

#[derive(Debug, Clone)]
pub struct CastRequest {
    pub file: PathBuf,
    pub subtitles: subtitles::Request,
    /// Positive delays captions; negative shows them earlier. Applied before playback.
    pub subtitle_delay_ms: i32,
    pub target: Option<Target>,
    pub bind_address: Option<IpAddr>,
    pub http_port: u16,
    pub probe: ProbeOptions,
    pub ffmpeg: PathBuf,
    pub mode: Mode,
    pub profile: Profile,
    pub device_database: crate::devices::Database,
    pub cache: CacheOptions,
    pub transcode: TranscodeOptions,
    /// Best-effort Linux sleep inhibition during preparation and playback.
    pub inhibit_sleep: bool,
    pub save_position: bool,
    pub start_position: f64,
    /// Override state location for embedding/testing; CLI uses XDG_STATE_HOME.
    pub resume_directory: Option<PathBuf>,
}

impl CastRequest {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            subtitles: subtitles::Request::Off,
            subtitle_delay_ms: 0,
            target: None,
            bind_address: None,
            http_port: 0,
            probe: ProbeOptions::default(),
            ffmpeg: "ffmpeg".into(),
            mode: Mode::Auto,
            profile: Profile::Auto,
            device_database: Default::default(),
            cache: CacheOptions::default(),
            transcode: TranscodeOptions::default(),
            inhibit_sleep: true,
            save_position: true,
            start_position: 0.0,
            resume_directory: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Preparing,
    Discovering,
    Connecting,
    Loading,
    Playing,
    Paused,
    Buffering,
    Stopping,
    Completed,
    Stopped,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionState {
    pub phase: Phase,
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub preparation_operation: Option<String>,
    pub preparation_fraction: Option<f64>,
    pub message: Option<String>,
    /// Durable preparation decisions, retained even when progress updates coalesce.
    pub notices: Vec<String>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            phase: Phase::Preparing,
            position_seconds: None,
            duration_seconds: None,
            preparation_operation: None,
            preparation_fraction: None,
            message: None,
            notices: Vec::new(),
        }
    }
}

fn report(
    progress: &watch::Sender<SessionState>,
    phase: Phase,
    position_seconds: Option<f64>,
    message: Option<String>,
) {
    progress.send_modify(|state| {
        state.phase = phase;
        if phase != Phase::Preparing {
            state.preparation_operation = None;
            state.preparation_fraction = None;
        }
        state.position_seconds = position_seconds;
        state.message = message;
    });
}

fn notice(progress: &watch::Sender<SessionState>, message: String) {
    progress.send_modify(|state| state.notices.push(message));
}

#[derive(Debug, Clone, Copy)]
pub enum Control {
    Pause,
    Play,
    Seek(f64),
}

pub struct ControlRequest {
    command: Control,
    reply: oneshot::Sender<Result<(), String>>,
}
#[derive(Clone)]
pub struct SessionController {
    sender: mpsc::Sender<ControlRequest>,
}
pub fn control_channel() -> (SessionController, mpsc::Receiver<ControlRequest>) {
    let (sender, receiver) = mpsc::channel(16);
    (SessionController { sender }, receiver)
}
impl SessionController {
    pub async fn command(&self, command: Control) -> Result<(), String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(ControlRequest { command, reply })
            .await
            .map_err(|_| "session has ended".to_string())?;
        response
            .await
            .map_err(|_| "session has ended".to_string())?
    }
}

pub async fn run(
    request: CastRequest,
    progress: watch::Sender<SessionState>,
    cancel: &CancellationToken,
) -> Result<(), YeetError> {
    let (_controller, controls) = control_channel();
    run_controlled(request, progress, cancel, controls).await
}

/// Cancel via the token and await completion. Progress uses a latest-state
/// channel, so slow or absent frontends cannot block the session or heartbeats.
pub async fn run_controlled(
    request: CastRequest,
    progress: watch::Sender<SessionState>,
    cancel: &CancellationToken,
    mut controls: mpsc::Receiver<ControlRequest>,
) -> Result<(), YeetError> {
    let mut prepared = None;
    let mut prepared_media = None;
    let mut server = None;
    let mut transport = None;
    let mut stopped_on_receiver = false;
    let mut resume = None;
    let mut completed_naturally = false;
    #[cfg(target_os = "linux")]
    let mut sleep_inhibitor = None;
    let mut result = async {
        #[cfg(target_os = "linux")]
        if request.inhibit_sleep {
            match crate::power::SleepInhibitor::acquire(cancel).await {
                Ok(inhibitor) => {
                    sleep_inhibitor = Some(inhibitor);
                    notice(&progress, "Sleep inhibition active for this casting session".into());
                }
                Err(YeetError::Cancelled) => return Err(YeetError::Cancelled),
                Err(error) => tracing::warn!(%error, "Could not inhibit sleep; automatic sleep may interrupt casting"),
            }
        }
        #[cfg(not(target_os = "linux"))]
        if request.inhibit_sleep {
            tracing::warn!("Sleep inhibition is currently implemented only on Linux; keep the host awake while casting");
        }
        report(
            &progress,
            Phase::Preparing,
            None,
            Some("Inspecting media".into()),
        );
        let info = media::inspect(&request.file, &request.probe, cancel).await?;
        transcode::validate_input(&info)?;
        progress.send_modify(|state| state.duration_seconds = info.duration_seconds);
        let start_position = request.start_position;
        if !start_position.is_finite() || start_position < 0.0 || info.duration_seconds.is_some_and(|d| start_position >= d) {
            return Err(YeetError::Cast("starting position is outside the video duration".into()));
        }
        if start_position > 0.0 { notice(&progress, format!("Resuming from {start_position:.0} seconds")); }
        if request.save_position {
            match crate::resume::ResumeStore::open(&info, request.resume_directory.as_deref()) {
                Ok(store) => resume = Some(store),
                Err(error) => tracing::warn!(%error, "Could not save position for this session"),
            }
        }
        let (address, model) = match request.target.as_ref() {
            Some(Target::Host(address)) => (*address, None),
            Some(Target::Device(device)) => {
                if device.capabilities.is_some_and(|c| c & 1 == 0) { return Err(YeetError::Discovery("selected device is audio-only".into())); }
                let address = device.addresses.first().ok_or_else(|| YeetError::Discovery("selected device has no IPv4 address".into()))?;
                (SocketAddr::new((*address).into(), device.port), Some(device.model.clone()))
            }
            None => return Err(YeetError::Discovery("an explicit device is required".into())),
        };
        let policy = request.profile.resolve_with(model.as_deref(), &request.device_database);
        if matches!(request.subtitles, subtitles::Request::Auto) { return Err(YeetError::Subtitles("an explicit subtitle choice is required".into())); }
        let subtitle_preferences = crate::config::SubtitlePreferences { auto_load: false, languages: vec![] };
        let subtitle = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(YeetError::Cancelled),
            selected = subtitles::select(&request.subtitles, &info, &request.file, &subtitle_preferences) => selected?,
        };
        let bitmap_index = match &subtitle { Some(subtitles::Selection::Bitmap { index }) => Some(*index), _ => None };
        let mode = if bitmap_index.is_some() {
            if !matches!(request.mode, Mode::Auto | Mode::Transcode) {
                return Err(YeetError::Subtitles("image subtitles require video burn-in; use --mode auto or transcode, choose a text track, or --no-subtitles".into()));
            }
            notice(&progress, "Image subtitles selected: burning into video requires full transcoding".into());
            Mode::Transcode
        } else { request.mode };
        let plan = playback::select(&info, mode, policy)?;
        notice(&progress, format!("Selected {:?} ({policy:?} profile): {}", plan.mode, plan.reason));
        if request.profile == Profile::Experimental {
            tracing::warn!("Experimental direct play/profile: receiver audio/video support is not guaranteed; check audible output and picture");
        }
        match subtitle {
            Some(subtitles::Selection::External(path)) => {
                notice(&progress, format!("Subtitles: external file {}", path.display()));
                report(&progress, Phase::Preparing, None, Some("Preparing subtitles".into()));
                prepared = Some(subtitles::prepare(&path, &request.ffmpeg, cancel).await?);
            }
            Some(subtitles::Selection::Text { index, styled }) => {
                notice(&progress, format!("Subtitles: embedded stream #{index}"));
                if styled { tracing::warn!("ASS/SSA converted to WebVTT; advanced positioning, fonts and effects are not preserved"); }
                report(&progress, Phase::Preparing, None, Some("Extracting subtitles".into()));
                prepared = Some(subtitles::extract(&info, index, &request.ffmpeg, cancel).await?);
            }
            Some(subtitles::Selection::Bitmap { index }) => notice(&progress, format!("Subtitles: embedded image stream #{index} (burn-in)")),
            None => notice(&progress, "Subtitles: none selected".into()),
        }
        if let Some(captions) = &prepared
            && !captions.apply_delay(request.subtitle_delay_ms, cancel).await? {
            prepared.take().unwrap().close()?;
            notice(&progress, "Subtitle delay moves all captions before the start of the video".into());
        }
        if let Some(mode) = plan.preparation() {
            let mut options = request.transcode.clone();
            options.mode = mode;
            options.playback_policy = plan.policy;
            options.bitmap_subtitle = bitmap_index;
            options.subtitle_delay_ms = request.subtitle_delay_ms;
            let label = match mode {
                TranscodeMode::AudioVideo => "Transcoding",
                TranscodeMode::AudioOnly => "Audio conversion",
                TranscodeMode::Remux => "Remuxing",
            };
            progress.send_modify(|state| { state.preparation_operation = Some(label.into()); state.preparation_fraction = None; });
            let mut last_percent = None;
            prepared_media = Some(cache::prepare(
                &info, &request.ffmpeg, &request.probe, &options, &request.cache, cancel,
                |event| {
                    let message = match event {
                        cache::Event::Checking => Some("Checking prepared-file cache".into()),
                        cache::Event::Reused(path) => { notice(&progress, format!("Reusing prepared file: {}", path.display())); None },
                        cache::Event::Stored(path) => { notice(&progress, format!("Saved prepared file: {}", path.display())); None },
                        cache::Event::Fallback(path) => { notice(&progress, format!("Source folder is not writable; using cache: {}", path.display())); None },
                        cache::Event::Progress(update) => {
                            progress.send_modify(|state| state.preparation_fraction = Some(update.fraction.clamp(0.0, 1.0)));
                            let percent = (update.fraction * 100.0).floor() as u32;
                            if last_percent == Some(percent) { None } else {
                                last_percent = Some(percent);
                                Some(format!("{label}: {percent}%"))
                            }
                        }
                    };
                    if let Some(message) = message { report(&progress, Phase::Preparing, None, Some(message)); }
                },
            ).await?);
        }
        let media_path = prepared_media.as_ref().map_or(&info.path, |prepared| &prepared.info.path);
        let bind = match request.bind_address {
            Some(ip) => ip,
            None => address_toward(address).await?,
        };
        if !address.is_ipv4() {
            return Err(YeetError::Cast(
                "M2 currently supports IPv4 receivers only".into(),
            ));
        }
        if bind.is_unspecified()
            || !bind.is_ipv4()
            || (bind.is_loopback() && !address.ip().is_loopback())
        {
            return Err(YeetError::Serve("--bind-address must be a local address the receiver can reach, not a wildcard or loopback address".into()));
        }
        if cancel.is_cancelled() {
            return Err(YeetError::Cancelled);
        }
        server = Some(
            MediaServer::start(
                SocketAddr::new(bind, request.http_port),
                media_path,
                prepared.as_ref().map(|s| s.path.as_path()),
            )
            .await?,
        );
        let serving = server.as_ref().unwrap();
        report(
            &progress,
            Phase::Connecting,
            None,
            Some(format!("Connecting to {address}")),
        );
        transport = Some(CastSession::connect(address, cancel).await?);
        let cast = transport.as_mut().unwrap();
        report(
            &progress,
            Phase::Loading,
            None,
            Some("Starting receiver".into()),
        );
        cast.launch(cancel).await?;
        let title = request
            .file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let mut current = cast
            .load(
                &serving.video_url,
                serving.subtitle_url.as_deref(),
                &title,
                start_position,
                cancel,
            )
            .await
            .map_err(|e| delivery_error(e, serving))?;
        let mut last_active = Instant::now();
        let mut reached_playable = false;
        let loaded_at = Instant::now();
        let mut tracks_enabled = false;
        let mut subtitle_confirmed = serving.subtitle_url.is_none();
        let mut controls_open = true;
        loop {
            if cancel.is_cancelled() {
                return Err(YeetError::Cancelled);
            }
            if serving.is_finished() {
                return Err(YeetError::Serve(
                    "media server exited during playback".into(),
                ));
            }
            let mut phase = Phase::Loading;
            if let Some(status) = &current {
                if status.player_state != "IDLE" && serving.subtitle_url.is_some() && !tracks_enabled {
                    current = Some(
                        cast.command("EDIT_TRACKS_INFO", json!({"activeTrackIds":[1]}), cancel)
                            .await?,
                    );
                    tracks_enabled = true;
                }
                let status = current.as_ref().unwrap();
                if matches!(status.player_state.as_str(), "PLAYING" | "PAUSED")
                    && let Some(store) = &mut resume
                { store.update(status.current_time); }
                if status.active_track_ids.contains(&1) {
                    subtitle_confirmed = true;
                }
                match (status.player_state.as_str(), status.idle_reason.as_deref()) {
                    ("IDLE", Some("CANCELLED" | "RECEIVER_STOPPED")) => {
                        stopped_on_receiver = true;
                        return Ok(());
                    }
                    ("IDLE", Some("FINISHED")) => {
                        if !subtitle_confirmed {
                            return Err(YeetError::Subtitles(
                                "receiver never confirmed the requested subtitle track".into(),
                            ));
                        }
                        require_subtitle_fetch(serving)?;
                        completed_naturally = true;
                        return Ok(());
                    }
                    ("IDLE", Some(reason)) => {
                        return Err(delivery_error(
                            YeetError::Cast(format!("playback ended: {reason}")),
                            serving,
                        ));
                    }
                    ("IDLE", None) => {}
                    ("PLAYING", _) => {
                        reached_playable = true;
                        phase = Phase::Playing;
                        last_active = Instant::now();
                    }
                    ("PAUSED", _) => {
                        reached_playable = true;
                        phase = Phase::Paused;
                        last_active = Instant::now();
                    }
                    ("BUFFERING", _) => {
                        phase = Phase::Buffering;
                    }
                    (state, _) => {
                        return Err(YeetError::Cast(format!(
                            "unknown receiver state {state:?}"
                        )));
                    }
                }
                if matches!(phase, Phase::Playing | Phase::Paused) && !subtitle_confirmed {
                    return Err(YeetError::Subtitles(
                        "receiver did not activate the requested subtitle track".into(),
                    ));
                }
            }
            if last_active.elapsed() > Duration::from_secs(30) {
                return Err(delivery_error(
                    YeetError::Cast(if reached_playable {
                        format!("receiver stopped reporting playable media for 30 seconds (last state: {}, position: {:.3}s); retry with --verbose for receiver and HTTP diagnostics",
                            current.as_ref().map_or("unavailable", |s| s.player_state.as_str()),
                            current.as_ref().map_or(0.0, |s| s.current_time))
                    } else {
                        "receiver did not reach a playable state within 30 seconds".into()
                    }),
                    serving,
                ));
            }
            if subtitle_confirmed && loaded_at.elapsed() > Duration::from_secs(10) {
                require_subtitle_fetch(serving)?;
            }
            report(
                &progress,
                phase,
                current.as_ref().map(|s| s.current_time),
                None,
            );
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(YeetError::Cancelled),
                command = controls.recv(), if controls_open => {
                    if let Some(command) = command {
                        let (kind, extra) = match command.command {
                            Control::Pause => ("PAUSE", json!({})),
                            Control::Play => ("PLAY", json!({})),
                            Control::Seek(position) => {
                                if !position.is_finite() || position < 0.0 || info.duration_seconds.is_some_and(|d| position >= d) {
                                    let _ = command.reply.send(Err("seek position is outside the video duration".into()));
                                    continue;
                                }
                                ("SEEK", json!({"currentTime":position}))
                            }
                        };
                        match cast.command(kind, extra, cancel).await {
                            Ok(status) => { current = Some(status); let _ = command.reply.send(Ok(())); },
                            Err(error) => { let _ = command.reply.send(Err(error.to_string())); return Err(error); },
                        }
                    } else { controls_open = false; }
                }
                status = cast.wait_for_status(Duration::from_secs(1), cancel) => {
                    current = status.map_err(|e| delivery_error(e, serving))?;
                }
            }

        }
    }.await;

    report(&progress, Phase::Stopping, None, None);
    // Remote STOP and HTTP shutdown run together, with their own bounded cleanup.
    let (remote, local) = tokio::join!(
        async {
            match transport {
                Some(cast) => cast.finish(result.is_err()).await,
                None => Ok(()),
            }
        },
        async {
            match server {
                Some(server) => server.shutdown().await,
                None => Ok(()),
            }
        }
    );
    for cleanup in [
        remote,
        local,
        prepared.map_or(Ok(()), subtitles::PreparedSubtitles::close),
        prepared_media.map_or(Ok(()), transcode::PreparedMedia::close),
    ] {
        if let Err(error) = cleanup {
            if result.is_ok() {
                result = Err(error);
            } else {
                tracing::warn!(%error, "cleanup warning; local resources released where possible");
            }
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(inhibitor) = sleep_inhibitor {
        inhibitor.close().await;
    }
    if let Some(store) = resume {
        store.finish(completed_naturally);
    }
    let phase = match &result {
        Ok(()) if stopped_on_receiver => Phase::Stopped,
        Ok(()) => Phase::Completed,
        Err(YeetError::Cancelled) => Phase::Cancelled,
        Err(_) => Phase::Failed,
    };
    report(
        &progress,
        phase,
        None,
        result.as_ref().err().map(ToString::to_string),
    );
    result
}

fn require_subtitle_fetch(server: &MediaServer) -> Result<(), YeetError> {
    if server.subtitle_url.is_some() && server.subtitle_requests.load(Ordering::Relaxed) == 0 {
        return Err(YeetError::Subtitles(
            "receiver activated the track but no successful subtitle GET reached the host; check receiver text-track support and HTTP reachability".into(),
        ));
    }
    Ok(())
}

fn delivery_error(error: YeetError, server: &MediaServer) -> YeetError {
    if matches!(error, YeetError::Cancelled) {
        return error;
    }
    if server.video_requests.load(Ordering::Relaxed) == 0 {
        return YeetError::Cast(format!(
            "{error}; no successful video download reached {}. Check the selected interface, firewall, VPN and Wi-Fi client isolation; --bind-address and --http-port can help diagnose this",
            server.video_url
        ));
    }
    error
}
