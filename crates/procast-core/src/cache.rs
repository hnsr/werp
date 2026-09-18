//! Validated, persistent prepared media beside the canonical source (or in a cache).
use std::{
    io::Write,
    path::{Path, PathBuf},
};

use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

use crate::{
    CancellationToken, ProcastError,
    media::{self, MediaInfo, ProbeOptions},
    transcode::{self, PreparedMedia, TranscodeOptions, TranscodeProgress},
};

// Bump whenever output-affecting FFmpeg arguments or preparation semantics change.
const RECIPE_VERSION: u32 = 1;

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

fn error(message: impl Into<String>) -> ProcastError {
    ProcastError::Transcode(message.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn fingerprint(path: &Path, cancel: &CancellationToken) -> Result<String, ProcastError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| error(format!("cannot read cache fingerprint: {e}")))?;
    let mut digest = Context::new(&SHA256);
    let mut bytes = vec![0; 1024 * 1024];
    loop {
        let count = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ProcastError::Cancelled),
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
    prefix: &str,
    key: &str,
    probe: &ProbeOptions,
    options: &TranscodeOptions,
    cancel: &CancellationToken,
) -> Result<Option<PreparedMedia>, ProcastError> {
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
            return Err(ProcastError::Cancelled);
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(prefix) || !name.ends_with(".mp4.json") {
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
            Err(ProcastError::Cancelled) => return Err(ProcastError::Cancelled),
            _ => continue,
        }
        let info = match media::inspect(&path, probe, cancel).await {
            Ok(info) => info,
            Err(ProcastError::Cancelled) => return Err(ProcastError::Cancelled),
            Err(ProcastError::MissingExecutable(exe)) => {
                return Err(ProcastError::MissingExecutable(exe));
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
) -> Result<PreparedMedia, ProcastError> {
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
    if let Some(index) = options.bitmap_subtitle {
        recipe.push_str(&format!("-bitmap-{index}"));
    }
    let mut digest = Context::new(&SHA256);
    digest.update(source.as_os_str().as_encoded_bytes());
    digest.update(b"\0");
    digest.update(source_hash.as_bytes());
    digest.update(recipe.as_bytes());
    let key = hex(digest.finish().as_ref());
    let stem: String = source
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .take(40)
        .collect();
    let prefix = format!("{stem}.procast-{}-", &key[..20]);
    let adjacent = source
        .parent()
        .ok_or_else(|| error("source has no parent directory"))?;
    let mut parent = cache.directory.clone().unwrap_or_else(|| adjacent.into());
    // Read valid entries before testing writability: a read-only folder can still
    // contain a perfectly reusable completed output.
    if let Some(prepared) = cached(&parent, &prefix, &key, probe, options, cancel).await? {
        event(Event::Reused(prepared.info.path.clone()));
        return Ok(prepared);
    }
    let writable = || -> std::io::Result<()> {
        std::fs::create_dir_all(&parent)?;
        tempfile::Builder::new()
            .prefix(".procast-write-check-")
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
        if let Some(prepared) = cached(&parent, &prefix, &key, probe, options, cancel).await? {
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
        return Err(ProcastError::Cancelled);
    }
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).map_err(|e| error(e.to_string()))?;
    let path = parent.join(format!("{prefix}{}.mp4", hex(&nonce)));
    let manifest_path = path.with_extension("mp4.json");
    // Hard-link publishing is atomic and never replaces another file. Concurrent
    // producers use different suffixes, so readers never see a partially written MP4.
    tokio::fs::hard_link(&prepared.info.path, &path)
        .await
        .map_err(|e| error(format!("cannot publish prepared file: {e}")))?;
    let publication = (|| -> Result<(), ProcastError> {
        let manifest = Manifest {
            version: RECIPE_VERSION,
            key,
            output_sha256,
            output_size,
        };
        let mut temp =
            tempfile::NamedTempFile::new_in(&parent).map_err(|e| error(e.to_string()))?;
        temp.write_all(&serde_json::to_vec(&manifest)?)
            .map_err(|e| error(e.to_string()))?;
        temp.as_file()
            .sync_all()
            .map_err(|e| error(e.to_string()))?;
        temp.persist_noclobber(&manifest_path)
            .map_err(|e| error(format!("cannot publish cache metadata: {e}")))?;
        Ok(())
    })();
    if let Err(cause) = publication {
        let _ = tokio::fs::remove_file(&path).await; // Only the link created by this call.
        return Err(cause);
    }
    prepared.close()?;
    let output = media::inspect(&path, probe, cancel).await?;
    event(Event::Stored(path));
    Ok(PreparedMedia::retained(output))
}
