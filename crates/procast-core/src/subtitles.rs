//! Validated, session-owned UTF-8 text subtitles. No source file is modified.
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use tempfile::TempDir;
use tokio::{io::AsyncReadExt, process::Command};

use crate::{
    CancellationToken, ProcastError,
    config::{SubtitlePreferences, normalize_language, normalize_preference},
    media::{MediaInfo, StreamInfo},
    process,
};

const MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub enum Request {
    #[default]
    Auto,
    Off,
    External(PathBuf),
    /// Absolute stream index reported by `inspect`, not subtitle ordinal.
    Embedded(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    External(PathBuf),
    Text { index: u32, styled: bool },
    Bitmap { index: u32 },
}

fn bitmap(codec: &str) -> bool {
    matches!(codec, "hdmv_pgs_subtitle" | "dvd_subtitle" | "dvb_subtitle")
}

fn supported_text(codec: &str) -> bool {
    matches!(
        codec,
        "subrip"
            | "srt"
            | "ass"
            | "ssa"
            | "webvtt"
            | "mov_text"
            | "text"
            | "ttml"
            | "sami"
            | "subviewer"
            | "microdvd"
    )
}

fn words(title: &str) -> Vec<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn language(stream: &StreamInfo) -> Option<&'static str> {
    if let Some(tag) = stream.language.as_deref()
        && !matches!(
            tag.trim().to_ascii_lowercase().as_str(),
            "" | "und" | "unknown"
        )
    {
        return normalize_language(tag);
    }
    words(stream.title.as_deref().unwrap_or(""))
        .iter()
        .find_map(|word| normalize_preference(word))
}

fn flags(stream: &StreamInfo) -> (bool, bool) {
    let title = words(stream.title.as_deref().unwrap_or(""));
    (
        stream.forced
            || title
                .iter()
                .any(|s| matches!(s.as_str(), "forced" | "signs" | "songs")),
        stream.hearing_impaired
            || title
                .iter()
                .any(|s| matches!(s.as_str(), "sdh" | "cc" | "hearing")),
    )
}

fn selection(stream: &StreamInfo) -> Result<Selection, ProcastError> {
    let codec = stream.codec.as_deref().unwrap_or("unknown");
    if bitmap(codec) {
        Ok(Selection::Bitmap {
            index: stream.index,
        })
    } else if supported_text(codec) {
        Ok(Selection::Text {
            index: stream.index,
            styled: matches!(codec, "ass" | "ssa"),
        })
    } else {
        Err(invalid(format!(
            "subtitle stream #{} uses unsupported codec {codec}",
            stream.index
        )))
    }
}

pub fn preferred_embedded<'a>(info: &'a MediaInfo, languages: &[String]) -> Option<&'a StreamInfo> {
    info.streams
        .iter()
        .filter(|s| s.kind == "subtitle")
        .filter_map(|stream| {
            let lang = language(stream)?;
            let priority = languages
                .iter()
                .position(|p| normalize_preference(p) == Some(lang))?;
            let codec = stream.codec.as_deref().unwrap_or("");
            if !bitmap(codec) && !supported_text(codec) {
                return None;
            }
            let (forced, sdh) = flags(stream);
            Some((
                (
                    priority,
                    forced,
                    bitmap(codec),
                    sdh,
                    !stream.default,
                    stream.index,
                ),
                stream,
            ))
        })
        .min_by_key(|(score, _)| *score)
        .map(|(_, stream)| stream)
}

