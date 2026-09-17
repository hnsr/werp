//! Validated, session-owned UTF-8 text subtitles. No source file is modified.
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use tempfile::TempDir;
use tokio::{io::AsyncReadExt, process::Command};

use crate::{CancellationToken, ProcastError, process};

const MAX_BYTES: u64 = 4 * 1024 * 1024;

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
        let text = String::from_utf8(bytes).map_err(|_| invalid("subtitles must be UTF-8"))?;
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
    let is_srt = match extension.as_str() {
        "srt" => true,
        "vtt" => false,
        _ => return Err(invalid("expected an external .srt or .vtt file")),
    };
    validate(&text, is_srt)?;
    let directory = tempfile::Builder::new()
        .prefix("procast-subtitles-")
        .tempdir()
        .map_err(|e| invalid(e.to_string()))?;
    let output = if is_srt {
        let input = directory.path().join("input.srt");
        tokio::fs::write(&input, text.as_bytes())
            .await
            .map_err(|e| invalid(e.to_string()))?;
        let mut command = Command::new(ffmpeg);
        command
            .args(["-nostdin", "-v", "error", "-f", "srt", "-i"])
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
