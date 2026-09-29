#![cfg(unix)]
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    time::Duration,
};
use werp_core::{
    CancellationToken, WerpError,
    cache::{self, CacheOptions, Event},
    media::{self, ProbeOptions},
    transcode::{TranscodeMode, TranscodeOptions},
};

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

struct Fixture {
    dir: tempfile::TempDir,
    source: std::path::PathBuf,
    ffmpeg: std::path::PathBuf,
    probe: ProbeOptions,
    options: TranscodeOptions,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("source");
        fs::create_dir(&source_dir).unwrap();
        let source = source_dir.join("movie.mkv");
        fs::write(&source, b"original").unwrap();
        let executable = dir.path().join("probe");
        script(&executable, "cat \"$0.json\"");
        fs::write(
            dir.path().join("probe.json"),
            include_str!("fixtures/h264.json"),
        )
        .unwrap();
        let ffmpeg = dir.path().join("ffmpeg");
        script(
            &ffmpeg,
            r#"
case "$*" in *-encoders*) printf ' V..... libx264 encoder\n A..... aac encoder\n'; exit 0;; esac
for last do :; done
printf 'complete prepared file' > "$last"
"#,
        );
        Self {
            dir,
            source,
            ffmpeg,
            probe: ProbeOptions {
                executable,
                ..Default::default()
            },
            options: TranscodeOptions {
                mode: TranscodeMode::Remux,
                timeout: Duration::from_secs(2),
                ..Default::default()
            },
        }
    }
    async fn prepare(
        &self,
        cache: &CacheOptions,
        cancel: &CancellationToken,
        event: impl FnMut(Event),
    ) -> Result<werp_core::transcode::PreparedMedia, WerpError> {
        let source = media::inspect(&self.source, &self.probe, cancel).await?;
        cache::prepare(
            &source,
            &self.ffmpeg,
            &self.probe,
            &self.options,
            cache,
            cancel,
            event,
        )
        .await
    }
}

#[tokio::test]
async fn bitmap_delay_has_its_own_cache_recipe() {
    let mut fixture = Fixture::new();
    let cancel = CancellationToken::new();
    let cache = CacheOptions::default();
    let mut info = media::inspect(&fixture.source, &fixture.probe, &cancel)
        .await
        .unwrap();
    info.streams.push(media::StreamInfo {
        index: 2,
        kind: "subtitle".into(),
        codec: Some("hdmv_pgs_subtitle".into()),
        ..Default::default()
    });
    fixture.options.mode = TranscodeMode::AudioVideo;
    fixture.options.bitmap_subtitle = Some(2);
    let mut paths = Vec::new();
    for delay in [0, 1500, -1500, 1500] {
        fixture.options.subtitle_delay_ms = delay;
        let mut reused = false;
        let output = cache::prepare(
            &info,
            &fixture.ffmpeg,
            &fixture.probe,
            &fixture.options,
            &cache,
            &cancel,
            |event| reused |= matches!(event, Event::Reused(_)),
        )
        .await
        .unwrap();
        assert_eq!(reused, paths.len() == 3);
        paths.push(output.info.path.clone());
        output.close().unwrap();
    }
    assert_ne!(paths[0], paths[1]);
    assert_ne!(paths[1], paths[2]);
    assert_ne!(paths[0], paths[2]);
    assert_eq!(paths[1], paths[3]);
}

