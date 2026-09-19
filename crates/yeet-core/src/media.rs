use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::{CancellationToken, YeetError, process};

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
#[derive(Debug, Clone, Serialize)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub container: String,
    pub duration_seconds: Option<f64>,
    pub streams: Vec<StreamInfo>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StreamInfo {
    pub index: u32,
    pub kind: String,
    pub codec: Option<String>,
    pub profile: Option<String>,
    pub level: Option<i32>,
    pub pixel_format: Option<String>,
    pub frame_rate: Option<f64>,
    pub color_transfer: Option<String>,
    pub dolby_vision: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u32>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    pub forced: bool,
    pub hearing_impaired: bool,
    pub attached_picture: bool,
}

/// Inspect a regular local file. Cancel via the token and keep awaiting this
/// function until it returns; dropping the future cannot guarantee reaping.
pub async fn inspect(
    path: &Path,
    options: &ProbeOptions,
    cancellation: &CancellationToken,
) -> Result<MediaInfo, YeetError> {
    let prepare = async {
        let canonical =
            tokio::fs::canonicalize(path)
                .await
                .map_err(|source| YeetError::InputIo {
                    path: path.into(),
                    source,
                })?;
        let metadata =
            tokio::fs::metadata(&canonical)
                .await
                .map_err(|source| YeetError::InputIo {
                    path: path.into(),
                    source,
                })?;
        if !metadata.is_file() {
            return Err(YeetError::NotRegularFile(path.into()));
        }
        Ok(canonical)
    };
    let canonical = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(YeetError::Cancelled),
        result = prepare => result?,
    };
    let mut command = Command::new(&options.executable);
    command.args([
        "-v", "error", "-print_format", "json", "-show_format", "-show_streams",
        "-show_entries",
        "format=format_name,duration:stream=index,codec_type,codec_name,profile,level,pix_fmt,r_frame_rate,avg_frame_rate,color_transfer,width,height,sample_rate,channels:stream_tags=language,title:stream_disposition=default,forced,hearing_impaired,attached_pic:stream_side_data=side_data_type",
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
    #[serde(default)]
    side_data_list: Vec<ProbeSideData>,
}

#[derive(Deserialize)]
struct ProbeSideData {
    side_data_type: Option<String>,
}

fn parse(bytes: &[u8], path: PathBuf) -> Result<MediaInfo, YeetError> {
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
                dolby_vision: stream.side_data_list.iter().any(|data| {
                    data.side_data_type
                        .as_deref()
                        .is_some_and(|kind| kind.contains("DOVI") || kind.contains("Dolby Vision"))
                }),
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
                hearing_impaired: stream
                    .disposition
                    .get("hearing_impaired")
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
    /// Bounded HEVC Main/Main 10 SDR, with mono/stereo AAC only.
    Hevc,
    /// H.264 with up to six-channel AAC-LC, without HEVC admission.
    AacSurround,
    /// HEVC and AAC-LC surround, with Dolby audio requiring conversion.
    Extended,
    /// H.264 through Level 4.2/1080p60, with independent codec/audio opt-ins.
    H264HighFrameRate {
        allow_hevc: bool,
        allow_aac_surround: bool,
    },
    /// Additionally allow multichannel AAC-LC, bounded HEVC, and H.264/AC-3 trials.
    Experimental,
}

impl DirectPlayPolicy {
    pub fn allows_h264_high_frame_rate(self) -> bool {
        matches!(self, Self::H264HighFrameRate { .. })
    }
    pub fn allows_hevc(self) -> bool {
        matches!(
            self,
            Self::Hevc
                | Self::Extended
                | Self::Experimental
                | Self::H264HighFrameRate {
                    allow_hevc: true,
                    ..
                }
        )
    }
    pub fn allows_aac_surround(self) -> bool {
        matches!(
            self,
            Self::AacSurround
                | Self::Extended
                | Self::Experimental
                | Self::H264HighFrameRate {
                    allow_aac_surround: true,
                    ..
                }
        )
    }
}

/// An assessment of metadata, not a promise of successful receiver playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectPlayAssessment {
    ConservativeProfile,
    ExtendedH264,
    ExperimentalAacSurround { channels: u32 },
    ExperimentalHevc { audio_channels: Option<u32> },
    ExperimentalAc3 { channels: u32 },
}

/// Conservative M2 policy, not receiver capability negotiation or a full decode.
pub fn validate_direct_play(info: &MediaInfo) -> Result<(), YeetError> {
    assess_direct_play(info, DirectPlayPolicy::Conservative).map(|_| ())
}

