//! Shared compatibility preferences and separate frontend policies.
use std::{
    io::Read,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::YeetError;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub cli: CliPreferences,
    pub compatibility: CompatibilityPreferences,
}

/// Opt-in additions to the default target, never permission to accept HDR or
/// relax the existing resolution, frame-rate, codec-profile or track limits.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CompatibilityPreferences {
    pub allow_hevc: bool,
    pub allow_aac_surround: bool,
}

impl CompatibilityPreferences {
    pub fn relax(self, base: crate::media::DirectPlayPolicy) -> crate::media::DirectPlayPolicy {
        use crate::media::DirectPlayPolicy as Policy;
        if base == Policy::Experimental {
            return base;
        }
        match (
            self.allow_hevc || base.allows_hevc(),
            self.allow_aac_surround || base.allows_aac_surround(),
        ) {
            (false, false) => Policy::Conservative,
            (true, false) => Policy::Hevc,
            (false, true) => Policy::AacSurround,
            (true, true) => Policy::Extended,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CliPreferences {
    pub subtitles: SubtitlePreferences,
    pub devices: DevicePreferences,
    pub playback: PlaybackPreferences,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SubtitlePreferences {
    pub auto_load: bool,
    pub languages: Vec<String>,
}

impl Default for SubtitlePreferences {
    fn default() -> Self {
        Self {
            auto_load: true,
            languages: vec!["en".into(), "nl".into()],
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DevicePreferences {
    pub preferred: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlaybackPreferences {
    pub auto_resume: bool,
}

impl Default for PlaybackPreferences {
    fn default() -> Self {
        Self { auto_resume: true }
    }
}

pub fn default_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        && path.is_absolute()
    {
        return Some(path.join("yeet/config.toml"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join(".config/yeet/config.toml"))
}

pub fn parse(text: &str) -> Result<Config, YeetError> {
    let mut config: Config = toml::from_str(text).map_err(|e| YeetError::Config(e.to_string()))?;
    for language in &mut config.cli.subtitles.languages {
        let original = language.clone();
        *language = normalize_preference(language)
            .ok_or_else(|| {
                YeetError::Config(format!(
                    "unsupported subtitle preference {original:?}; use English/en or Dutch/nl"
                ))
            })?
            .into();
    }
    config.cli.subtitles.languages.dedup();
    if config
        .cli
        .devices
        .preferred
        .iter()
        .any(|p| p.trim().is_empty())
    {
        return Err(YeetError::Config(
            "preferred device names/IDs cannot be empty".into(),
        ));
    }
    Ok(config)
}

/// Language tags are not country codes: `uk` means Ukrainian, not English.
pub fn normalize_language(value: &str) -> Option<&'static str> {
    let normalized = value.trim().to_lowercase().replace('_', "-");
    match normalized.as_str() {
        "english" => Some("en"),
        "dutch" | "nederlands" | "flemish" | "vlaams" => Some("nl"),
        _ => match normalized.split('-').next()? {
            "en" | "eng" => Some("en"),
            "nl" | "nld" | "dut" => Some("nl"),
            _ => None,
        },
    }
}

/// Country shorthand is accepted in user preferences/titles, never as a media
/// language tag (ISO 639 `uk` denotes Ukrainian).
pub fn normalize_preference(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "uk" | "gb" => Some("en"),
        _ => normalize_language(value),
    }
}

pub fn load(explicit: Option<&Path>) -> Result<Config, YeetError> {
    load_with(explicit, parse)
}

/// Frontends consume only the shared section. Syntactically valid CLI settings
/// are ignored here, including CLI-only validation and preference selection.
pub fn load_compatibility(explicit: Option<&Path>) -> Result<CompatibilityPreferences, YeetError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Shared {
        #[serde(default)]
        compatibility: CompatibilityPreferences,
        #[serde(default, rename = "cli")]
        _cli: Option<serde::de::IgnoredAny>,
    }
    load_with(explicit, |text| {
        toml::from_str::<Shared>(text)
            .map(|config| config.compatibility)
            .map_err(|e| YeetError::Config(e.to_string()))
    })
}

fn load_with<T: Default>(
    explicit: Option<&Path>,
    parse: impl FnOnce(&str) -> Result<T, YeetError>,
) -> Result<T, YeetError> {
    let default = default_path();
    let Some(path) = explicit.or(default.as_deref()) else {
        return Ok(T::default());
    };
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && explicit.is_none() => {
            return Ok(T::default());
        }
        Err(e) => return Err(YeetError::Config(format!("cannot read {path:?}: {e}"))),
    };
    if !file
        .metadata()
        .map_err(|e| YeetError::Config(e.to_string()))?
        .is_file()
    {
        return Err(YeetError::Config("expected a regular config file".into()));
    }
    let mut text = String::new();
    file.take(65537)
        .read_to_string(&mut text)
        .map_err(|e| YeetError::Config(e.to_string()))?;
    if text.len() > 65536 {
        return Err(YeetError::Config("config exceeds 64 KiB".into()));
    }
    parse(&text).map_err(|e| YeetError::Config(format!("{path:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_compatibility_defaults_validation_and_cli_isolation() {
        let defaults = parse("").unwrap().compatibility;
        assert!(!defaults.allow_hevc && !defaults.allow_aac_surround);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let valid = "[compatibility]\nallow_hevc=true\nallow_aac_surround=true\n";
        fs_write(&path, valid);
        let shared = load_compatibility(Some(&path)).unwrap();
        assert!(shared.allow_hevc && shared.allow_aac_surround);
        assert!(load(Some(&path)).unwrap().compatibility.allow_hevc);
        fs_write(
            &path,
            &format!(
                "{valid}[cli.subtitles]\nlanguages=['unavailable language']\nauto_load='not a boolean'\n"
            ),
        );
        assert!(load(Some(&path)).is_err());
        assert!(load_compatibility(Some(&path)).unwrap().allow_hevc);
        for invalid in [
            "allow_hevc='yes'",
            "allow_av1=true",
            "allow_aac_suround=true",
        ] {
            fs_write(&path, &format!("[compatibility]\n{invalid}\n"));
            assert!(load(Some(&path)).is_err());
            assert!(load_compatibility(Some(&path)).is_err());
        }
        fn fs_write(path: &Path, text: &str) {
            std::fs::write(path, text).unwrap();
        }
    }
    #[test]
    fn defaults_aliases_and_invalid_preferences() {
        let config = parse("").unwrap();
        assert!(config.cli.subtitles.auto_load && config.cli.playback.auto_resume);
        assert_eq!(config.cli.subtitles.languages, ["en", "nl"]);
        let config = parse("[cli.subtitles]\nauto_load = false\nlanguages = ['Dutch', 'en-GB', 'eng']\n[cli.devices]\npreferred = ['TV', 'tv-id']\n[cli.playback]\nauto_resume = false").unwrap();
        assert_eq!(config.cli.subtitles.languages, ["nl", "en"]);
        assert!(!config.cli.subtitles.auto_load && !config.cli.playback.auto_resume);
        for invalid in [
            "[cli.subtitles]\nlanguages=['German']",
            "[cli.devices]\npreferred=['']",
            "[cli.playback]\nresuem=true",
            "[cli.subtitles]\nauto_load='yes'",
        ] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(load(Some(&dir.path().join("missing.toml"))).is_err());
    }
}
