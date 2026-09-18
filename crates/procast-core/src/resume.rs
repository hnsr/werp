//! Per-source playback checkpoints. Only confirmed receiver positions are saved.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, UNIX_EPOCH},
};

use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};

use crate::media::MediaInfo;

#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    position: f64,
    duration: f64,
}

pub(crate) struct ResumeStore {
    path: PathBuf,
    _lock: File,
    duration: f64,
    saved: f64,
    latest: Option<f64>,
    last_write: Instant,
    write_failed: bool,
}

impl Drop for ResumeStore {
    fn drop(&mut self) {
        // Explicit unlock also releases transient fork-inherited descriptors
        // before their close-on-exec, rather than waiting for their last close.
        let _ = self._lock.unlock();
    }
}

fn default_directory() -> std::io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from)
        && path.is_absolute()
    {
        return Ok(path.join("procast/resume"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join(".local/state/procast/resume"))
        .ok_or_else(|| std::io::Error::other("cannot locate the user state directory"))
}

impl ResumeStore {
    pub(crate) fn open(info: &MediaInfo, directory: Option<&Path>) -> std::io::Result<Self> {
        let source = fs::canonicalize(&info.path)?;
        let meta = fs::metadata(&source)?;
        let modified = meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(std::io::Error::other)?;
        let mut digest = Context::new(&SHA256);
        digest.update(source.as_os_str().as_encoded_bytes());
        digest.update(&meta.len().to_le_bytes());
        digest.update(&modified.as_nanos().to_le_bytes());
        let key: String = digest
            .finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let directory = directory
            .map(Path::to_path_buf)
            .map_or_else(default_directory, Ok)?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        let path = directory.join(format!("{key}.json"));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(directory.join(format!("{key}.lock")))?;
        lock.try_lock().map_err(std::io::Error::other)?;
        let duration = info
            .duration_seconds
            .filter(|d| d.is_finite() && *d > 0.0)
            .ok_or_else(|| std::io::Error::other("resume requires a positive duration"))?;
        let saved = match File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(4097).read_to_end(&mut bytes)?;
                match serde_json::from_slice::<Record>(&bytes) {
                    Ok(r)
                        if bytes.len() <= 4096
                            && r.version == 1
                            && r.position.is_finite()
                            && r.position >= 0.0
                            && r.position < duration
                            && (r.duration - duration).abs() < 1.0 =>
                    {
                        r.position
                    }
                    _ => {
                        tracing::warn!("Invalid resume checkpoint ignored");
                        0.0
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0.0,
            Err(e) => return Err(e),
        };
        Ok(Self {
            path,
            _lock: lock,
            duration,
            saved,
            latest: None,
            last_write: Instant::now() - Duration::from_secs(5),
            write_failed: false,
        })
    }

    pub(crate) fn start_position(&self) -> f64 {
        if self.saved >= 10.0 && self.duration - self.saved >= 5.0 {
            self.saved - 5.0
        } else {
            0.0
        }
    }

    pub(crate) fn update(&mut self, position: f64) {
        if position.is_finite() && position >= 0.0 && position < self.duration {
            self.latest = Some(position);
            if self.last_write.elapsed() >= Duration::from_secs(5) {
                self.flush();
            }
        }
    }

    fn flush(&mut self) {
        if self.write_failed {
            return;
        }
        let Some(position) = self.latest else {
            return;
        };
        if position == self.saved {
            return;
        }
        let result = (|| -> std::io::Result<()> {
            let bytes = serde_json::to_vec(&Record {
                version: 1,
                position,
                duration: self.duration,
            })?;
            let mut file = tempfile::NamedTempFile::new_in(self.path.parent().unwrap())?;
            file.write_all(&bytes)?;
            file.as_file().sync_all()?;
            file.persist(&self.path).map_err(|e| e.error)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.write_failed = true;
            tracing::warn!(%error, "Could not save playback position; casting continues");
        } else {
            self.saved = position;
        }
        self.last_write = Instant::now();
    }

    pub(crate) fn finish(mut self, completed: bool) {
        if completed {
            if let Err(error) = fs::remove_file(&self.path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, "Could not clear completed playback position");
            }
        } else {
            self.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_positions_resume_completion_clears_and_identity_changes_invalidate() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("video.mp4");
        fs::write(&source, "source").unwrap();
        let info = MediaInfo {
            path: source.clone(),
            container: "mp4".into(),
            duration_seconds: Some(120.0),
            streams: vec![],
        };
        let state = dir.path().join("state");
        let open = || ResumeStore::open(&info, Some(&state)).unwrap();
        let mut store = open();
        assert_eq!(store.start_position(), 0.0);
        assert!(
            ResumeStore::open(&info, Some(&state)).is_err(),
            "simultaneous sessions must not race the same checkpoint"
        );
        store.update(40.0);
        store.update(f64::NAN);
        store.update(999.0);
        store.finish(false);
        let store = open();
        assert_eq!(store.start_position(), 35.0);
        store.finish(false);
        // Loading fails before any confirmed position: keep the previous checkpoint.
        let mut store = open();
        store.update(20.0);
        store.finish(false);
        let store = open();
        assert_eq!(store.start_position(), 15.0);
        store.finish(true);
        let mut store = open();
        assert_eq!(store.start_position(), 0.0);
        store.update(50.0);
        store.finish(false);
        fs::write(source, "changed source size").unwrap();
        let store = open();
        assert_eq!(store.start_position(), 0.0);
        fs::write(&store.path, "corrupt").unwrap();
        drop(store);
        assert_eq!(open().start_position(), 0.0);
    }
}