pub async fn select(
    request: &Request,
    info: &MediaInfo,
    original_path: &Path,
    preferences: &SubtitlePreferences,
) -> Result<Option<Selection>, ProcastError> {
    match request {
        Request::Off => return Ok(None),
        Request::External(path) => return Ok(Some(Selection::External(path.clone()))),
        Request::Embedded(index) => {
            let stream = info
                .streams
                .iter()
                .find(|s| s.kind == "subtitle" && s.index == *index)
                .ok_or_else(|| {
                    invalid(format!(
                        "no embedded subtitle stream #{index}; use inspect to list stream indexes"
                    ))
                })?;
            return selection(stream).map(Some);
        }
        Request::Auto if !preferences.auto_load => return Ok(None),
        Request::Auto => {}
    }
    if let Some(stream) = preferred_embedded(info, &preferences.languages) {
        return selection(stream).map(Some);
    }
    // Inspect the real source directory first, then the supplied alias directory.
    for video in [&info.path, original_path] {
        let Some(parent) = video.parent() else {
            continue;
        };
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        let mut entries = tokio::fs::read_dir(parent)
            .await
            .map_err(|e| invalid(format!("cannot search for matching subtitles: {e}")))?;
        let mut matches = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| invalid(e.to_string()))?
        {
            let path = entry.path();
            if path.file_stem() == video.file_stem()
                && path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case("srt"))
                && tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_file())
            {
                matches.push(path);
            }
        }
        match matches.as_slice() {
            [path] => return Ok(Some(Selection::External(path.clone()))),
            [] => {}
            _ => {
                return Err(invalid(
                    "multiple matching .srt files; choose one with --subtitles",
                ));
            }
        }
    }
    Ok(None)
}

pub async fn extract(
    source: &MediaInfo,
    index: u32,
    ffmpeg: &Path,
    cancel: &CancellationToken,
) -> Result<PreparedSubtitles, ProcastError> {
    let stream = source
        .streams
        .iter()
        .find(|s| s.index == index && s.kind == "subtitle")
        .ok_or_else(|| invalid(format!("no subtitle stream #{index}")))?;
    if !supported_text(stream.codec.as_deref().unwrap_or("")) {
        return Err(invalid(
            "bitmap subtitles require video burn-in, not text extraction",
        ));
    }
    let mut command = Command::new(ffmpeg);
    command
        .args(["-nostdin", "-v", "error", "-copyts", "-start_at_zero", "-i"])
        .arg(&source.path)
        .args([
            "-map",
            &format!("0:{index}"),
            "-c:s",
            "webvtt",
            "-f",
            "webvtt",
            "pipe:1",
        ]);
    let bytes = process::capture(&mut command, cancel, Duration::from_secs(120)).await?;
    let output = String::from_utf8(bytes).map_err(|_| invalid("FFmpeg returned invalid UTF-8"))?;
    save_webvtt(&output, cancel).await
}

async fn save_webvtt(
    output: &str,
    cancel: &CancellationToken,
) -> Result<PreparedSubtitles, ProcastError> {
    validate(output, false)?;
    if cancel.is_cancelled() {
        return Err(ProcastError::Cancelled);
    }
    let directory = tempfile::Builder::new()
        .prefix("procast-subtitles-")
        .tempdir()
        .map_err(|e| invalid(e.to_string()))?;
    let path = directory.path().join("subtitles.vtt");
    tokio::fs::write(&path, output)
        .await
        .map_err(|e| invalid(e.to_string()))?;
    Ok(PreparedSubtitles { path, directory })
}

pub struct PreparedSubtitles {
    pub path: PathBuf,
    directory: TempDir,
}

impl PreparedSubtitles {
    pub fn close(self) -> Result<(), ProcastError> {
        self.directory.close().map_err(|e| {
            ProcastError::Subtitles(format!("could not remove temporary subtitles: {e}"))
        })
    }
}