#[tokio::test]
async fn completed_output_is_retained_reused_and_invalidated_by_source_and_recipe() {
    let mut fixture = Fixture::new();
    let cancel = CancellationToken::new();
    let cache = CacheOptions::default();
    let output = fixture.prepare(&cache, &cancel, |_| {}).await.unwrap();
    let first = output.info.path.clone();
    output.close().unwrap();
    assert_eq!(first.parent(), fixture.source.parent());
    assert!(first.exists());
    // A valid hit does not even need an installed FFmpeg executable.
    fs::rename(&fixture.ffmpeg, fixture.dir.path().join("saved-ffmpeg")).unwrap();
    let mut reused = false;
    // Text subtitle timing does not change a prepared video's cache recipe.
    fixture.options.subtitle_delay_ms = 1500;
    let hit = fixture
        .prepare(&cache, &cancel, |e| reused |= matches!(e, Event::Reused(_)))
        .await
        .unwrap();
    assert!(reused);
    assert_eq!(hit.info.path, first);
    hit.close().unwrap();
    fs::rename(fixture.dir.path().join("saved-ffmpeg"), &fixture.ffmpeg).unwrap();
    // Same-length change still invalidates the full source fingerprint.
    fs::write(&fixture.source, b"modified").unwrap();
    let next = fixture.prepare(&cache, &cancel, |_| {}).await.unwrap();
    assert_ne!(next.info.path, first);
    let second = next.info.path.clone();
    next.close().unwrap();
    fixture.options.mode = TranscodeMode::AudioOnly;
    let changed_recipe = fixture.prepare(&cache, &cancel, |_| {}).await.unwrap();
    assert_ne!(changed_recipe.info.path, second);
    changed_recipe.close().unwrap();
    assert!(
        first.exists(),
        "never evict potentially in-use results implicitly"
    );
}

#[tokio::test]
async fn corrupt_outputs_incomplete_sidecars_and_foreign_files_are_not_reused_or_overwritten() {
    let fixture = Fixture::new();
    let cancel = CancellationToken::new();
    let cache = CacheOptions::default();
    let first = fixture.prepare(&cache, &cancel, |_| {}).await.unwrap();
    let path = first.info.path.clone();
    first.close().unwrap();
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, vec![b'x'; bytes.len()]).unwrap();
    let fresh = fixture.prepare(&cache, &cancel, |_| {}).await.unwrap();
    assert_ne!(fresh.info.path, path);
    assert_eq!(fs::read(&path).unwrap(), vec![b'x'; bytes.len()]);
    let metadata = fresh.info.path.with_extension("mp4.json");
    let safe = fixture.dir.path().join("unrelated.txt");
    fs::write(&safe, "leave alone").unwrap();
    fs::write(&metadata, "invalid metadata").unwrap();
    fresh.close().unwrap();
    let next = fixture.prepare(&cache, &cancel, |_| {}).await.unwrap();
    assert_ne!(next.info.path.with_extension("mp4.json"), metadata);
    next.close().unwrap();
    assert_eq!(fs::read(&safe).unwrap(), b"leave alone");
    assert_eq!(fs::read(&fixture.source).unwrap(), b"original");
}

#[tokio::test]
async fn symlinks_store_beside_real_source_and_no_cache_preserves_temporary_lifecycle() {
    let mut fixture = Fixture::new();
    let cancel = CancellationToken::new();
    let original = fixture.source.clone();
    let link = fixture.dir.path().join("alias.mkv");
    symlink(&original, &link).unwrap();
    fixture.source = link;
    let output = fixture
        .prepare(&CacheOptions::default(), &cancel, |_| {})
        .await
        .unwrap();
    assert_eq!(output.info.path.parent(), original.parent());
    output.close().unwrap();
    let folder = fixture.dir.path().join("temporary");
    let options = CacheOptions {
        enabled: false,
        directory: Some(folder.clone()),
    };
    let output = fixture.prepare(&options, &cancel, |_| {}).await.unwrap();
    let path = output.info.path.clone();
    assert!(path.exists());
    output.close().unwrap();
    assert!(!path.exists());
    assert_eq!(fs::read_dir(folder).unwrap().count(), 0);
}

#[tokio::test]
async fn failed_or_cancelled_preparation_never_publishes_a_reusable_file() {
    let fixture = Fixture::new();
    script(
        &fixture.ffmpeg,
        "for last do :; done\nprintf partial > \"$last\"\nexit 7",
    );
    let failed = fixture
        .prepare(&CacheOptions::default(), &CancellationToken::new(), |_| {})
        .await;
    assert!(failed.is_err());
    assert_eq!(
        fs::read_dir(fixture.source.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        fixture
            .prepare(&CacheOptions::default(), &cancel, |_| {})
            .await,
        Err(WerpError::Cancelled)
    ));
}

