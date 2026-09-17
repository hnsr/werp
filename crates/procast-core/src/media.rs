use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::{CancellationToken, ProcastError, process};

#[derive(Debug, Clone)]
pub struct ProbeOptions {
    pub executable: PathBuf,
    pub timeout: Duration,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        Self {
            executable: "ffprobe".into(),
            timeout: Duration::from_secs(30),
        }
    }
}

/// Normalized metadata, not a promise that a receiver can play this file.
#[derive(Debug, Serialize)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub container: String,
    pub duration_seconds: Option<f64>,
    pub streams: Vec<StreamInfo>,
}

#[derive(Debug, Serialize)]
pub struct StreamInfo {
    pub index: u32,
    pub kind: String,
    pub codec: Option<String>,
    pub profile: Option<String>,
    pub level: Option<i32>,
    pub pixel_format: Option<String>,
    pub frame_rate: Option<f64>,
    pub color_transfer: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u32>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    pub forced: bool,
    pub attached_picture: bool,
}

/// Inspect a regular local file. Cancel via the token and keep awaiting this
/// function until it returns; dropping the future cannot guarantee reaping.
pub async fn inspect(
    path: &Path,
    options: &ProbeOptions,
    cancellation: &CancellationToken,
) -> Result<MediaInfo, ProcastError> {
    let prepare = async {
        let canonical =
            tokio::fs::canonicalize(path)
                .await
                .map_err(|source| ProcastError::InputIo {
                    path: path.into(),
                    source,
                })?;
        let metadata =
            tokio::fs::metadata(&canonical)
                .await
                .map_err(|source| ProcastError::InputIo {
                    path: path.into(),
                    source,
                })?;
        if !metadata.is_file() {
            return Err(ProcastError::NotRegularFile(path.into()));
        }
        Ok(canonical)
    };
    let canonical = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(ProcastError::Cancelled),
        result = prepare => result?,
    };
    let mut command = Command::new(&options.executable);
    command.args([
        "-v", "error", "-print_format", "json", "-show_format", "-show_streams",
        "-show_entries",
        "format=format_name,duration:stream=index,codec_type,codec_name,profile,level,pix_fmt,r_frame_rate,avg_frame_rate,color_transfer,width,height,sample_rate,channels:stream_tags=language,title:stream_disposition=default,forced,attached_pic",
        "-i",
    ]).arg(&canonical);
    let bytes = process::capture(&mut command, cancellation, options.timeout).await?;
    parse(&bytes, canonical)
}

#[derive(Deserialize)]
struct ProbeOutput {
    format: ProbeFormat,
    #[serde(default)]
    streams: Vec<ProbeStream>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    format_name: String,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct ProbeStream {
    index: u32,
    codec_type: Option<String>,
    codec_name: Option<String>,
    profile: Option<String>,
    level: Option<i32>,
    pix_fmt: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    color_transfer: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    #[serde(default)]
    tags: BTreeMap<String, String>,
    #[serde(default)]
    disposition: BTreeMap<String, u32>,
}

fn parse(bytes: &[u8], path: PathBuf) -> Result<MediaInfo, ProcastError> {
    let probe: ProbeOutput = serde_json::from_slice(bytes)?;
    let duration_seconds = probe
        .format
        .duration
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration >= 0.0);
    Ok(MediaInfo {
        path,
        container: probe.format.format_name,
        duration_seconds,
        streams: probe
            .streams
            .into_iter()
            .map(|stream| StreamInfo {
                index: stream.index,
                kind: stream.codec_type.unwrap_or_else(|| "unknown".into()),
                codec: stream.codec_name,
                profile: stream.profile,
                level: stream.level,
                pixel_format: stream.pix_fmt,
                frame_rate: [stream.r_frame_rate, stream.avg_frame_rate]
                    .into_iter()
                    .flatten()
                    .filter_map(|rate| parse_rate(&rate))
                    .reduce(f64::max),
                color_transfer: stream.color_transfer,
                width: stream.width,
                height: stream.height,
                sample_rate_hz: stream.sample_rate.and_then(|text| text.parse().ok()),
                channels: stream.channels,
                language: stream.tags.get("language").cloned(),
                title: stream.tags.get("title").cloned(),
                default: stream
                    .disposition
                    .get("default")
                    .is_some_and(|value| *value != 0),
                forced: stream
                    .disposition
                    .get("forced")
                    .is_some_and(|value| *value != 0),
                attached_picture: stream
                    .disposition
                    .get("attached_pic")
                    .is_some_and(|value| *value != 0),
            })
            .collect(),
    })
}

fn parse_rate(rate: &str) -> Option<f64> {
    let (numerator, denominator) = rate.split_once('/')?;
    let rate = numerator.parse::<f64>().ok()? / denominator.parse::<f64>().ok()?;
    (rate.is_finite() && rate > 0.0).then_some(rate)
}

