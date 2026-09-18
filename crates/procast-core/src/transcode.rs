//! Prepare a complete session-owned SDR MP4 before contacting a receiver.
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use tempfile::TempDir;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

use crate::{
    CancellationToken, ProcastError,
    media::{self, DirectPlayPolicy, MediaInfo, ProbeOptions},
    process,
};

const RESERVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PROGRESS_LINE: usize = 8192;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TranscodeMode {
    #[default]
    AudioVideo,
    /// Preserve conservative-profile H.264 MP4 video and encode only audio.
    AudioOnly,
    /// Copy conservative-profile H.264/AAC streams from Matroska or MP4 into MP4.
    Remux,
}

#[derive(Debug, Clone)]
pub struct TranscodeOptions {
    pub mode: TranscodeMode,
    pub playback_policy: DirectPlayPolicy,
    /// Parent for private session directories. Defaults to the user's cache.
    pub directory: Option<PathBuf>,
    /// Encoding deadline, separate from the short ffprobe timeout.
    pub timeout: Duration,
}

impl Default for TranscodeOptions {
    fn default() -> Self {
        Self {
            mode: TranscodeMode::default(),
            playback_policy: DirectPlayPolicy::Conservative,
            directory: None,
            timeout: Duration::from_secs(24 * 60 * 60),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TranscodeProgress {
    pub encoded_seconds: f64,
    pub fraction: f64,
}

pub struct PreparedMedia {
    pub info: MediaInfo,
    directory: Option<TempDir>,
}

impl PreparedMedia {
    pub(crate) fn retained(info: MediaInfo) -> Self {
        Self {
            info,
            directory: None,
        }
    }

    pub fn close(self) -> Result<(), ProcastError> {
        self.directory.map_or(Ok(()), |directory| {
            directory
                .close()
                .map_err(|e| error(format!("cannot remove prepared media: {e}")))
        })
    }
}

fn error(message: impl Into<String>) -> ProcastError {
    ProcastError::Transcode(message.into())
}

pub(crate) fn cache_directory() -> Result<PathBuf, ProcastError> {
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        && path.is_absolute()
    {
        return Ok(path.join("procast"));
    }
    if let Some(path) = std::env::var_os("HOME").map(PathBuf::from)
        && path.is_absolute()
    {
        return Ok(path.join(".cache/procast"));
    }
    Err(error(
        "cannot locate the user cache; specify --transcode-dir PATH",
    ))
}

#[cfg(unix)]
fn available_space(path: &Path) -> Result<u64, ProcastError> {
    let stat = rustix::fs::statvfs(path)
        .map_err(|e| error(format!("cannot check preparation disk space: {e}")))?;
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

#[cfg(not(unix))]
fn available_space(_path: &Path) -> Result<u64, ProcastError> {
    Err(error(
        "transcoding disk-space checks are currently implemented on Unix only",
    ))
}

fn require_space(available: u64, required: u64) -> Result<(), ProcastError> {
    if available < required {
        return Err(error(format!(
            "insufficient preparation disk space: need about {} MiB, have {} MiB; choose --transcode-dir on a larger disk",
            required.div_ceil(1024 * 1024),
            available / (1024 * 1024)
        )));
    }
    Ok(())
}

struct InputPlan {
    video: u32,
    audio: Option<u32>,
    duration: f64,
    fps: f64,
}

fn plan(info: &MediaInfo) -> Result<InputPlan, ProcastError> {
    let duration = info
        .duration_seconds
        .filter(|d| d.is_finite() && *d > 0.0)
        .ok_or_else(|| error("media preparation requires a known positive duration"))?;
    let videos: Vec<_> = info
        .streams
        .iter()
        .filter(|s| s.kind == "video" && !s.attached_picture)
        .collect();
    if videos.len() != 1 {
        return Err(error("media preparation requires exactly one video stream"));
    }
    let video = videos[0];
    if matches!(
        video.color_transfer.as_deref(),
        Some("smpte2084" | "arib-std-b67")
    ) || video.dolby_vision
    {
        return Err(error(
            "HDR/Dolby Vision tone mapping is not implemented; media preparation currently supports SDR input only",
        ));
    }
    let fps = video
        .frame_rate
        .filter(|r| r.is_finite() && *r > 0.0)
        .ok_or_else(|| error("cannot determine input frame rate"))?
        .min(30.0);
    let audio: Vec<_> = info.streams.iter().filter(|s| s.kind == "audio").collect();
    if audio.len() > 1 {
        return Err(error(
            "multiple audio tracks need explicit selection, which is not implemented yet",
        ));
    }
    Ok(InputPlan {
        video: video.index,
        audio: audio.first().map(|s| s.index),
        duration,
        fps,
    })
}

pub(crate) fn validate_input(info: &MediaInfo) -> Result<(), ProcastError> {
    plan(info).map(|_| ())
}

async fn check_encoders(
    ffmpeg: &Path,
    audio: bool,
    mode: TranscodeMode,
    cancel: &CancellationToken,
) -> Result<(), ProcastError> {
    if mode == TranscodeMode::Remux {
        return Ok(()); // Stream copy needs a demuxer/muxer, not encoders.
    }
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-encoders"]);
    let output = process::capture(&mut command, cancel, Duration::from_secs(10)).await?;
    let text = String::from_utf8_lossy(&output);
    for required in [
        (mode == TranscodeMode::AudioVideo).then_some("libx264"),
        audio.then_some("aac"),
    ]
    .into_iter()
    .flatten()
    {
        if !text
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(required))
        {
            return Err(error(format!(
                "FFmpeg lacks the {required} encoder; install an FFmpeg build with this encoder or select one using --ffmpeg"
            )));
        }
    }
    Ok(())
}

/// Streams FFmpeg's key=value progress without retaining the complete output.
async fn read_progress(
    mut reader: impl AsyncRead + Unpin,
    duration: f64,
    mut update: impl FnMut(TranscodeProgress) -> Result<(), ProcastError>,
) -> Result<(), ProcastError> {
    let mut buffer = [0; 4096];
    let mut line = Vec::new();
    let mut seconds: f64 = 0.0;
    loop {
        let length = reader
            .read(&mut buffer)
            .await
            .map_err(|e| error(format!("cannot read encoding progress: {e}")))?;
        if length == 0 {
            return Ok(());
        }
        for byte in &buffer[..length] {
            if *byte == b'\n' {
                let text = String::from_utf8_lossy(&line);
                if let Some(value) = text.trim().strip_prefix("out_time_us=")
                    && let Ok(us) = value.parse::<f64>()
                    && us.is_finite()
                {
                    seconds = seconds.max((us / 1_000_000.0).max(0.0));
                }
                if text.starts_with("progress=") {
                    // Faststart relocation and output validation may still be pending.
                    update(TranscodeProgress {
                        encoded_seconds: seconds,
                        fraction: (seconds / duration).clamp(0.0, 0.99),
                    })?;
                }
                line.clear();
            } else {
                if line.len() >= MAX_PROGRESS_LINE {
                    return Err(error("FFmpeg progress line exceeds 8 KiB"));
                }
                line.push(*byte);
            }
        }
    }
}

/// Cancel through the token and await completion, ensuring FFmpeg is reaped and
/// partial output removed. A successful result owns the output until close/drop.
pub async fn prepare(
    info: &MediaInfo,
    ffmpeg: &Path,
    probe: &ProbeOptions,
    options: &TranscodeOptions,
    cancel: &CancellationToken,
    mut update: impl FnMut(TranscodeProgress),
) -> Result<PreparedMedia, ProcastError> {
    if cancel.is_cancelled() {
        return Err(ProcastError::Cancelled);
    }
    let input = plan(info)?;
    match options.mode {
        TranscodeMode::AudioOnly => {
            media::validate_audio_transcode_input(info, options.playback_policy)?
        }
        TranscodeMode::Remux => media::validate_remux_input(info, options.playback_policy)?,
        TranscodeMode::AudioVideo => {}
    }
    check_encoders(ffmpeg, input.audio.is_some(), options.mode, cancel).await?;
    let parent = match &options.directory {
        Some(path) => path.clone(),
        None => cache_directory()?,
    };
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|e| error(format!("cannot create preparation directory: {e}")))?;
    // Upper-rate estimate plus muxing overhead and headroom; not a reservation.
    let estimate = match options.mode {
        TranscodeMode::AudioVideo => (input.duration * 8_192_000.0 / 8.0 * 1.1).ceil() as u64,
        TranscodeMode::AudioOnly | TranscodeMode::Remux => {
            // Copied video has no encoder rate cap. Use the entire source size
            // plus the new AAC stream and muxing overhead as a conservative estimate.
            let size = tokio::fs::metadata(&info.path)
                .await
                .map_err(|e| error(format!("cannot check source size: {e}")))?
                .len();
            let new_audio = if options.mode == TranscodeMode::AudioOnly {
                input.duration * 192_000.0 / 8.0
            } else {
                0.0
            };
            ((size as f64 + new_audio) * 1.1).ceil() as u64
        }
    };
    require_space(
        available_space(&parent)?,
        estimate.saturating_add(RESERVE_BYTES),
    )?;
    let directory = tempfile::Builder::new()
        .prefix("session-")
        .tempdir_in(&parent)
        .map_err(|e| error(format!("cannot create private preparation directory: {e}")))?;
    let path = directory.path().join("video.mp4");
    let result = async {
        let mut command = Command::new(ffmpeg);
        command.args(["-nostdin", "-hide_banner", "-v", "error", "-nostats", "-progress", "pipe:1", "-stats_period", "1", "-n", "-copyts", "-start_at_zero", "-i"])
            .arg(&info.path).args(["-map", &format!("0:{}", input.video)]);
        if let Some(index) = input.audio { command.args(["-map", &format!("0:{index}")]); }
        command.args(["-map_metadata", "-1", "-map_chapters", "-1", "-sn", "-dn"]);
        match options.mode {
            TranscodeMode::AudioOnly | TranscodeMode::Remux => { command.args(["-c:v", "copy"]); }
            TranscodeMode::AudioVideo => {
                let filter = format!("scale=w='min(1920,iw)':h='min(1080,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2,fps={:.8},format=yuv420p", input.fps);
                command.args(["-vf", &filter, "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-profile:v", "high", "-level:v", "4.1",
                    "-maxrate", "8M", "-bufsize", "16M", "-pix_fmt", "yuv420p"]);
            }
        }
        if input.audio.is_some() {
            if options.mode == TranscodeMode::Remux { command.args(["-c:a", "copy"]); }
            else { command.args(["-c:a", "aac", "-b:a", "192k", "-ac", "2", "-ar", "48000"]); }
        }
        command.args(["-movflags", "+faststart", "-f", "mp4"]).arg(&path);
        update(TranscodeProgress { encoded_seconds: 0.0, fraction: 0.0 });
        process::run(&mut command, cancel, options.timeout, |stdout| read_progress(stdout, input.duration, |progress| {
            require_space(available_space(directory.path())?, RESERVE_BYTES)?;
            update(progress);
            Ok(())
        })).await?;
        let output = media::inspect(&path, probe, cancel).await?;
        media::assess_direct_play(&output, options.playback_policy)?;
        if options.mode == TranscodeMode::Remux {
            let source_audio = info.streams.iter().find(|s| s.kind == "audio");
            let output_audio = output.streams.iter().find(|s| s.kind == "audio");
            if source_audio.map(|s| (&s.codec, &s.profile, s.channels, s.sample_rate_hz))
                != output_audio.map(|s| (&s.codec, &s.profile, s.channels, s.sample_rate_hz)) {
                return Err(error("remuxed audio is missing or differs from the source"));
            }
        } else if input.audio.is_some() && !output.streams.iter().any(|s|
            s.kind == "audio" && s.codec.as_deref() == Some("aac")
                && s.channels == Some(2) && s.sample_rate_hz == Some(48000)) {
            return Err(error("prepared stereo AAC audio is missing or invalid"));
        }
        let output_duration = output.duration_seconds.unwrap();
        if (output_duration - input.duration).abs() > (input.duration * 0.001).max(2.0) {
            return Err(error("prepared duration differs from the source; refusing potentially truncated output"));
        }
        update(TranscodeProgress { encoded_seconds: input.duration, fraction: 1.0 });
        Ok(output)
    }.await;
    match result {
        Ok(info) => Ok(PreparedMedia {
            info,
            directory: Some(directory),
        }),
        Err(cause) => {
            if let Err(cleanup) = directory.close() {
                return Err(error(format!(
                    "{cause}; additionally could not remove partial output: {cleanup}"
                )));
            }
            Err(cause)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn progress_is_incremental_bounded_and_finite() {
        let mut updates = Vec::new();
        read_progress(&b"out_time_us=1000000\nprogress=continue\nout_time_us=NaN\nprogress=continue\nout_time_us=9000000\nprogress=end\n"[..], 2.0, |p| { updates.push(p.fraction); Ok(()) }).await.unwrap();
        assert_eq!(updates, [0.5, 0.5, 0.99]);
        let long = vec![b'x'; MAX_PROGRESS_LINE + 1];
        assert!(read_progress(&long[..], 2.0, |_| Ok(())).await.is_err());
        assert!(require_space(1024, 2048).is_err());
        assert!(require_space(2048, 2048).is_ok());
    }
}
