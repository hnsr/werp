//! Saved timing per source identity and subtitle content/embedded stream.
use crate::{media::MediaInfo, subtitles::Request};
use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::AsyncReadExt;

const MAX_SUBTITLE_BYTES: u64 = 4 * 1024 * 1024;
const RETENTION_SECONDS: u64 = 180 * 24 * 60 * 60;
#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    delay_ms: i32,
    updated: u64,
}

fn directory(override_path: Option<&Path>) -> io::Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path.to_owned());
    }
    if let Some(path) = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return Ok(path.join("werp/subtitle-offsets"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join(".local/state/werp/subtitle-offsets"))
        .ok_or_else(|| io::Error::other("cannot locate the user state directory"))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn location(
    info: &MediaInfo,
    subtitle: &Request,
    root: Option<&Path>,
) -> io::Result<Option<PathBuf>> {
    if matches!(subtitle, Request::Off) {
        return Ok(None);
    }
    let source = tokio::fs::canonicalize(&info.path).await?;
    let meta = tokio::fs::metadata(&source).await?;
    let mut digest = Context::new(&SHA256);
    digest.update(source.as_os_str().as_encoded_bytes());
    digest.update(&meta.len().to_le_bytes());
    digest.update(
        &meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos()
            .to_le_bytes(),
    );
    match subtitle {
        Request::Embedded(index) => {
            digest.update(b"embedded");
            digest.update(&index.to_le_bytes());
        }
        Request::External(path) => {
            digest.update(b"external");
            let file = tokio::fs::File::open(path).await?;
            if !file.metadata().await?.is_file() {
                return Err(io::Error::other("subtitle is not a regular file"));
            }
            let mut bytes = Vec::new();
            file.take(MAX_SUBTITLE_BYTES + 1)
                .read_to_end(&mut bytes)
                .await?;
            if bytes.len() as u64 > MAX_SUBTITLE_BYTES {
                return Err(io::Error::other("subtitle exceeds 4 MiB"));
            }
            digest.update(&bytes);
        }
        Request::Auto => {
            return Err(io::Error::other(
                "saved offsets require an explicit subtitle choice",
            ));
        }
        Request::Off => unreachable!(),
    }
    let key: String = digest
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Some(directory(root)?.join(format!("{key}.json"))))
}
fn read_record(path: &Path) -> io::Result<Record> {
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(io::Error::other("oversized subtitle offset state"));
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if record.version != 1 {
        return Err(io::Error::other("unsupported subtitle offset state"));
    }
    Ok(record)
}
/// Read-only lookup. Missing or expired records yield zero.
pub async fn load(info: &MediaInfo, subtitle: &Request, root: Option<&Path>) -> io::Result<i32> {
    let Some(path) = location(info, subtitle, root).await? else {
        return Ok(0);
    };
    match read_record(&path) {
        Ok(record) if now().saturating_sub(record.updated) <= RETENTION_SECONDS => {
            Ok(record.delay_ms)
        }
        Ok(_) => Ok(0),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}
/// Atomic save under a nonblocking store lock; busy state yields a warning to the caller. Zero removes the preference; other offsets survive completion.
pub async fn save(
    info: &MediaInfo,
    subtitle: &Request,
    delay_ms: i32,
    root: Option<&Path>,
) -> io::Result<()> {
    let Some(path) = location(info, subtitle, root).await? else {
        return Ok(());
    };
    let directory = path.parent().unwrap();
    if delay_ms == 0 && !directory.exists() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(directory.join("state.lock"))?;
    lock.try_lock().map_err(io::Error::other)?;
    if delay_ms == 0 {
        match fs::remove_file(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        return Ok(());
    }
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(
        &serde_json::to_vec(&Record {
            version: 1,
            delay_ms,
            updated: now(),
        })
        .map_err(io::Error::other)?,
    )?;
    file.as_file().sync_all()?;
    file.persist(&path).map_err(|e| e.error)?;
    // Only recognized state filenames belong to this store. A failed cleanup must
    // not discard the successfully saved preference.
    if let Ok(entries) = fs::read_dir(directory) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.len() != 69
                || !name.ends_with(".json")
                || !name.as_bytes()[..64]
                    .iter()
                    .copied()
                    .all(|b| b.is_ascii_hexdigit())
            {
                continue;
            }
            if let Ok(record) = read_record(&entry.path())
                && now().saturating_sub(record.updated) > RETENTION_SECONDS
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn offsets_follow_subtitle_content_but_are_scoped_to_video_and_stream() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state");
        let video = dir.path().join("video.mp4");
        fs::write(&video, "video").unwrap();
        let mut info = MediaInfo {
            path: video,
            container: "mp4".into(),
            duration_seconds: Some(120.0),
            streams: vec![],
        };
        let captions = dir.path().join("captions.srt");
        fs::write(&captions, "captions").unwrap();
        let subtitle = Request::External(captions.clone());
        assert_eq!(load(&info, &subtitle, Some(&root)).await.unwrap(), 0);
        assert!(!root.exists());
        save(&info, &subtitle, -1500, Some(&root)).await.unwrap();
        let renamed = dir.path().join("renamed.srt");
        fs::rename(&captions, &renamed).unwrap();
        let subtitle = Request::External(renamed.clone());
        assert_eq!(load(&info, &subtitle, Some(&root)).await.unwrap(), -1500);
        let original = info.path.clone();
        info.path = dir.path().join("other.mp4");
        fs::write(&info.path, "video").unwrap();
        assert_eq!(load(&info, &subtitle, Some(&root)).await.unwrap(), 0);
        info.path = original;
        fs::write(&renamed, "edited captions").unwrap();
        assert_eq!(load(&info, &subtitle, Some(&root)).await.unwrap(), 0);
        save(&info, &Request::Embedded(2), 500, Some(&root))
            .await
            .unwrap();
        assert_eq!(
            load(&info, &Request::Embedded(3), Some(&root))
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            load(&info, &Request::Embedded(2), Some(&root))
                .await
                .unwrap(),
            500
        );
        save(&info, &Request::Embedded(2), 0, Some(&root))
            .await
            .unwrap();
        assert_eq!(
            load(&info, &Request::Embedded(2), Some(&root))
                .await
                .unwrap(),
            0
        );
        save(&info, &Request::Off, 500, Some(&root)).await.unwrap();
        fs::write(&info.path, "changed video").unwrap();
        assert_eq!(
            load(&info, &Request::Embedded(2), Some(&root))
                .await
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn state_is_bounded_and_expired_records_are_pruned_without_removing_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("video");
        fs::write(&video, "video").unwrap();
        let info = MediaInfo {
            path: video,
            container: "mp4".into(),
            duration_seconds: None,
            streams: vec![],
        };
        let root = dir.path().join("state");
        fs::create_dir(&root).unwrap();
        let path = location(&info, &Request::Embedded(2), Some(&root))
            .await
            .unwrap()
            .unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&Record {
                version: 1,
                delay_ms: 500,
                updated: now() - RETENTION_SECONDS - 1,
            })
            .unwrap(),
        )
        .unwrap();
        fs::write(root.join("unrelated.json"), "keep").unwrap();
        assert_eq!(
            load(&info, &Request::Embedded(2), Some(&root))
                .await
                .unwrap(),
            0
        );
        save(&info, &Request::Embedded(3), 700, Some(&root))
            .await
            .unwrap();
        assert!(!path.exists());
        assert!(root.join("unrelated.json").exists());
        fs::write(&path, "broken").unwrap();
        assert!(
            load(&info, &Request::Embedded(2), Some(&root))
                .await
                .is_err()
        );
        fs::write(&path, vec![b' '; 4097]).unwrap();
        assert!(
            load(&info, &Request::Embedded(2), Some(&root))
                .await
                .is_err()
        );
    }
}
