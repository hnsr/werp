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
        "format=format_name,duration:stream=index,codec_type,codec_name,profile,width,height,sample_rate,channels:stream_tags=language,title:stream_disposition=default,forced,attached_pic",
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

#[cfg(test)]
mod tests {
    use super::*;

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
