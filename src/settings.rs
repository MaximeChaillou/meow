//! Choices the user makes from the cat's menu, kept across launches.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sprites::Coat;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub coat: Coat,
    /// Slack drafts follow the user's learned style guide.
    pub write_as_me: bool,
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("Could not read settings from {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Could not read {}: {e}", path.display())),
        }
    }

    /// Writes atomically, so a crash never leaves a half-written file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let fail = |e: &dyn std::fmt::Display| {
            format!("Could not save settings to {}: {e}", path.display())
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| fail(&e))?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, json).map_err(|e| fail(&e))?;
        std::fs::rename(&temp, path).map_err(|e| fail(&e))
    }
}

/// `~/Library/Application Support/Meow/settings.json`.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Meow/settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("meow-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let settings = Settings {
            coat: Coat::Alfie,
            write_as_me: true,
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), settings);
        assert_eq!(
            Settings::load(&dir.join("missing.json")).unwrap(),
            Settings::default()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.coat, Coat::Ginger);
        assert!(!settings.write_as_me);
        let willow: Settings = serde_json::from_str(r#"{"coat":"Willow"}"#).unwrap();
        assert_eq!(willow.coat, Coat::Willow);
    }
}
