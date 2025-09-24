use std::io::Write;
use std::path::PathBuf;

use crate::utils::app_config_dir;

// use crate::utils::app_config_dir; // no longer needed after legacy removal

use super::{Settings, SettingsWindow};

impl SettingsWindow {
    pub(super) fn save_settings(&self) {
        if let Ok(config_str) = toml::to_string(&self.settings) {
            let config_path = Self::get_config_path();

            // Create settings directory if missing
            if let Some(parent) = config_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            // Atomic-ish write: write to temp file then rename
            let tmp_path = config_path.with_extension("toml.tmp");
            if let Ok(mut f) = std::fs::File::create(&tmp_path) {
                let _ = f.write_all(config_str.as_bytes());
                let _ = f.flush();
                let _ = std::fs::rename(tmp_path, config_path);
            }
        }
    }

    pub(super) fn load_settings() -> Result<Settings, Box<dyn std::error::Error>> {
        let config_path = Self::get_config_path();

        // Try OS-standard config dir first
        if config_path.exists() {
            let config_str = std::fs::read_to_string(&config_path)?;
            return Ok(toml::from_str(&config_str)?);
        }

        Err("Settings file not found".into())
    }

    pub(super) fn get_config_path() -> PathBuf {
        // Use app_config_dir() which prefers lowercase (legacy uppercase supported)
        app_config_dir().join("settings.toml")
    }

    // removed: legacy root config migration helpers

    // Removed: Ollama/ONNX-related settings
}
