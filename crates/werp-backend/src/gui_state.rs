//! Machine-written GUI state, separate from user-edited configuration.
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::PathBuf,
};

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub struct GuiState {
    pub last_device_id: String,
}

fn path() -> Result<PathBuf, String> {
    let directory = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(".local/state"))
        })
        .ok_or("no state directory is available")?;
    Ok(directory.join("werp/gui.toml"))
}

pub fn load() -> Result<GuiState, String> {
    let path = path()?;
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GuiState::default());
        }
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err(format!("expected a regular state file: {}", path.display()));
    }
    let mut text = String::new();
    file.take(65537)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 65536 {
        return Err("GUI state exceeds 64 KiB".into());
    }
    toml::from_str(&text).map_err(|error| format!("invalid {}: {error}", path.display()))
}

pub fn save(state: &GuiState) -> Result<(), String> {
    let path = path()?;
    let parent = path.parent().ok_or("invalid GUI state path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let text = toml::to_string(state).map_err(|e| e.to_string())?;
    // The only field is replaced as a unit: concurrent selections are last-writer-wins.
    // Unique temporary files and atomic replacement prevent partial state reads.
    let result = (|| -> std::io::Result<()> {
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(text.as_bytes())?;
        temporary.as_file().sync_all()?;
        temporary.persist(&path).map_err(|e| e.error)?;
        Ok(())
    })();
    result.map_err(|e| format!("cannot write {}: {e}", path.display()))
}