pub async fn prepare(
    path: &Path,
    ffmpeg: &Path,
    cancel: &CancellationToken,
) -> Result<PreparedSubtitles, ProcastError> {
    let read = async {
        if !tokio::fs::metadata(path)
            .await
            .map_err(|e| invalid(format!("cannot access {path:?}: {e}")))?
            .is_file()
        {
            return Err(invalid("expected a regular subtitle file"));
        }
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| invalid(format!("cannot open {path:?}: {e}")))?;
        if !file
            .metadata()
            .await
            .map_err(|e| invalid(e.to_string()))?
            .is_file()
        {
            return Err(invalid("expected a regular subtitle file"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| invalid(e.to_string()))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("file exceeds the 4 MiB limit"));
        }
        let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
            if bytes.len() % 2 != 0 {
                return Err(invalid("invalid UTF-16 subtitle length"));
            }
            let little = bytes[0] == 0xff;
            let units: Vec<_> = bytes[2..]
                .chunks_exact(2)
                .map(|b| {
                    if little {
                        u16::from_le_bytes([b[0], b[1]])
                    } else {
                        u16::from_be_bytes([b[0], b[1]])
                    }
                })
                .collect();
            String::from_utf16(&units).map_err(|_| invalid("invalid UTF-16 subtitles"))?
        } else {
            String::from_utf8(bytes)
                .map_err(|_| invalid("subtitles must be UTF-8 or BOM-marked UTF-16"))?
        };
        Ok(text
            .trim_start_matches('\u{feff}')
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .lines()
            .map(|line| if line.trim().is_empty() { "" } else { line })
            .collect::<Vec<_>>()
            .join("\n"))
    };
    let text = tokio::select! { biased; _ = cancel.cancelled() => return Err(ProcastError::Cancelled), result = read => result? };
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let input_format = match extension.as_str() {
        "srt" => "srt",
        "vtt" => "webvtt",
        "ass" | "ssa" => "ass",
        _ => {
            return Err(invalid(
                "expected an external .srt, .vtt, .ass, or .ssa file",
            ));
        }
    };
    if input_format != "ass" {
        validate(&text, input_format == "srt")?;
    } else {
        tracing::warn!(
            "ASS/SSA converted to WebVTT; advanced positioning, fonts and effects are not preserved"
        );
    }
    let directory = tempfile::Builder::new()
        .prefix("procast-subtitles-")
        .tempdir()
        .map_err(|e| invalid(e.to_string()))?;
    let output = if input_format != "webvtt" {
        let input = directory.path().join(format!("input.{extension}"));
        tokio::fs::write(&input, text.as_bytes())
            .await
            .map_err(|e| invalid(e.to_string()))?;
        let mut command = Command::new(ffmpeg);
        command
            .args(["-nostdin", "-v", "error", "-f", input_format, "-i"])
            .arg(&input)
            .args(["-map", "0:s:0", "-c:s", "webvtt", "-f", "webvtt", "pipe:1"]);
        let bytes = process::capture(&mut command, cancel, Duration::from_secs(30)).await?;
        String::from_utf8(bytes).map_err(|_| invalid("FFmpeg returned invalid UTF-8"))?
    } else {
        text
    };
    validate(&output, false)?;
    if cancel.is_cancelled() {
        return Err(ProcastError::Cancelled);
    }
    let path = directory.path().join("subtitles.vtt");
    tokio::fs::write(&path, output)
        .await
        .map_err(|e| invalid(e.to_string()))?;
    Ok(PreparedSubtitles { path, directory })
}

fn invalid(reason: impl Into<String>) -> ProcastError {
    ProcastError::Subtitles(reason.into())
}

