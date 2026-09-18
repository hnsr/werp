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
    CancellationToken, ProcastError,
    cast::CastSession,
    discovery,
    media::{self, DirectPlayAssessment, DirectPlayPolicy, ProbeOptions},
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
    pub subtitles: Option<PathBuf>,
    pub target: Target,
    pub bind_address: Option<IpAddr>,
    pub http_port: u16,
    pub scan_duration: Duration,
    pub probe: ProbeOptions,
    pub ffmpeg: PathBuf,
    pub direct_play_policy: DirectPlayPolicy,
    /// Prepare MP4 using `transcode.mode` even when the source is directly playable.
    pub force_transcode: bool,
    pub transcode: TranscodeOptions,
}

impl CastRequest {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            subtitles: None,
            target: Target::Auto,
            bind_address: None,
            http_port: 0,
            scan_duration: Duration::from_secs(5),
            probe: ProbeOptions::default(),
            ffmpeg: "ffmpeg".into(),
            direct_play_policy: DirectPlayPolicy::default(),
            force_transcode: false,
            transcode: TranscodeOptions::default(),
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
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            phase: Phase::Preparing,
            position_seconds: None,
            message: None,
        }
    }
}

fn report(
    progress: &watch::Sender<SessionState>,
    phase: Phase,
    position_seconds: Option<f64>,
    message: Option<String>,
) {
    progress.send_replace(SessionState {
        phase,
        position_seconds,
        message,
    });
}

