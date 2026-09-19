//! Receiver-independent preparation using the same validated cache as casting.
use std::path::PathBuf;

use serde::Serialize;
use tokio::sync::watch;

use crate::{
    CancellationToken, YeetError, cache,
    media::{self, DirectPlayPolicy, MediaInfo, ProbeOptions},
    playback::{self, Mode},
    transcode::TranscodeOptions,
};

pub struct Request {
    pub file: PathBuf,
    pub probe: ProbeOptions,
    pub ffmpeg: PathBuf,
    pub inhibit_sleep: bool,
    pub cache: cache::CacheOptions,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Inspecting,
    Preparing,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Default, Serialize)]
pub struct State {
    pub phase: Phase,
    pub source: Option<MediaInfo>,
    pub target: Option<MediaInfo>,
    pub operation: Option<String>,
    pub fraction: Option<f64>,
    pub message: String,
    pub output: Option<PathBuf>,
    pub reused: bool,
    pub already_compatible: bool,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

/// Publishes a terminal snapshot only after local cleanup, including the inhibitor.
/// No discovery, receiver connection, HTTP server, subtitle or resume side effects.
pub async fn run(request: Request, updates: watch::Sender<State>, cancel: &CancellationToken) {
    let mut state = State {
        message: "Inspecting media".into(),
        ..Default::default()
    };
    updates.send_replace(state.clone());
    #[cfg(target_os = "linux")]
    let inhibitor = if request.inhibit_sleep {
        match crate::power::SleepInhibitor::acquire(cancel).await {
            Ok(inhibitor) => Some(inhibitor),
            Err(YeetError::Cancelled) => None,
            Err(error) => {
                state
                    .warnings
                    .push(format!("Could not inhibit sleep: {error}"));
                None
            }
        }
    } else {
        None
    };
    let outcome = prepare(&request, &mut state, &updates, cancel).await;
    #[cfg(target_os = "linux")]
    if let Some(inhibitor) = inhibitor {
        inhibitor.close().await;
    }
    match outcome {
        Ok(()) => {
            state.phase = Phase::Completed;
            state.fraction = Some(1.0);
        }
        Err(YeetError::Cancelled) => {
            state.phase = Phase::Cancelled;
            state.message = "Cancelled; cleanup completed".into();
        }
        Err(error) => {
            state.phase = Phase::Failed;
            state.message = "Conversion failed".into();
            state.error = Some(error.to_string());
        }
    }
    updates.send_replace(state);
}

async fn prepare(
    request: &Request,
    state: &mut State,
    updates: &watch::Sender<State>,
    cancel: &CancellationToken,
) -> Result<(), YeetError> {
    let info = media::inspect(&request.file, &request.probe, cancel).await?;
    state.source = Some(info.clone());
    updates.send_replace(state.clone());
    let plan = playback::select(&info, Mode::Auto, DirectPlayPolicy::Conservative)?;
    let Some(mode) = plan.preparation() else {
        state.already_compatible = true;
        state.output = Some(info.path.clone());
        state.target = Some(info);
        state.message = "Already compatible; no conversion needed".into();
        return Ok(());
    };
    state.phase = Phase::Preparing;
    state.operation = Some(
        match plan.mode {
            Mode::Remux => "Copying video and audio into MP4",
            Mode::Audio => "Copying video; converting audio to stereo AAC",
            _ => "Converting to H.264 and stereo AAC in MP4",
        }
        .into(),
    );
    // Successful output must survive this operation; conversion-only never uses
    // session-temporary storage even if a caller supplies a disabled cache.
    let cache = cache::CacheOptions {
        enabled: true,
        directory: request.cache.directory.clone(),
    };
    let options = TranscodeOptions {
        mode,
        playback_policy: plan.policy,
        ..Default::default()
    };
    let prepared = cache::prepare(
        &info,
        &request.ffmpeg,
        &request.probe,
        &options,
        &cache,
        cancel,
        |event| {
            match event {
                cache::Event::Checking => {
                    state.message = "Checking for an existing conversion".into();
                    state.fraction = None;
                }
                cache::Event::Reused(_) => {
                    state.reused = true;
                }
                cache::Event::Stored(_) => {}
                cache::Event::Fallback(path) => state.warnings.push(format!(
                    "Source directory is not writable; using {}",
                    path.display()
                )),
                cache::Event::Progress(progress) => {
                    state.message = if progress.fraction >= 1.0 {
                        "Validating and saving output".into()
                    } else {
                        state.operation.clone().unwrap()
                    };
                    state.fraction = Some(progress.fraction.min(0.99));
                }
            }
            updates.send_replace(state.clone());
        },
    )
    .await?;
    state.output = Some(prepared.info.path.clone());
    state.target = Some(prepared.info.clone());
    prepared.close()?;
    state.message = if state.reused {
        "Existing conversion reused"
    } else {
        "Conversion complete"
    }
    .into();
    Ok(())
}