/// Validate timed cues, not styling semantics. Cue identifiers/settings and
/// NOTE/STYLE/REGION blocks are preserved for the WebVTT receiver to interpret.
fn validate(text: &str, srt: bool) -> Result<(), ProcastError> {
    if text.contains('\0') {
        return Err(invalid("NUL characters are not allowed"));
    }
    let mut blocks = text.split("\n\n").filter(|block| !block.trim().is_empty());
    if !srt {
        let header = blocks.next().ok_or_else(|| invalid("empty WebVTT file"))?;
        let first = header.lines().next().unwrap_or("");
        if !(first == "WEBVTT" || first.starts_with("WEBVTT ") || first.starts_with("WEBVTT\t"))
            || header.contains("-->")
        {
            return Err(invalid(
                "WebVTT requires a WEBVTT header followed by a blank line",
            ));
        }
    }
    let mut cues = 0;
    let mut previous = 0;
    for block in blocks {
        let mut lines = block.trim_matches('\n').lines();
        let first = lines.next().unwrap_or("");
        if !srt
            && (first == "NOTE"
                || first.starts_with("NOTE ")
                || first.starts_with("NOTE\t")
                || first == "STYLE"
                || first == "REGION")
        {
            continue;
        }
        let timing = if first.contains("-->") {
            first
        } else {
            lines
                .next()
                .ok_or_else(|| invalid("cue is missing timestamps"))?
        };
        let (start, end_and_settings) = timing
            .split_once("-->")
            .ok_or_else(|| invalid("cue is missing '-->'"))?;
        let end = end_and_settings
            .split_whitespace()
            .next()
            .ok_or_else(|| invalid("cue has no end timestamp"))?;
        let start = timestamp(start.trim(), srt)
            .ok_or_else(|| invalid(format!("invalid start timestamp: {start}")))?;
        let end =
            timestamp(end, srt).ok_or_else(|| invalid(format!("invalid end timestamp: {end}")))?;
        if start < previous || end <= start {
            return Err(invalid("cue times must be ordered, with end after start"));
        }
        if !lines.any(|line| !line.trim().is_empty()) {
            return Err(invalid("cue has no text"));
        }
        previous = start;
        cues += 1;
    }
    if cues == 0 {
        return Err(invalid("file contains no timed subtitle cues"));
    }
    Ok(())
}