/// Decide whether the original MP4 can be tried under the selected policy.
/// This never prepares, remuxes, or transcodes media.
pub fn assess_direct_play(
    info: &MediaInfo,
    policy: DirectPlayPolicy,
) -> Result<DirectPlayAssessment, YeetError> {
    assess_input(info, policy, InputMode::Original)
}

/// Audio conversion preserves video already supported by the selected profile.
pub(crate) fn validate_audio_transcode_input(
    info: &MediaInfo,
    policy: DirectPlayPolicy,
) -> Result<(), YeetError> {
    assess_input(info, policy, InputMode::AudioOnly).map(|_| ())
}

/// Admit compatible streams for copying into MP4, without weakening direct play.
pub(crate) fn validate_remux_input(
    info: &MediaInfo,
    policy: DirectPlayPolicy,
) -> Result<(), YeetError> {
    assess_input(info, policy, InputMode::Remux).map(|_| ())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Original,
    AudioOnly,
    Remux,
}

fn assess_input(
    info: &MediaInfo,
    policy: DirectPlayPolicy,
    mode: InputMode,
) -> Result<DirectPlayAssessment, YeetError> {
    let unsupported = |reason: &str| {
        YeetError::UnsupportedMedia(format!(
            "{reason}. The baseline profile requires MP4-family H.264 (8-bit 4:2:0, up to 1080p/30 and level 4.1) with optional mono/stereo AAC-LC. Experimental mode also permits HEVC Main/Main 10 up to level 4.0 at 1080p30, 3–6 channel AAC-LC, and H.264 with AC-3, without known HDR signalling. Opt in to H.264 Level 4.2/1080p60 with compatibility.allow_h264_high_frame_rate. Use automatic mode to select conversion, or --mode transcode to force SDR H.264/AAC output"
        ))
    };
    if !info
        .container
        .split(',')
        .any(|format| format == "mp4" || (mode != InputMode::Original && format == "matroska"))
    {
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
    let h264 = video.codec.as_deref() == Some("h264")
        && matches!(
            video.profile.as_deref(),
            Some("Constrained Baseline" | "Baseline" | "Main" | "High")
        )
        && video.level.is_some_and(|level| {
            (9..=if policy.allows_h264_high_frame_rate() {
                42
            } else {
                41
            })
                .contains(&level)
        })
        && video.pixel_format.as_deref() == Some("yuv420p");
    // ffprobe reports HEVC levels in units of 30 (120 means level 4.0).
    let hevc = video.codec.as_deref() == Some("hevc")
        && matches!(
            (video.profile.as_deref(), video.pixel_format.as_deref()),
            (Some("Main"), Some("yuv420p")) | (Some("Main 10"), Some("yuv420p" | "yuv420p10le"))
        )
        && matches!(video.level, Some(30 | 60 | 63 | 90 | 93 | 120));
    if !(h264 || hevc)
        || !video.width.is_some_and(|width| (1..=1920).contains(&width))
        || !video
            .height
            .is_some_and(|height| (1..=1080).contains(&height))
        || !video.frame_rate.is_some_and(|rate| {
            rate.is_finite()
                && rate > 0.0
                && rate
                    <= if h264 && policy.allows_h264_high_frame_rate() {
                        60.01
                    } else {
                        30.01
                    }
        })
        || matches!(
            video.color_transfer.as_deref(),
            Some("smpte2084" | "arib-std-b67")
        )
        || video.dolby_vision
    {
        return Err(unsupported(&format!(
            "video stream #{} is outside the supported profile (or lacks required metadata)",
            video.index
        )));
    }
    if hevc && !policy.allows_hevc() {
        return Err(unsupported(
            "HEVC requires --profile extended or compatibility.allow_hevc for copying or original-file playback",
        ));
    }
    let audio: Vec<_> = info.streams.iter().filter(|s| s.kind == "audio").collect();
    if audio.len() > 1 {
        return Err(unsupported(
            "multiple audio tracks need explicit selection/remuxing",
        ));
    }
    if mode == InputMode::AudioOnly {
        if audio.is_empty() {
            return Err(unsupported(
                "audio-only conversion requires one audio track",
            ));
        }
        return Ok(DirectPlayAssessment::ConservativeProfile);
    }
    if let Some(audio) = audio.first() {
        let aac = audio.codec.as_deref() == Some("aac")
            && audio.profile.as_deref() == Some("LC")
            && audio
                .sample_rate_hz
                .is_some_and(|rate| (8000..=48000).contains(&rate));
        let ac3 = h264
            && audio.codec.as_deref() == Some("ac3")
            && matches!(audio.sample_rate_hz, Some(32000 | 44100 | 48000));
        if !(aac || ac3)
            || !audio
                .channels
                .is_some_and(|channels| (1..=6).contains(&channels))
        {
            return Err(unsupported(&format!(
                "audio stream #{} is outside the available direct-play profiles (or lacks required metadata)",
                audio.index
            )));
        }
        if ac3 {
            return match policy {
                DirectPlayPolicy::Conservative
                | DirectPlayPolicy::Hevc
                | DirectPlayPolicy::AacSurround
                | DirectPlayPolicy::Extended
                | DirectPlayPolicy::H264HighFrameRate { .. } => Err(unsupported(
                    "AC-3 passthrough requires --profile experimental; audio output depends on the receiver and connected equipment",
                )),
                DirectPlayPolicy::Experimental => Ok(DirectPlayAssessment::ExperimentalAc3 {
                    channels: audio.channels.unwrap(),
                }),
            };
        }
    }
    let audio_channels = audio.first().and_then(|audio| audio.channels);
    // Check audio independently of video: enabling HEVC alone must not silently
    // admit surround AAC on the early HEVC assessment return.
    if audio_channels.is_some_and(|channels| channels > 2) && !policy.allows_aac_surround() {
        return Err(unsupported(
            "multichannel AAC-LC requires --profile extended or compatibility.allow_aac_surround for copying or original-file playback; check audible audio/downmix on your receiver",
        ));
    }
    if hevc {
        return Ok(DirectPlayAssessment::ExperimentalHevc { audio_channels });
    }
    if let Some(channels) = audio_channels
        && channels > 2
    {
        return Ok(DirectPlayAssessment::ExperimentalAacSurround { channels });
    }
    if video.level.is_some_and(|level| level > 41)
        || video.frame_rate.is_some_and(|rate| rate > 30.01)
    {
        return Ok(DirectPlayAssessment::ExtendedH264);
    }
    Ok(DirectPlayAssessment::ConservativeProfile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remux_admits_matroska_without_weakening_stream_or_direct_play_policy() {
        let mut fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/h264.json")).unwrap();
        fixture["format"]["format_name"] = serde_json::json!("matroska,webm");
        let read = |value: &serde_json::Value| {
            parse(&serde_json::to_vec(value).unwrap(), "movie.mkv".into()).unwrap()
        };
        let info = read(&fixture);
        assert!(validate_remux_input(&info, DirectPlayPolicy::Conservative).is_ok());
        assert!(validate_direct_play(&info).is_err());
        assert!(assess_direct_play(&info, DirectPlayPolicy::Experimental).is_err());
        assert!(validate_audio_transcode_input(&info, DirectPlayPolicy::Conservative).is_ok());
        let mut silent = fixture.clone();
        silent["streams"].as_array_mut().unwrap().truncate(1);
        assert!(validate_remux_input(&read(&silent), DirectPlayPolicy::Conservative).is_ok());
        let mut mono = fixture.clone();
        mono["streams"][1]["channels"] = serde_json::json!(1);
        assert!(validate_remux_input(&read(&mono), DirectPlayPolicy::Conservative).is_ok());
        for (pointer, value) in [
            ("/format/format_name", serde_json::json!("mpegts")),
            ("/format/duration", serde_json::json!("N/A")),
            ("/streams/0/codec_name", serde_json::json!("hevc")),
            ("/streams/0/width", serde_json::json!(3840)),
            ("/streams/0/level", serde_json::json!(42)),
            ("/streams/0/pix_fmt", serde_json::Value::Null),
            ("/streams/1/codec_name", serde_json::json!("ac3")),
            ("/streams/1/profile", serde_json::json!("HE-AAC")),
            ("/streams/1/channels", serde_json::json!(6)),
            ("/streams/1/channels", serde_json::Value::Null),
        ] {
            let mut invalid = fixture.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(
                validate_remux_input(&read(&invalid), DirectPlayPolicy::Conservative).is_err(),
                "{pointer}"
            );
        }
        fixture["streams"][0]["color_transfer"] = serde_json::json!("smpte2084");
        assert!(validate_remux_input(&read(&fixture), DirectPlayPolicy::Conservative).is_err());
    }

    #[test]
    fn ac3_trials_require_opt_in_and_keep_codec_and_track_limits() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/h264-ac3.json")).unwrap();
        let check = |value: &serde_json::Value, policy| {
            let info = parse(&serde_json::to_vec(value).unwrap(), "movie.mp4".into()).unwrap();
            assess_direct_play(&info, policy)
        };
        assert!(
            check(&fixture, DirectPlayPolicy::Conservative)
                .unwrap_err()
                .to_string()
                .contains("AC-3 passthrough requires --profile experimental")
        );
        for channels in [1, 2, 6] {
            for rate in ["32000", "44100", "48000"] {
                let mut valid = fixture.clone();
                valid["streams"][1]["channels"] = serde_json::json!(channels);
                valid["streams"][1]["sample_rate"] = serde_json::json!(rate);
                assert_eq!(
                    check(&valid, DirectPlayPolicy::Experimental).unwrap(),
                    DirectPlayAssessment::ExperimentalAc3 { channels }
                );
            }
        }
        for (pointer, value) in [
            ("/format/format_name", serde_json::json!("matroska,webm")),
            ("/streams/0/width", serde_json::json!(3840)),
            ("/streams/1/codec_name", serde_json::json!("eac3")),
            ("/streams/1/codec_name", serde_json::json!("dts")),
            ("/streams/1/channels", serde_json::json!(0)),
            ("/streams/1/channels", serde_json::json!(8)),
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
        let mut hdr = fixture.clone();
        hdr["streams"][0]["color_transfer"] = serde_json::json!("smpte2084");
        assert!(check(&hdr, DirectPlayPolicy::Experimental).is_err());
        let mut multiple = fixture.clone();
        multiple["streams"]
            .as_array_mut()
            .unwrap()
            .push(fixture["streams"][1].clone());
        assert!(check(&multiple, DirectPlayPolicy::Experimental).is_err());
    }

    #[test]
    fn hevc_trials_require_opt_in_and_reject_unsupported_combinations() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/hevc.json")).unwrap();
        let check = |value: &serde_json::Value, policy| {
            let info = parse(&serde_json::to_vec(value).unwrap(), "movie.mp4".into()).unwrap();
            assess_direct_play(&info, policy)
        };
        assert!(
            check(&fixture, DirectPlayPolicy::Conservative)
                .unwrap_err()
                .to_string()
                .contains("--profile extended")
        );
        assert_eq!(
            check(&fixture, DirectPlayPolicy::Experimental).unwrap(),
            DirectPlayAssessment::ExperimentalHevc {
                audio_channels: Some(6)
            }
        );
        let mut main = fixture.clone();
        main["streams"][0]["profile"] = serde_json::json!("Main");
        main["streams"][0]["pix_fmt"] = serde_json::json!("yuv420p");
        main["streams"][1]["channels"] = serde_json::json!(2);
        assert_eq!(
            check(&main, DirectPlayPolicy::Experimental).unwrap(),
            DirectPlayAssessment::ExperimentalHevc {
                audio_channels: Some(2)
            }
        );
        main["streams"].as_array_mut().unwrap().truncate(1);
        assert_eq!(
            check(&main, DirectPlayPolicy::Experimental).unwrap(),
            DirectPlayAssessment::ExperimentalHevc {
                audio_channels: None
            }
        );
        for (pointer, value) in [
            ("/format/format_name", serde_json::json!("matroska,webm")),
            ("/streams/0/codec_name", serde_json::json!("av1")),
            ("/streams/0/profile", serde_json::json!("Main")), // Main cannot carry 10-bit pixels.
            ("/streams/0/profile", serde_json::json!("Rext")),
            ("/streams/0/pix_fmt", serde_json::json!("yuv422p10le")),
            ("/streams/0/level", serde_json::json!(123)),
            ("/streams/0/level", serde_json::Value::Null),
            ("/streams/0/width", serde_json::json!(3840)),
            ("/streams/0/r_frame_rate", serde_json::json!("60/1")),
            ("/streams/1/codec_name", serde_json::json!("ac3")),
            ("/streams/1/profile", serde_json::json!("HE-AAC")),
            ("/streams/1/channels", serde_json::json!(8)),
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
        let mut dolby = fixture.clone();
        dolby["streams"][0]["side_data_list"] =
            serde_json::json!([{"side_data_type":"DOVI configuration record"}]);
        assert!(check(&dolby, DirectPlayPolicy::Experimental).is_err());
    }

    #[test]
    fn experimental_h264_policy_preserves_video_and_audio_bounds() {
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
            assert!(error.to_string().contains("--profile extended"));
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
            ("/streams/1/codec_name", serde_json::json!("eac3")),
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
        let mut dolby = fixture.clone();
        dolby["streams"][0]["side_data_list"] = serde_json::json!([
            {"side_data_type":"DOVI configuration record"}
        ]);
        let info = parse(&serde_json::to_vec(&dolby).unwrap(), "movie.mp4".into()).unwrap();
        assert!(info.streams[0].dolby_vision);
        assert!(assess_direct_play(&info, DirectPlayPolicy::Experimental).is_err());
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
                Err(YeetError::InvalidMetadata(_))
            ));
        }
    }
}