/// Cancel via the token and await completion. Progress uses a latest-state
/// channel, so slow or absent frontends cannot block the session or heartbeats.
pub async fn run(
    request: CastRequest,
    progress: watch::Sender<SessionState>,
    cancel: &CancellationToken,
) -> Result<(), ProcastError> {
    let mut prepared = None;
    let mut prepared_media = None;
    let mut server = None;
    let mut transport = None;
    let mut stopped_on_receiver = false;
    let mut result = async {
        report(
            &progress,
            Phase::Preparing,
            None,
            Some("Inspecting media".into()),
        );
        let info = media::inspect(&request.file, &request.probe, cancel).await?;
        if request.force_transcode && request.direct_play_policy != DirectPlayPolicy::Conservative {
            return Err(ProcastError::Transcode("media preparation and experimental direct play are mutually exclusive".into()));
        }
        let assessment = if request.force_transcode { None } else {
            Some(media::assess_direct_play(&info, request.direct_play_policy)?)
        };
        if let Some(DirectPlayAssessment::ExperimentalAacSurround { channels }) = assessment {
            tracing::warn!(
                channels,
                "Experimental direct play: AAC-LC surround support is unverified on this receiver. Serving the original file without conversion; check audible audio and correct downmix or surround output."
            );
        }
        if let Some(DirectPlayAssessment::ExperimentalHevc { audio_channels }) = assessment {
            tracing::warn!(
                ?audio_channels,
                "Experimental direct play: HEVC support depends on the receiver. Serving the original MP4 without conversion; check picture, colours, audible audio and synchronization."
            );
        }
        if let Some(DirectPlayAssessment::ExperimentalAc3 { channels }) = assessment {
            tracing::warn!(
                channels,
                "Experimental direct play: AC-3 audio output depends on the receiver and connected equipment. Serving the original MP4 without conversion; check audible dialogue, channel balance and synchronization."
            );
        }
        if let Some(path) = &request.subtitles {
            report(
                &progress,
                Phase::Preparing,
                None,
                Some("Preparing subtitles".into()),
            );
            prepared = Some(subtitles::prepare(path, &request.ffmpeg, cancel).await?);
        }
        if request.force_transcode {
            let message = match request.transcode.mode {
                TranscodeMode::AudioVideo => "Preparing H.264/stereo AAC MP4 before playback",
                TranscodeMode::AudioOnly => "Copying video and converting audio to stereo AAC before playback",
                TranscodeMode::Remux => "Copying video and audio into MP4 before playback",
            };
            report(&progress, Phase::Preparing, None, Some(message.into()));
            let mut last_percent = None;
            prepared_media = Some(transcode::prepare(
                &info, &request.ffmpeg, &request.probe, &request.transcode, cancel,
                |update| {
                    let percent = (update.fraction * 100.0).floor() as u32;
                    if last_percent != Some(percent) {
                        report(&progress, Phase::Preparing, None, Some(format!("{}: {percent}%", match request.transcode.mode {
                            TranscodeMode::AudioOnly => "Audio conversion",
                            TranscodeMode::AudioVideo => "Transcoding",
                            TranscodeMode::Remux => "Remuxing",
                        })));
                        last_percent = Some(percent);
                    }
                },
            ).await?);
        }
        let media_path = prepared_media.as_ref().map_or(&info.path, |prepared| &prepared.info.path);
        let address = match &request.target {
            Target::Host(address) => *address,
            target => {
                report(&progress, Phase::Discovering, None, None);
                let devices = discovery::discover(request.scan_duration, cancel).await?;
                let selected = select_target(&devices, target)?;
                SocketAddr::new(
                    (*selected.addresses.first().ok_or_else(|| {
                        ProcastError::Discovery("selected device has no IPv4 address".into())
                    })?)
                    .into(),
                    selected.port,
                )
            }
        };
        let bind = match request.bind_address {
            Some(ip) => ip,
            None => address_toward(address).await?,
        };
        if !address.is_ipv4() {
            return Err(ProcastError::Cast(
                "M2 currently supports IPv4 receivers only".into(),
            ));
        }
        if bind.is_unspecified()
            || !bind.is_ipv4()
            || (bind.is_loopback() && !address.ip().is_loopback())
        {
            return Err(ProcastError::Serve("--bind-address must be a local address the receiver can reach, not a wildcard or loopback address".into()));
        }
        if cancel.is_cancelled() {
            return Err(ProcastError::Cancelled);
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
                return Err(ProcastError::Cancelled);
            }
            if serving.is_finished() {
                return Err(ProcastError::Serve(
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
                            return Err(ProcastError::Subtitles(
                                "receiver never confirmed the requested subtitle track".into(),
                            ));
                        }
                        require_subtitle_fetch(serving)?;
                        return Ok(());
                    }
                    ("IDLE", Some(reason)) => {
                        return Err(delivery_error(
                            ProcastError::Cast(format!("playback ended: {reason}")),
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
                        return Err(ProcastError::Cast(format!(
                            "unknown receiver state {state:?}"
                        )));
                    }
                }
                if matches!(phase, Phase::Playing | Phase::Paused) && !subtitle_confirmed {
                    return Err(ProcastError::Subtitles(
                        "receiver did not activate the requested subtitle track".into(),
                    ));
                }
            }
            if last_active.elapsed() > Duration::from_secs(30) {
                return Err(delivery_error(
                    ProcastError::Cast(if reached_playable {
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
    let phase = match &result {
        Ok(()) if stopped_on_receiver => Phase::Stopped,
        Ok(()) => Phase::Completed,
        Err(ProcastError::Cancelled) => Phase::Cancelled,
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

fn require_subtitle_fetch(server: &MediaServer) -> Result<(), ProcastError> {
    if server.subtitle_url.is_some() && server.subtitle_requests.load(Ordering::Relaxed) == 0 {
        return Err(ProcastError::Subtitles(
            "receiver activated the track but no successful subtitle GET reached the host; check receiver text-track support and HTTP reachability".into(),
        ));
    }
    Ok(())
}

fn delivery_error(error: ProcastError, server: &MediaServer) -> ProcastError {
    if matches!(error, ProcastError::Cancelled) {
        return error;
    }
    if server.video_requests.load(Ordering::Relaxed) == 0 {
        return ProcastError::Cast(format!(
            "{error}; no successful video download reached {}. Check the selected interface, firewall, VPN and Wi-Fi client isolation; --bind-address and --http-port can help diagnose this",
            server.video_url
        ));
    }
    error
}

pub fn select_target(
    devices: &[discovery::Device],
    target: &Target,
) -> Result<discovery::Device, ProcastError> {
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
                    return Err(ProcastError::Discovery(format!(
                        "found {} confirmed video receivers; choose --device NAME/ID or --host IP. Discovered: {}",
                        eligible.len(),
                        devices
                            .iter()
                            .map(|d| format!("{:?} ({})", d.name, d.id))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
        }
        Target::Host(_) => {
            return Err(ProcastError::Discovery(
                "explicit IP does not require discovery".into(),
            ));
        }
    };
    if selected.capabilities.is_some_and(|bits| bits & 1 == 0) {
        return Err(ProcastError::Discovery(
            "selected device is audio-only".into(),
        ));
    }
    if selected.addresses.is_empty() {
        return Err(ProcastError::Discovery(
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