fn timestamp(text: &str, srt: bool) -> Option<u64> {
    let (clock, millis) = text.split_once(if srt { ',' } else { '.' })?;
    if millis.len() != 3 || !millis.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let fields: Vec<_> = clock.split(':').collect();
    if fields.len() != 3 && (srt || fields.len() != 2) {
        return None;
    }
    if fields
        .iter()
        .any(|s| s.len() < 2 || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let length = fields.len();
    if fields[length - 1].len() != 2 || fields[length - 2].len() != 2 {
        return None;
    }
    let hours = if length == 3 {
        fields[0].parse::<u64>().ok()?
    } else {
        0
    };
    let minutes = fields[length - 2].parse::<u64>().ok()?;
    let seconds = fields[length - 1].parse::<u64>().ok()?;
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    hours
        .checked_mul(3600)?
        .checked_add(minutes * 60 + seconds)?
        .checked_mul(1000)?
        .checked_add(millis.parse().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(index: u32, language: &str, title: &str, codec: &str) -> StreamInfo {
        StreamInfo {
            index,
            kind: "subtitle".into(),
            codec: Some(codec.into()),
            language: Some(language.into()),
            title: Some(title.into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn language_ranking_explicit_overrides_and_sidecar_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("movie ; unicode ü.mkv");
        std::fs::write(&video, "video").unwrap();
        let mut info = MediaInfo {
            path: video.clone(),
            container: "matroska".into(),
            duration_seconds: Some(90.0),
            streams: vec![
                stream(2, "eng", "Forced", "subrip"),
                stream(3, "eng", "English SDH", "subrip"),
                stream(4, "eng", "English", "subrip"),
                stream(5, "dut", "Nederlands", "subrip"),
                stream(6, "uk", "English", "subrip"),
                stream(7, "eng", "English", "hdmv_pgs_subtitle"),
            ],
        };
        info.streams[0].default = true;
        let mut prefs = SubtitlePreferences::default();
        assert_eq!(
            preferred_embedded(&info, &prefs.languages).unwrap().index,
            4
        );
        prefs.languages = vec!["nl".into(), "en".into()];
        assert_eq!(
            preferred_embedded(&info, &prefs.languages).unwrap().index,
            5
        );
        assert_eq!(
            select(&Request::Embedded(7), &info, &video, &prefs)
                .await
                .unwrap(),
            Some(Selection::Bitmap { index: 7 })
        );
        assert!(
            select(&Request::Embedded(90), &info, &video, &prefs)
                .await
                .is_err()
        );
        prefs.auto_load = false;
        assert_eq!(
            select(&Request::Auto, &info, &video, &prefs).await.unwrap(),
            None
        );
        assert!(
            select(&Request::Embedded(4), &info, &video, &prefs)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            select(&Request::Off, &info, &video, &prefs).await.unwrap(),
            None
        );
        prefs.auto_load = true;
        info.streams = vec![stream(2, "uk", "English", "subrip")];
        assert!(
            preferred_embedded(&info, &prefs.languages).is_none(),
            "uk is Ukrainian"
        );
        info.streams[0].language = Some("und".into());
        assert_eq!(
            preferred_embedded(&info, &prefs.languages).unwrap().index,
            2
        );
        info.streams.clear();
        let unrelated = dir.path().join("another.srt");
        std::fs::write(&unrelated, "unrelated").unwrap();
        assert_eq!(
            select(&Request::Auto, &info, &video, &prefs).await.unwrap(),
            None
        );
        let matching = video.with_extension("SRT");
        std::fs::write(&matching, "captions").unwrap();
        assert_eq!(
            select(&Request::Auto, &info, &video, &prefs).await.unwrap(),
            Some(Selection::External(matching))
        );
        std::fs::write(video.with_extension("srt"), "ambiguous").unwrap();
        assert!(select(&Request::Auto, &info, &video, &prefs).await.is_err());
        assert_eq!(
            select(&Request::External(unrelated.clone()), &info, &video, &prefs)
                .await
                .unwrap(),
            Some(Selection::External(unrelated))
        );
    }

    #[tokio::test]
    async fn bom_marked_utf16_webvtt_is_normalized_without_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("captions.vtt");
        let text = "WEBVTT\n\n00:00.000 --> 00:01.000\nHallo ë!\n";
        for little in [true, false] {
            let mut bytes = if little {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for unit in text.encode_utf16() {
                bytes.extend(if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            std::fs::write(&path, bytes).unwrap();
            let result = prepare(&path, Path::new("missing"), &CancellationToken::new())
                .await
                .unwrap();
            assert!(
                std::fs::read_to_string(&result.path)
                    .unwrap()
                    .contains("Hallo ë!")
            );
            result.close().unwrap();
        }
    }
    #[test]
    fn validates_cues_and_rejects_silent_or_broken_subtitles() {
        assert!(
            validate(
                "WEBVTT\n\ncue-id\n00:00.000 --> 00:01.000 align:start\nHello\n",
                false
            )
            .is_ok()
        );
        assert!(validate("1\n00:00:00,000 --> 00:00:01,000\nHello\n", true).is_ok());
        for text in [
            "WEBVTT\n",
            "WEBVTT\n00:00.000 --> 00:01.000\nHello",
            "WEBVTT\n\n00:01.000 --> 00:00.000\nHello",
            "WEBVTT\n\n00:00.000 --> 00:01.000\n",
            "WEBVTT\n\n00:99.000 --> 01:00.000\nHello",
        ] {
            assert!(validate(text, false).is_err(), "{text}");
        }
    }
    #[tokio::test]
    async fn webvtt_needs_no_ffmpeg_and_owns_a_cleanable_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("captions.vtt");
        tokio::fs::write(
            &original,
            "\u{feff}WEBVTT\r\n\r\n00:00.000 --> 00:01.000\r\nHello\r\n",
        )
        .await
        .unwrap();
        let prepared = prepare(
            &original,
            Path::new("/no/such/ffmpeg"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let artifact = prepared.path.clone();
        assert!(artifact.exists());
        prepared.close().unwrap();
        assert!(!artifact.exists());
        assert!(original.exists());
    }
}
