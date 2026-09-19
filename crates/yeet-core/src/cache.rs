//! Validated, persistent prepared media beside the canonical source (or in a cache).
use std::{
    io::Write,
    path::{Path, PathBuf},
};

use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

use crate::{
    CancellationToken, YeetError,
    media::{self, MediaInfo, ProbeOptions},
    transcode::{self, PreparedMedia, TranscodeOptions, TranscodeProgress},
};

// Base recipe version. Output changes must bump this or a versioned component
// below (the audio matrix has its own version to preserve unaffected remux hits).
const RECIPE_VERSION: u32 = 1;
// Filename tags are lookup hints; manifests retain the full SHA-256 values.
const KEY_TAG_LEN: usize = 12;
const GENERATION_BYTES: usize = 4;
const MAX_NAME_BYTES: usize = 255;
const PUBLICATION_ATTEMPTS: usize = 32;

#[derive(Debug, Clone)]
pub struct CacheOptions {
    pub enabled: bool,
    /// Override both adjacent storage and the fallback user cache.
    pub directory: Option<PathBuf>,
}

impl Default for CacheOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            directory: None,
        }
    }
}

pub enum Event {
    Checking,
    Reused(PathBuf),
    Stored(PathBuf),
    Fallback(PathBuf),
    Progress(TranscodeProgress),
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    key: String,
    output_sha256: String,
    output_size: u64,
}

fn error(message: impl Into<String>) -> YeetError {
    YeetError::Transcode(message.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn filename_prefix(source: &Path, key: &str) -> String {
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    // Reserve room for the longer metadata filename, including both extensions.
    let suffix_bytes = ".yeet-".len() + KEY_TAG_LEN + 1 + GENERATION_BYTES * 2 + ".mp4.json".len();
    let mut end = stem.len().min(MAX_NAME_BYTES - suffix_bytes);
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}.yeet-{}-", &stem[..end], &key[..KEY_TAG_LEN])
}

async fn publish(
    parent: &Path,
    prefix: &str,
    source: &Path,
    metadata: &[u8],
    cancel: &CancellationToken,
    mut generation: impl FnMut() -> Result<String, YeetError>,
) -> Result<PathBuf, YeetError> {
    for _ in 0..PUBLICATION_ATTEMPTS {
        if cancel.is_cancelled() {
            return Err(YeetError::Cancelled);
        }
        let path = parent.join(format!("{prefix}{}.mp4", generation()?));
        // An atomic no-replace link handles competing producers and random-ID
        // collisions without overwriting anything or repeating the conversion.
        match tokio::fs::hard_link(source, &path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(error(format!("cannot publish prepared file: {e}"))),
        }
        let publication = (|| -> std::io::Result<()> {
            let mut temp = tempfile::NamedTempFile::new_in(parent)?;
            temp.write_all(metadata)?;
            temp.as_file().sync_all()?;
            temp.persist_noclobber(path.with_extension("mp4.json"))
                .map_err(|e| e.error)?;
            Ok(())
        })();
        match publication {
            Ok(()) => return Ok(path),
            Err(cause) => {
                // Only remove the media link successfully created by this call.
                tokio::fs::remove_file(&path)
                    .await
                    .map_err(|e| error(format!("cannot remove unpublished prepared file: {e}")))?;
                if cause.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(error(format!("cannot publish cache metadata: {cause}")));
                }
            }
        }
    }
    Err(error(
        "could not allocate an unused prepared-media filename",
    ))
}

async fn fingerprint(path: &Path, cancel: &CancellationToken) -> Result<String, YeetError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| error(format!("cannot read cache fingerprint: {e}")))?;
    let mut digest = Context::new(&SHA256);
    let mut bytes = vec![0; 1024 * 1024];
    loop {
        let count = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(YeetError::Cancelled),
            read = file.read(&mut bytes) => read.map_err(|e| error(format!("cannot fingerprint media: {e}")))?,
        };
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(hex(digest.finish().as_ref()))
}