#[tokio::test]
async fn concurrent_producers_do_not_overwrite_each_other() {
    let fixture = Fixture::new();
    let cancel = CancellationToken::new();
    let cache = CacheOptions::default();
    let (left, right) = tokio::join!(
        fixture.prepare(&cache, &cancel, |_| {}),
        fixture.prepare(&cache, &cancel, |_| {})
    );
    for prepared in [left.unwrap(), right.unwrap()] {
        let path = prepared.info.path.clone();
        prepared.close().unwrap();
        assert_eq!(fs::read(path).unwrap(), b"complete prepared file");
    }
    let reused = fixture.prepare(&cache, &cancel, |_| {}).await.unwrap();
    reused.close().unwrap();
}

#[tokio::test]
async fn full_names_short_tags_and_existing_prepared_names_remain_reusable() {
    let mut fixture = Fixture::new();
    let stem = "Sample.Series.S01E01.1080p.10bit.WEBRip.Extended.Release.Name";
    let source = fixture.source.with_file_name(format!("{stem}.mkv"));
    fs::rename(&fixture.source, &source).unwrap();
    fixture.source = source;
    let cancel = CancellationToken::new();
    let output = fixture
        .prepare(&CacheOptions::default(), &cancel, |_| {})
        .await
        .unwrap();
    let path = output.info.path.clone();
    output.close().unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let suffix = name
        .strip_prefix(&format!("{stem}.werp-"))
        .unwrap()
        .strip_suffix(".mp4")
        .unwrap();
    let (tag, generation) = suffix.split_once('-').unwrap();
    assert_eq!(tag.len(), 12);
    assert_eq!(generation.len(), 8);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(path.with_extension("mp4.json")).unwrap()).unwrap();
    let key = manifest["key"].as_str().unwrap();
    assert_eq!(key.len(), 64);
    assert_eq!(manifest["output_sha256"].as_str().unwrap().len(), 64);
    let old_stem: String = fixture
        .source
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .chars()
        .take(40)
        .collect();
    let old = path.with_file_name(format!(
        "{old_stem}.werp-{}-0123456789abcdef.mp4",
        &key[..20]
    ));
    fs::rename(&path, &old).unwrap();
    fs::rename(
        path.with_extension("mp4.json"),
        old.with_extension("mp4.json"),
    )
    .unwrap();
    fs::remove_file(&fixture.ffmpeg).unwrap();
    let reused = fixture
        .prepare(&CacheOptions::default(), &cancel, |_| {})
        .await
        .unwrap();
    assert_eq!(reused.info.path, old);
    reused.close().unwrap();
    assert_eq!(fs::read(&fixture.source).unwrap(), b"original");

    // Exercise the actual filesystem with a long multibyte source name too.
    let mut unicode = Fixture::new();
    let source = unicode
        .source
        .with_file_name(format!("{}.mkv", "界".repeat(80)));
    fs::rename(&unicode.source, &source).unwrap();
    unicode.source = source;
    let prepared = unicode
        .prepare(&CacheOptions::default(), &cancel, |_| {})
        .await
        .unwrap();
    let metadata = prepared.info.path.with_extension("mp4.json");
    assert!(metadata.exists());
    assert!(metadata.file_name().unwrap().as_encoded_bytes().len() <= 255);
    prepared.close().unwrap();
}

#[tokio::test]
async fn matching_short_tag_never_substitutes_for_the_full_cache_key() {
    let fixture = Fixture::new();
    let cancel = CancellationToken::new();
    let options = CacheOptions::default();
    let output = fixture.prepare(&options, &cancel, |_| {}).await.unwrap();
    let path = output.info.path.clone();
    output.close().unwrap();
    let metadata = path.with_extension("mp4.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    let mut other_key = manifest["key"].as_str().unwrap().to_owned();
    let replacement = if &other_key[12..13] == "0" { "1" } else { "0" };
    other_key.replace_range(12..13, replacement);
    manifest["key"] = other_key.into();
    let foreign_metadata = serde_json::to_vec(&manifest).unwrap();
    fs::write(&metadata, &foreign_metadata).unwrap();
    let fresh = fixture.prepare(&options, &cancel, |_| {}).await.unwrap();
    assert_ne!(fresh.info.path, path);
    assert_eq!(fs::read(&metadata).unwrap(), foreign_metadata);
    assert_eq!(fs::read(&path).unwrap(), b"complete prepared file");
    fresh.close().unwrap();
}
