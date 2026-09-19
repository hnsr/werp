//! One foreground casting session, independent of terminal or GUI presentation.
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::atomic::Ordering,
    time::Duration,
};

use serde::Serialize;
use serde_json::json;
use tokio::{sync::watch, time::Instant};

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
    Auto,
    Device(String),
    Host(SocketAddr),
}

#[derive(Debug, Clone)]
pub struct CastRequest {
    pub file: PathBuf,
    pub subtitles: subtitles::Request,
    pub target: Target,
    pub bind_address: Option<IpAddr>,
    pub http_port: u16,
    pub scan_duration: Duration,
    pub probe: ProbeOptions,
    pub ffmpeg: PathBuf,
    pub mode: Mode,
    pub profile: Profile,
    pub cache: CacheOptions,
    pub transcode: TranscodeOptions,
    /// Best-effort Linux sleep inhibition during preparation and playback.
    pub inhibit_sleep: bool,
    pub preferences: crate::config::Config,
    pub restart: bool,
    /// Override state location for embedding/testing; CLI uses XDG_STATE_HOME.
    pub resume_directory: Option<PathBuf>,
}

impl CastRequest {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            subtitles: subtitles::Request::Auto,
            target: Target::Auto,
            bind_address: None,
            http_port: 0,
            scan_duration: Duration::from_secs(5),
            probe: ProbeOptions::default(),
            ffmpeg: "ffmpeg".into(),
            mode: Mode::Auto,
            profile: Profile::Auto,
            cache: CacheOptions::default(),
            transcode: TranscodeOptions::default(),
            inhibit_sleep: true,
            preferences: crate::config::Config::default(),
            restart: false,
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
    pub message: Option<String>,
    /// Durable preparation decisions, retained even when progress updates coalesce.
    pub notices: Vec<String>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            phase: Phase::Preparing,
            position_seconds: None,
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
        state.position_seconds = position_seconds;
        state.message = message;
    });
}

fn notice(progress: &watch::Sender<SessionState>, message: String) {
    progress.send_modify(|state| state.notices.push(message));
}

