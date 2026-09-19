//! Configuration for frontend policies. The backend helper never loads CLI settings.
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
    let default = default_path();
    let Some(path) = explicit.or(default.as_deref()) else {
        return Ok(Config::default());
    };
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && explicit.is_none() => {
            return Ok(Config::default());
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