/// Local admission policy, not receiver capability negotiation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DirectPlayPolicy {
    #[default]
    Conservative,
    /// Additionally allow AAC-LC with 3–6 channels; retain all video/container checks.
    Experimental,
}

/// An assessment of metadata, not a promise of successful receiver playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectPlayAssessment {
    ConservativeProfile,
    ExperimentalAacSurround { channels: u32 },
}

/// Conservative M2 policy, not receiver capability negotiation or a full decode.
pub fn validate_direct_play(info: &MediaInfo) -> Result<(), ProcastError> {
    assess_direct_play(info, DirectPlayPolicy::Conservative).map(|_| ())
}

/// Decide whether the original MP4 can be tried under the selected policy.
/// This never prepares, remuxes, or transcodes media.
pub fn assess_direct_play(
    info: &MediaInfo,
    policy: DirectPlayPolicy,
) -> Result<DirectPlayAssessment, ProcastError> {
    let unsupported = |reason: &str| {
        ProcastError::UnsupportedMedia(format!(
            "{reason}. Direct play requires MP4-family H.264 (8-bit 4:2:0, up to 1080p/30 and level 4.1) with optional mono/stereo AAC-LC; experimental mode additionally permits 3–6 channel AAC-LC. Remuxing/transcoding is not implemented yet"
        ))
    };
    if !info.container.split(',').any(|format| format == "mp4") {
        return Err(unsupported(&format!(
            "container {} needs a supported container",
            info.container
        )));
    }
    if !info
        .duration_seconds
        .is_some_and(|duration| duration.is_finite() && duration > 0.0)
    {
        return Err(unsupported(
            "a finite positive duration could not be determined",
        ));
    }
    let videos: Vec<_> = info
        .streams
        .iter()
        .filter(|s| s.kind == "video" && !s.attached_picture)
        .collect();
    if videos.len() != 1 {
        return Err(unsupported("exactly one video stream is required"));
    }
    let video = videos[0];
    if video.codec.as_deref() != Some("h264")
        || !matches!(
            video.profile.as_deref(),
            Some("Constrained Baseline" | "Baseline" | "Main" | "High")
        )
        || !video.level.is_some_and(|level| (9..=41).contains(&level))
        || video.pixel_format.as_deref() != Some("yuv420p")
        || !video.width.is_some_and(|width| (1..=1920).contains(&width))
        || !video
            .height
            .is_some_and(|height| (1..=1080).contains(&height))
        || !video
            .frame_rate
            .is_some_and(|rate| rate.is_finite() && rate > 0.0 && rate <= 30.01)
        || matches!(
            video.color_transfer.as_deref(),
            Some("smpte2084" | "arib-std-b67")
        )
    {
        return Err(unsupported(&format!(
            "video stream #{} is outside the supported profile (or lacks required metadata)",
            video.index
        )));
    }
    let audio: Vec<_> = info.streams.iter().filter(|s| s.kind == "audio").collect();
    if audio.len() > 1 {
        return Err(unsupported(
            "multiple audio tracks need explicit selection/remuxing",
        ));
    }
    if let Some(audio) = audio.first()
        && (audio.codec.as_deref() != Some("aac")
            || audio.profile.as_deref() != Some("LC")
            || !audio
                .channels
                .is_some_and(|channels| (1..=6).contains(&channels))
            || !audio
                .sample_rate_hz
                .is_some_and(|rate| (8000..=48000).contains(&rate)))
    {
        return Err(unsupported(&format!(
            "audio stream #{} is outside the available direct-play profiles (or lacks required metadata)",
            audio.index
        )));
    }
    if let Some(channels) = audio.first().and_then(|audio| audio.channels)
        && channels > 2
    {
        return match policy {
            DirectPlayPolicy::Conservative => Err(unsupported(
                "multichannel AAC-LC is unverified; use --experimental-direct-play to try the original file and check audible audio/downmix on your receiver",
            )),
            DirectPlayPolicy::Experimental => {
                Ok(DirectPlayAssessment::ExperimentalAacSurround { channels })
            }
        };
    }
    Ok(DirectPlayAssessment::ConservativeProfile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experimental_policy_only_relaxes_aac_lc_channel_count() {
        let mut fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/h264.json")).unwrap();
        let check = |value: &serde_json::Value, policy| {
            let info = parse(&serde_json::to_vec(value).unwrap(), "movie.mp4".into()).unwrap();
            assess_direct_play(&info, policy)
        };
        assert_eq!(
            check(&fixture, DirectPlayPolicy::Experimental).unwrap(),
            DirectPlayAssessment::ConservativeProfile
        );
        for channels in 3..=6 {
            fixture["streams"][1]["channels"] = serde_json::json!(channels);
            let error = check(&fixture, DirectPlayPolicy::Conservative).unwrap_err();
            assert!(error.to_string().contains("--experimental-direct-play"));
            assert_eq!(
                check(&fixture, DirectPlayPolicy::Experimental).unwrap(),
                DirectPlayAssessment::ExperimentalAacSurround { channels }
            );
        }
        for (pointer, value) in [
            ("/format/format_name", serde_json::json!("matroska,webm")),
            ("/format/duration", serde_json::json!("N/A")),
            ("/streams/0/codec_name", serde_json::json!("hevc")),
            ("/streams/0/level", serde_json::json!(42)),
            ("/streams/0/pix_fmt", serde_json::json!("yuv420p10le")),
            ("/streams/0/width", serde_json::json!(3840)),
            ("/streams/0/r_frame_rate", serde_json::json!("60/1")),
            ("/streams/1/codec_name", serde_json::json!("ac3")),
            ("/streams/1/profile", serde_json::json!("HE-AAC")),
            ("/streams/1/channels", serde_json::json!(8)),
            ("/streams/1/channels", serde_json::json!(0)),
            ("/streams/1/channels", serde_json::Value::Null),
            ("/streams/1/sample_rate", serde_json::json!("96000")),
            ("/streams/1/sample_rate", serde_json::Value::Null),
        ] {
            let mut invalid = fixture.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(
                check(&invalid, DirectPlayPolicy::Experimental).is_err(),
                "{pointer}"
            );
        }
        for transfer in ["smpte2084", "arib-std-b67"] {
            let mut hdr = fixture.clone();
            hdr["streams"][0]["color_transfer"] = serde_json::json!(transfer);
            assert!(check(&hdr, DirectPlayPolicy::Experimental).is_err());
        }
        let mut multiple = fixture.clone();
        multiple["streams"]
            .as_array_mut()
            .unwrap()
            .push(fixture["streams"][1].clone());
        assert!(check(&multiple, DirectPlayPolicy::Experimental).is_err());
    }

    #[test]
    fn direct_play_policy_rejects_unsupported_and_unknown_metadata() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/h264.json")).unwrap();
        let check = |value: &serde_json::Value| {
            let info = parse(&serde_json::to_vec(value).unwrap(), "movie.mp4".into()).unwrap();
            validate_direct_play(&info)
        };
        assert!(check(&fixture).is_ok());
        for (pointer, value) in [
            ("/format/format_name", serde_json::json!("matroska,webm")),
            ("/format/duration", serde_json::json!("N/A")),
            ("/streams/0/codec_name", serde_json::json!("hevc")),
            ("/streams/0/profile", serde_json::json!("High 10")),
            ("/streams/0/pix_fmt", serde_json::json!("yuv420p10le")),
            ("/streams/0/level", serde_json::json!(42)),
            ("/streams/0/width", serde_json::json!(3840)),
            ("/streams/0/r_frame_rate", serde_json::json!("60/1")),
            ("/streams/1/codec_name", serde_json::json!("eac3")),
            ("/streams/1/channels", serde_json::json!(6)),
            ("/streams/1/profile", serde_json::json!("HE-AAC")),
        ] {
            let mut value_fixture = fixture.clone();
            *value_fixture.pointer_mut(pointer).unwrap() = value;
            assert!(check(&value_fixture).is_err(), "{pointer}");
        }
        let mut unknown = fixture.clone();
        unknown["streams"][0]
            .as_object_mut()
            .unwrap()
            .remove("pix_fmt");
        assert!(check(&unknown).is_err());
        let mut silent = fixture.clone();
        silent["streams"].as_array_mut().unwrap().truncate(1);
        assert!(check(&silent).is_ok());
        let mut multiple = fixture.clone();
        multiple["streams"]
            .as_array_mut()
            .unwrap()
            .push(fixture["streams"][1].clone());
        assert!(check(&multiple).is_err());
    }

    #[test]
    fn parses_tracks_and_tolerates_unavailable_metadata() {
        let info = parse(br#"{
            "format":{"format_name":"matroska,webm","duration":"N/A"},
            "streams":[
                {"index":0,"codec_type":"video","codec_name":"h264","width":1920,"height":1080},
                {"index":1,"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000","tags":{"language":"eng"},"disposition":{"default":1}},
                {"index":2,"codec_type":"subtitle","codec_name":"subrip","tags":{"title":"English"},"disposition":{"forced":1}},
                {"index":3,"codec_type":"attachment"}
            ]
        }"#, "movie.mkv".into()).unwrap();
        assert_eq!(info.duration_seconds, None);
        assert_eq!(info.streams[0].width, Some(1920));
        assert_eq!(info.streams[1].sample_rate_hz, Some(48000));
        assert_eq!(info.streams[1].language.as_deref(), Some("eng"));
        assert!(info.streams[1].default);
        assert!(info.streams[2].forced);
        assert_eq!(info.streams[3].codec, None);
    }

    #[test]
    fn rejects_malformed_probe_output() {
        for bytes in [b"not json".as_slice(), b"{}", br#"{"format": {}}"#] {
            assert!(matches!(
                parse(bytes, "movie".into()),
                Err(ProcastError::InvalidMetadata(_))
            ));
        }
    }
}
