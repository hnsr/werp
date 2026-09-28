//! Bundled model compatibility and the evidence behind each playback policy.
//! This is local knowledge, not receiver capability negotiation.
use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde::Deserialize;

use crate::{YeetError, media::DirectPlayPolicy};

const BUNDLED: &str = include_str!("../data/devices.toml");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
    pub schema_version: u32,
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: String,
    pub aliases: Vec<String>,
    pub receiver_app: String,
    pub firmware: String,
    pub scope: String,
    pub limitations: Vec<String>,
    pub playback: PlaybackPolicy,
    pub observations: Vec<Observation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybackPolicy {
    pub allow_hevc: bool,
    pub allow_aac_surround: bool,
    pub h264_max_level: u8,
    pub h264_max_fps: u16,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    Intermittent,
    Untested,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub id: String,
    pub status: Status,
    pub path: String,
    pub format: String,
    pub result: String,
    /// Repository-relative evidence links, with optional heading fragments.
    pub evidence: Vec<String>,
}

impl Device {
    pub fn policy(&self) -> DirectPlayPolicy {
        if self.playback.h264_max_level == 42 {
            DirectPlayPolicy::H264HighFrameRate {
                allow_hevc: self.playback.allow_hevc,
                allow_aac_surround: self.playback.allow_aac_surround,
                max_fps: self.playback.h264_max_fps,
            }
        } else {
            match (self.playback.allow_hevc, self.playback.allow_aac_surround) {
                (false, false) => DirectPlayPolicy::Conservative,
                (true, false) => DirectPlayPolicy::Hevc,
                (false, true) => DirectPlayPolicy::AacSurround,
                (true, true) => DirectPlayPolicy::Extended,
            }
        }
    }
}

impl Database {
    fn parse(text: &str) -> Result<Self, String> {
        let database: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        database.validate(true)?;
        Ok(database)
    }