async fn cached(
    parent: &Path,
    key: &str,
    probe: &ProbeOptions,
    options: &TranscodeOptions,
    cancel: &CancellationToken,
) -> Result<Option<PreparedMedia>, YeetError> {
    let mut entries = match tokio::fs::read_dir(parent).await {
        Ok(entries) => entries,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(error(format!("cannot inspect prepared-media cache: {e}"))),
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| error(format!("cannot read cache entry: {e}")))?
    {
        if cancel.is_cancelled() {
            return Err(YeetError::Cancelled);
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        // The readable stem is not an identity check. This also permits reuse
        // of existing entries with previously longer tags or truncated names.
        let matches_tag = name
            .rsplit_once(".yeet-")
            .is_some_and(|(_, suffix)| suffix.starts_with(&key[..KEY_TAG_LEN]));
        if !matches_tag || !name.ends_with(".mp4.json") {
            continue;
        }
        let meta = tokio::fs::symlink_metadata(entry.path()).await;
        if !meta.is_ok_and(|m| m.is_file() && m.len() <= 4096) {
            continue;
        }
        let Ok(bytes) = tokio::fs::read(entry.path()).await else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes) else {
            continue;
        };
        if manifest.version != RECIPE_VERSION || manifest.key != key {
            continue;
        }
        let path = parent.join(name.strip_suffix(".json").unwrap());
        let meta = tokio::fs::symlink_metadata(&path).await;
        if !meta.is_ok_and(|m| m.is_file() && m.len() == manifest.output_size) {
            continue;
        }
        match fingerprint(&path, cancel).await {
            Ok(hash) if hash == manifest.output_sha256 => {}
            Err(YeetError::Cancelled) => return Err(YeetError::Cancelled),
            _ => continue,
        }
        let info = match media::inspect(&path, probe, cancel).await {
            Ok(info) => info,
            Err(YeetError::Cancelled) => return Err(YeetError::Cancelled),
            Err(YeetError::MissingExecutable(exe)) => {
                return Err(YeetError::MissingExecutable(exe));
            }
            Err(_) => continue,
        };
        if media::assess_direct_play(&info, options.playback_policy).is_ok() {
            return Ok(Some(PreparedMedia::retained(info)));
        }
    }
    Ok(None)
}