/// Cancel via the token and await completion. Progress uses a latest-state
/// channel, so slow or absent frontends cannot block the session or heartbeats.
pub async fn run(
    request: CastRequest,
    progress: watch::Sender<SessionState>,
    cancel: &CancellationToken,
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
        let mut start_position = 0.0;
        if request.preferences.playback.resume {
            match crate::resume::ResumeStore::open(&info, request.resume_directory.as_deref()) {
                Ok(store) => {
                    if !request.restart { start_position = store.start_position(); }
                    if start_position > 0.0 { notice(&progress, format!("Resuming from {start_position:.0} seconds")); }
                    resume = Some(store);
                }
                Err(error) => tracing::warn!(%error, "Resume unavailable for this session; starting from the beginning"),
            }
        }
        let (address, model) = match &request.target {
            Target::Host(address) => (*address, None),
            target => {
                report(&progress, Phase::Discovering, None, None);
                let devices = discovery::discover(request.scan_duration, cancel).await?;
                let selected = select_preferred_target(&devices, target, &request.preferences.devices.preferred)?;
                (SocketAddr::new(
                    (*selected.addresses.first().ok_or_else(|| {
                        YeetError::Discovery("selected device has no IPv4 address".into())
                    })?).into(), selected.port,
                ), Some(selected.model.clone()))
            }
        };
        let policy = request.profile.resolve(model.as_deref());
        let subtitle = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(YeetError::Cancelled),
            selected = subtitles::select(&request.subtitles, &info, &request.file, &request.preferences.subtitles) => selected?,
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
        if let Some(mode) = plan.preparation() {
            let mut options = request.transcode.clone();
            options.mode = mode;
            options.playback_policy = plan.policy;
            options.bitmap_subtitle = bitmap_index;
            let label = match mode {
                TranscodeMode::AudioVideo => "Transcoding",
                TranscodeMode::AudioOnly => "Audio conversion",
                TranscodeMode::Remux => "Remuxing",
            };
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
            current = cast
                .wait_for_status(Duration::from_secs(1), cancel)
                .await
                .map_err(|e| delivery_error(e, serving))?;
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

pub fn select_target(
    devices: &[discovery::Device],
    target: &Target,
) -> Result<discovery::Device, YeetError> {
    select_preferred_target(devices, target, &[])
}

pub fn select_preferred_target(
    devices: &[discovery::Device],
    target: &Target,
    preferred: &[String],
) -> Result<discovery::Device, YeetError> {
    let result = (|| {
        if matches!(target, Target::Auto) {
            let eligible: Vec<_> = devices
                .iter()
                .filter(|d| {
                    d.capabilities.is_some_and(|bits| bits & 1 != 0) && !d.addresses.is_empty()
                })
                .cloned()
                .collect();
            for preference in preferred {
                if eligible
                    .iter()
                    .any(|d| d.id == *preference || d.name == *preference)
                {
                    return discovery::select(&eligible, preference);
                }
            }
        }
        select_target_inner(devices, target)
    })();
    result.map_err(|error| match error {
        YeetError::Discovery(message) => YeetError::Discovery(format!(
            "{message}. Detected devices: {}",
            if devices.is_empty() {
                "none".into()
            } else {
                devices
                    .iter()
                    .map(|d| {
                        format!(
                            "{:?} ({}){}",
                            d.name,
                            d.id,
                            if d.capabilities.is_some_and(|c| c & 1 == 0) {
                                " [audio-only]"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )),
        other => other,
    })
}

fn select_target_inner(
    devices: &[discovery::Device],
    target: &Target,
) -> Result<discovery::Device, YeetError> {
    let selected = match target {
        Target::Device(selector) => discovery::select(devices, selector)?,
        Target::Auto => {
            let eligible: Vec<_> = devices
                .iter()
                .filter(|d| {
                    d.capabilities.is_some_and(|bits| bits & 1 != 0) && !d.addresses.is_empty()
                })
                .collect();
            match eligible.as_slice() {
                [device] => (*device).clone(),
                _ => {
                    return Err(YeetError::Discovery(format!(
                        "found {} confirmed video receivers; choose --device NAME/ID or --host IP",
                        eligible.len(),
                    )));
                }
            }
        }
        Target::Host(_) => {
            return Err(YeetError::Discovery(
                "explicit IP does not require discovery".into(),
            ));
        }
    };
    if selected.capabilities.is_some_and(|bits| bits & 1 == 0) {
        return Err(YeetError::Discovery("selected device is audio-only".into()));
    }
    if selected.addresses.is_empty() {
        return Err(YeetError::Discovery(
            "selected device has no IPv4 address".into(),
        ));
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_selection_requires_exactly_one_known_video_receiver() {
        let video = discovery::Device {
            id: "tv-id".into(),
            name: "TV".into(),
            model: "test".into(),
            addresses: vec!["127.0.0.1".parse().unwrap()],
            port: 8009,
            capabilities: Some(5),
        };
        let audio = discovery::Device {
            id: "speaker".into(),
            name: "Speaker".into(),
            capabilities: Some(4),
            ..video.clone()
        };
        let unknown = discovery::Device {
            id: "unknown".into(),
            name: "Unknown".into(),
            capabilities: None,
            ..video.clone()
        };
        let other = discovery::Device {
            id: "other-id".into(),
            name: "Other TV".into(),
            ..video.clone()
        };
        let devices = [video.clone(), other.clone(), audio.clone()];
        assert_eq!(
            select_preferred_target(
                &devices,
                &Target::Auto,
                &[
                    "Speaker".into(),
                    "missing".into(),
                    "Other TV".into(),
                    "TV".into()
                ]
            )
            .unwrap()
            .id,
            "other-id"
        );
        assert_eq!(
            select_preferred_target(&devices, &Target::Device("TV".into()), &["Other TV".into()])
                .unwrap()
                .id,
            "tv-id"
        );
        let error = select_preferred_target(&devices, &Target::Device("Missing".into()), &[])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Other TV")
                && error.contains("Speaker")
                && error.contains("[audio-only]")
        );
        let duplicate = discovery::Device {
            name: "TV".into(),
            ..other
        };
        assert!(
            select_preferred_target(&[video.clone(), duplicate], &Target::Auto, &["TV".into()])
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        assert!(select_target(&[], &Target::Auto).is_err());
        assert!(select_target(&[audio.clone(), unknown.clone()], &Target::Auto).is_err());
        assert_eq!(
            select_target(
                &[audio.clone(), unknown.clone(), video.clone()],
                &Target::Auto
            )
            .unwrap()
            .id,
            "tv-id"
        );
        assert!(
            select_target(
                &[
                    video.clone(),
                    discovery::Device {
                        id: "other-tv".into(),
                        ..video.clone()
                    }
                ],
                &Target::Auto
            )
            .is_err()
        );
        assert!(select_target(&[audio], &Target::Device("speaker".into())).is_err());
        assert!(select_target(&[unknown], &Target::Device("unknown".into())).is_ok());
        assert!(
            select_target(
                &[discovery::Device {
                    addresses: vec![],
                    ..video
                }],
                &Target::Device("tv-id".into())
            )
            .is_err()
        );
    }
}