    fn validate(&self, require_evidence: bool) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported device database schema".into());
        }
        let mut names = HashSet::new();
        for device in &self.devices {
            for name in std::iter::once(&device.id).chain(&device.aliases) {
                if name.trim().is_empty() || !names.insert(name.trim().to_ascii_lowercase()) {
                    return Err("empty or duplicate device identifier/alias".into());
                }
            }
            let policy = &device.playback;
            if !matches!(
                (policy.h264_max_level, policy.h264_max_fps),
                (41, 30) | (42, 30 | 50 | 60)
            ) {
                return Err("unsupported device playback limits".into());
            }
            let mut observations = HashSet::new();
            for observation in &device.observations {
                if observation.id.is_empty()
                    || !observations.insert(&observation.id)
                    || observation.evidence.is_empty()
                    || observation.result.trim().is_empty()
                {
                    return Err("observations require unique IDs, evidence and a result".into());
                }
            }
            if require_evidence && device.observations.is_empty() {
                return Err("device requires evidence".into());
            }
        }
        Ok(())
    }

    /// Apply a complete override file atomically. IDs target canonical models,
    /// never aliases; supplied lists replace existing lists.
    pub fn with_overrides(&self, text: &str) -> Result<Self, String> {
        let overrides: Overrides = toml::from_str(text).map_err(|e| e.to_string())?;
        if overrides.schema_version != 1 {
            return Err("unsupported device override schema".into());
        }
        let mut merged = self.clone();
        let mut ids = HashSet::new();
        for entry in overrides.devices {
            let id = entry.id.trim();
            if id.is_empty() || !ids.insert(id.to_ascii_lowercase()) {
                return Err("empty or duplicate override model ID".into());
            }
            let index = match merged
                .devices
                .iter()
                .position(|d| d.id.trim().eq_ignore_ascii_case(id))
            {
                Some(index) => index,
                None => {
                    merged.devices.push(Device {
                        id: id.into(),
                        aliases: vec![],
                        receiver_app: "User-specified".into(),
                        firmware: "Not recorded".into(),
                        scope: "Local override; not bundled validation evidence".into(),
                        limitations: vec![],
                        observations: vec![],
                        playback: PlaybackPolicy {
                            allow_hevc: false,
                            allow_aac_surround: false,
                            h264_max_level: 41,
                            h264_max_fps: 30,
                        },
                    });
                    merged.devices.len() - 1
                }
            };
            let device = &mut merged.devices[index];
            if let Some(value) = entry.aliases {
                device.aliases = value;
            }
            if let Some(value) = entry.receiver_app {
                device.receiver_app = value;
            }
            if let Some(value) = entry.firmware {
                device.firmware = value;
            }
            if let Some(value) = entry.scope {
                device.scope = value;
            }
            if let Some(value) = entry.limitations {
                device.limitations = value;
            }
            if let Some(value) = entry.observations {
                device.observations = value;
            }
            if let Some(value) = entry.playback.allow_hevc {
                device.playback.allow_hevc = value;
            }
            if let Some(value) = entry.playback.allow_aac_surround {
                device.playback.allow_aac_surround = value;
            }
            if let Some(value) = entry.playback.h264_max_level {
                device.playback.h264_max_level = value;
            }
            if let Some(value) = entry.playback.h264_max_fps {
                device.playback.h264_max_fps = value;
            }
        }
        merged.validate(false)?;
        Ok(merged)
    }

    pub fn policy(&self, model: Option<&str>) -> DirectPlayPolicy {
        model
            .and_then(|model| self.find(model))
            .map_or(DirectPlayPolicy::Conservative, Device::policy)
    }

    pub fn find(&self, model: &str) -> Option<&Device> {
        let model = model.trim();
        self.devices.iter().find(|device| {
            device.id.trim().eq_ignore_ascii_case(model)
                || device
                    .aliases
                    .iter()
                    .any(|alias| alias.trim().eq_ignore_ascii_case(model))
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Overrides {
    schema_version: u32,
    #[serde(default)]
    devices: Vec<DeviceOverride>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceOverride {
    id: String,
    aliases: Option<Vec<String>>,
    receiver_app: Option<String>,
    firmware: Option<String>,
    scope: Option<String>,
    limitations: Option<Vec<String>>,
    observations: Option<Vec<Observation>>,
    #[serde(default)]
    playback: PlaybackOverride,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaybackOverride {
    allow_hevc: Option<bool>,
    allow_aac_surround: Option<bool>,
    h264_max_level: Option<u8>,
    h264_max_fps: Option<u16>,
}

impl Default for Database {
    fn default() -> Self {
        database().clone()
    }
}

pub fn default_path() -> Option<PathBuf> {
    crate::config::default_path().map(|path| path.with_file_name("devices.toml"))
}

/// Snapshot the bundled database plus local overrides. Re-read for each session;
/// missing default files are optional, malformed or unreadable files are errors.
pub fn load(explicit: Option<&Path>) -> Result<Database, YeetError> {
    let default = default_path();
    let Some(path) = explicit.or(default.as_deref()) else {
        return Ok(Database::default());
    };
    let read = || -> Result<Database, String> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && explicit.is_none() => {
                return Ok(Database::default());
            }
            Err(e) => return Err(e.to_string()),
        };
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("expected a regular device override file".into());
        }
        let mut text = String::new();
        file.take(65537)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        if text.len() > 65536 {
            return Err("device overrides exceed 64 KiB".into());
        }
        database().with_overrides(&text)
    };
    read().map_err(|e| YeetError::Config(format!("device database {path:?}: {e}")))
}

pub fn database() -> &'static Database {
    static DATABASE: OnceLock<Database> = OnceLock::new();
    DATABASE.get_or_init(|| Database::parse(BUNDLED).expect("invalid bundled device database"))
}

pub fn policy(model: Option<&str>) -> DirectPlayPolicy {
    database().policy(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_merge_restrict_extend_and_preserve_bundled_evidence() {
        let bundled = database();
        let merged = bundled
            .with_overrides(
                r#"
            schema_version = 1
            [[devices]]
            id = " kpn diw7022 "
            aliases = ["DIW7022", " Alternate Model "]
            [devices.playback]
            allow_hevc = false
            h264_max_fps = 60
            [[devices]]
            id = "New TV"
            aliases = ["New TV Alias"]
            [devices.playback]
            allow_hevc = true
        "#,
            )
            .unwrap();
        let known = merged.find("alternate model").unwrap();
        assert!(!known.policy().allows_hevc());
        assert!(known.policy().allows_aac_surround());
        assert_eq!(known.policy().h264_max_fps(), 60);
        assert_eq!(
            known.observations.len(),
            bundled.find("DIW7022").unwrap().observations.len()
        );
        let added = merged.find(" new tv alias ").unwrap();
        assert_eq!(added.policy(), DirectPlayPolicy::Hevc);
        assert!(added.observations.is_empty());
        assert!(bundled.find("New TV").is_none());
        assert!(bundled.find("DIW7022").unwrap().policy().allows_hevc());
        assert_eq!(bundled.find("DIW7022").unwrap().policy().h264_max_fps(), 50);
        assert_eq!(
            merged.policy(Some("Unknown TV")),
            DirectPlayPolicy::Conservative
        );
        assert_eq!(merged.policy(None), DirectPlayPolicy::Conservative);
        let cleared = merged
            .with_overrides("schema_version=1\n[[devices]]\nid='KPN DIW7022'\naliases=[]")
            .unwrap();
        assert!(cleared.find("DIW7022").is_none());
        assert!(cleared.find("KPN DIW7022").is_some());
        // The example is also a valid, restrictive overlay, not just prose.
        assert!(
            !bundled
                .with_overrides(include_str!("../../../docs/devices.example.toml"))
                .unwrap()
                .policy(Some("DIW7022"))
                .allows_aac_surround()
        );
    }

    #[test]
    fn invalid_overrides_cannot_shadow_models_or_bypass_bounds() {
        for text in [
            "schema_version=2",
            "schema_version=1\nunknown=true",
            "schema_version=1\n[[devices]]\nid=''",
            "schema_version=1\n[[devices]]\nid='KPN DIW7022'\n[[devices]]\nid=' kpn diw7022 '",
            "schema_version=1\n[[devices]]\nid='DIW7022'", // alias is not a canonical ID
            "schema_version=1\n[[devices]]\nid='Other'\naliases=['diw7022']",
            "schema_version=1\n[[devices]]\nid='Other'\naliases=['OTHER']",
            "schema_version=1\n[[devices]]\nid='Other'\naliases=['  ']",
            "schema_version=1\n[[devices]]\nid='Other'\n[devices.playback]\nallow_av1=true",
            "schema_version=1\n[[devices]]\nid='Other'\n[devices.playback]\nallow_hevc='yes'",
            "schema_version=1\n[[devices]]\nid='Other'\n[devices.playback]\nh264_max_fps=60", // level 41 default
            "schema_version=1\n[[devices]]\nid='KPN DIW7022'\n[devices.playback]\nh264_max_fps=120",
            "schema_version=1\n[[devices]]\nid='KPN DIW7022'\n[devices.playback]\nh264_max_level=50",
        ] {
            assert!(database().with_overrides(text).is_err(), "{text}");
        }
    }

    #[test]
    fn file_loading_is_bounded_and_snapshots_do_not_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.toml");
        assert!(load(Some(&path)).is_err());
        assert!(load(Some(dir.path())).is_err());
        std::fs::write(
            &path,
            "schema_version=1\n[[devices]]\nid='KPN DIW7022'\n[devices.playback]\nallow_hevc=false",
        )
        .unwrap();
        let first = load(Some(&path)).unwrap();
        assert!(!first.policy(Some("DIW7022")).allows_hevc());
        std::fs::write(&path, "schema_version=1").unwrap();
        assert!(
            load(Some(&path))
                .unwrap()
                .policy(Some("DIW7022"))
                .allows_hevc()
        );
        assert!(!first.policy(Some("DIW7022")).allows_hevc());
        std::fs::write(&path, "x".repeat(65537)).unwrap();
        let error = load(Some(&path)).unwrap_err().to_string();
        assert!(error.contains("64 KiB") && error.contains("devices.toml"));
        std::fs::write(&path, "invalid TOML").unwrap();
        assert!(
            load(Some(&path))
                .unwrap_err()
                .to_string()
                .contains("devices.toml")
        );
    }

    #[test]
    fn bundled_models_aliases_and_evidence_are_valid() {
        let database = database();
        let device = database.find("KPN DIW7022").unwrap();
        assert!(std::ptr::eq(device, database.find("  diw7022  ").unwrap()));
        assert!(database.find("KPN DIW7022-extra").is_none());
        assert_eq!(policy(None), DirectPlayPolicy::Conservative);
        assert_eq!(policy(Some("Unknown TV")), DirectPlayPolicy::Conservative);
        assert_eq!(device.policy().h264_max_fps(), 50);
        assert!(device.policy().allows_hevc() && device.policy().allows_aac_surround());
        assert_eq!(
            device
                .observations
                .iter()
                .find(|o| o.id == "mp4-h264-ac3")
                .unwrap()
                .status,
            Status::Failed
        );
        assert_eq!(
            device
                .observations
                .iter()
                .find(|o| o.id == "h264-5994-60fps")
                .unwrap()
                .status,
            Status::Untested
        );
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for device in &database.devices {
            for observation in &device.observations {
                for evidence in &observation.evidence {
                    let (path, anchor) = evidence.split_once('#').unwrap_or((evidence, ""));
                    assert!(path.starts_with("docs/") && !path.contains(".."));
                    let text =
                        std::fs::read_to_string(root.join(path)).expect("evidence file exists");
                    if !anchor.is_empty() {
                        assert!(
                            text.lines()
                                .filter(|line| line.starts_with('#'))
                                .any(|line| {
                                    line.trim_start_matches('#')
                                        .trim()
                                        .to_ascii_lowercase()
                                        .chars()
                                        .filter(|c| {
                                            c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')
                                        })
                                        .collect::<String>()
                                        .replace(' ', "-")
                                        == anchor
                                }),
                            "missing heading: {evidence}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_schema_ambiguous_aliases_and_unbounded_policies_are_rejected() {
        for text in [
            BUNDLED.replace("schema_version = 1", "schema_version = 2"),
            BUNDLED.replace("aliases = [\"DIW7022\"]", "aliases = [\"kpn diw7022\"]"),
            BUNDLED.replace("h264_max_fps = 50", "h264_max_fps = 120"),
            BUNDLED.replace("h264_max_level = 42", "h264_max_level = 50"),
        ] {
            assert!(Database::parse(&text).is_err());
        }
    }
}