pub async fn prepare(
    info: &MediaInfo,
    ffmpeg: &Path,
    probe: &ProbeOptions,
    options: &TranscodeOptions,
    cache: &CacheOptions,
    cancel: &CancellationToken,
    mut event: impl FnMut(Event),
) -> Result<PreparedMedia, YeetError> {
    if !cache.enabled {
        let mut options = options.clone();
        options.directory = cache.directory.clone().or(options.directory);
        return transcode::prepare(info, ffmpeg, probe, &options, cancel, |p| {
            event(Event::Progress(p))
        })
        .await;
    }
    event(Event::Checking);
    let source = tokio::fs::canonicalize(&info.path)
        .await
        .map_err(|e| error(format!("cannot resolve source for cache: {e}")))?;
    let source_hash = fingerprint(&source, cancel).await?;
    let mut recipe = format!(
        "v{RECIPE_VERSION}-{:?}-{:?}",
        options.mode, options.playback_policy
    );
    // Keep remux and silent-video recipes reusable. Audio encodes now use an
    // explicit normalized stereo matrix; old encoded audio must not masquerade
    // as output produced by the new recipe.
    if options.mode != transcode::TranscodeMode::Remux
        && info.streams.iter().any(|s| s.kind == "audio")
    {
        recipe.push_str("-stereo-matrix-v1");
    }
    if let Some(index) = options.bitmap_subtitle {
        recipe.push_str(&format!("-bitmap-{index}"));
    }
    let mut digest = Context::new(&SHA256);
    digest.update(source.as_os_str().as_encoded_bytes());
    digest.update(b"\0");
    digest.update(source_hash.as_bytes());
    digest.update(recipe.as_bytes());
    let key = hex(digest.finish().as_ref());
    let prefix = filename_prefix(&source, &key);
    let adjacent = source
        .parent()
        .ok_or_else(|| error("source has no parent directory"))?;
    let mut parent = cache.directory.clone().unwrap_or_else(|| adjacent.into());
    // Read valid entries before testing writability: a read-only folder can still
    // contain a perfectly reusable completed output.
    if let Some(prepared) = cached(&parent, &key, probe, options, cancel).await? {
        event(Event::Reused(prepared.info.path.clone()));
        return Ok(prepared);
    }
    let writable = || -> std::io::Result<()> {
        std::fs::create_dir_all(&parent)?;
        tempfile::Builder::new()
            .prefix(".yeet-write-check-")
            .tempdir_in(&parent)?
            .close()
    };
    if let Err(e) = writable() {
        if cache.directory.is_some() {
            return Err(error(format!(
                "chosen cache directory is not writable: {e}"
            )));
        }
        parent = transcode::cache_directory()?;
        event(Event::Fallback(parent.clone()));
        if let Some(prepared) = cached(&parent, &key, probe, options, cancel).await? {
            event(Event::Reused(prepared.info.path.clone()));
            return Ok(prepared);
        }
    }
    let mut options = options.clone();
    options.directory = Some(parent.clone());
    let prepared = transcode::prepare(info, ffmpeg, probe, &options, cancel, |p| {
        event(Event::Progress(p))
    })
    .await?;
    // Never publish an output against a source that changed during preparation.
    if fingerprint(&source, cancel).await? != source_hash {
        return Err(error(
            "source changed during preparation; completed output was not cached",
        ));
    }
    let output_sha256 = fingerprint(&prepared.info.path, cancel).await?;
    let output_size = tokio::fs::metadata(&prepared.info.path)
        .await
        .map_err(|e| error(e.to_string()))?
        .len();
    if cancel.is_cancelled() {
        return Err(YeetError::Cancelled);
    }
    let manifest = Manifest {
        version: RECIPE_VERSION,
        key,
        output_sha256,
        output_size,
    };
    let path = publish(
        &parent,
        &prefix,
        &prepared.info.path,
        &serde_json::to_vec(&manifest)?,
        cancel,
        || {
            let mut nonce = [0; GENERATION_BYTES];
            getrandom::fill(&mut nonce).map_err(|e| error(e.to_string()))?;
            Ok(hex(&nonce))
        },
    )
    .await?;
    prepared.close()?;
    let output = media::inspect(&path, probe, cancel).await?;
    event(Event::Stored(path));
    Ok(PreparedMedia::retained(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_names_are_preserved_with_a_utf8_safe_metadata_budget() {
        let key = "a".repeat(64);
        let stem = "Sample.Series.S01E01.1080p.10bit.WEBRip.Extended.Release.Name";
        assert_eq!(
            filename_prefix(Path::new(&format!("{stem}.mkv")), &key),
            format!("{stem}.yeet-aaaaaaaaaaaa-")
        );
        for stem in ["a".repeat(251), "界".repeat(80), "é".repeat(120)] {
            let prefix = filename_prefix(Path::new(&format!("{stem}.mkv")), &key);
            let readable = prefix.split(".yeet-").next().unwrap();
            assert!(stem.starts_with(readable));
            assert!(readable.len() > 200);
            assert!(format!("{prefix}01234567.mp4.json").len() <= MAX_NAME_BYTES);
        }
    }

    #[tokio::test]
    async fn publication_retries_colliding_media_and_orphan_metadata_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("prepared.mp4");
        std::fs::write(&source, b"new output").unwrap();
        let prefix = "movie.yeet-0123456789ab-";
        let media = dir.path().join(format!("{prefix}00000001.mp4"));
        let orphan = dir.path().join(format!("{prefix}00000002.mp4.json"));
        std::fs::write(&media, b"existing media").unwrap();
        std::fs::write(&orphan, b"existing metadata").unwrap();
        let mut next = 0;
        let path = publish(
            dir.path(),
            prefix,
            &source,
            b"new metadata",
            &CancellationToken::new(),
            || {
                next += 1;
                Ok(format!("{next:08x}"))
            },
        )
        .await
        .unwrap();
        assert_eq!(next, 3);
        assert_eq!(std::fs::read(&path).unwrap(), b"new output");
        assert_eq!(
            std::fs::read(path.with_extension("mp4.json")).unwrap(),
            b"new metadata"
        );
        assert_eq!(std::fs::read(&media).unwrap(), b"existing media");
        assert_eq!(std::fs::read(&orphan).unwrap(), b"existing metadata");
        assert!(!dir.path().join(format!("{prefix}00000002.mp4")).exists());
        let mut attempts = 0;
        let failed = publish(
            dir.path(),
            prefix,
            &source,
            b"unused",
            &CancellationToken::new(),
            || {
                attempts += 1;
                Ok("00000001".into())
            },
        )
        .await;
        assert!(failed.is_err());
        assert_eq!(attempts, PUBLICATION_ATTEMPTS);
        assert_eq!(std::fs::read(&media).unwrap(), b"existing media");
    }
}
