//! Bundled model compatibility and the evidence behind each playback policy.
//! This is local knowledge, not receiver capability negotiation.
use std::{collections::HashSet, sync::OnceLock};

use serde::Deserialize;

use crate::{config::CompatibilityPreferences, media::DirectPlayPolicy};

const BUNDLED: &str = include_str!("../data/devices.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
    pub schema_version: u32,
    pub devices: Vec<Device>,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybackPolicy {
    pub allow_hevc: bool,
    pub allow_aac_surround: bool,
    pub h264_max_level: u8,
    pub h264_max_fps: u16,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    Intermittent,
    Untested,
}

#[derive(Debug, Deserialize)]
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
            CompatibilityPreferences {
                allow_hevc: self.playback.allow_hevc,
                allow_aac_surround: self.playback.allow_aac_surround,
                ..Default::default()
            }
            .relax(DirectPlayPolicy::Conservative)
        }
    }
}

impl Database {
    fn parse(text: &str) -> Result<Self, String> {
        let database: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if database.schema_version != 1 {
            return Err("unsupported device database schema".into());
        }
        let mut names = HashSet::new();
        for device in &database.devices {
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
            if device.observations.is_empty() {
                return Err("device requires evidence".into());
            }
        }
        Ok(database)
    }

    pub fn find(&self, model: &str) -> Option<&Device> {
        let model = model.trim();
        self.devices.iter().find(|device| {
            device.id.eq_ignore_ascii_case(model)
                || device
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(model))
        })
    }
}

pub fn database() -> &'static Database {
    static DATABASE: OnceLock<Database> = OnceLock::new();
    DATABASE.get_or_init(|| Database::parse(BUNDLED).expect("invalid bundled device database"))
}

pub fn policy(model: Option<&str>) -> DirectPlayPolicy {
    model
        .and_then(|model| database().find(model))
        .map_or(DirectPlayPolicy::Conservative, Device::policy)
}

#[cfg(test)]
mod tests {
    use super::*;

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
