// SPDX-License-Identifier: MPL-2.0

use crate::config::APP_ID;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Font size as a scale factor (1.0 = default)
/// Slider steps: 0.8, 0.85, 0.9, 0.95, 1.0, 1.05, 1.1, 1.15, 1.2
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FontSize(pub f64);

impl FontSize {
    /// All discrete slider positions
    pub const STEPS: &'static [f64] = &[0.8, 0.85, 0.9, 0.95, 1.0, 1.05, 1.1, 1.15, 1.2];
    pub const MIN: f64 = 0.8;
    pub const MAX: f64 = 1.2;
    pub const STEP: f64 = 0.05;
    pub const DEFAULT: f64 = 1.0;

    pub fn scale_factor(self) -> f64 {
        self.0
    }

    pub fn label(self) -> &'static str {
        // Snap to nearest step for label
        let val = self.0;
        if val <= 0.82 {
            "Smallest"
        } else if val <= 0.87 {
            "Smaller"
        } else if val <= 0.92 {
            "Small"
        } else if val <= 0.97 {
            "Medium"
        } else if val <= 1.02 {
            "Default"
        } else if val <= 1.07 {
            "Large"
        } else if val <= 1.12 {
            "Larger"
        } else if val <= 1.17 {
            "Largest"
        } else {
            "Maximum"
        }
    }
}

impl Default for FontSize {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl Eq for FontSize {}

/// Persistent application settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    pub font_size: FontSize,
    /// When true, disable animations/transitions regardless of system setting
    #[serde(default)]
    pub reduce_motion: bool,
}

impl AppSettings {
    /// Get the settings file path (~/.config/io.github.sethcottle.Hangar/settings.json)
    fn settings_path() -> Option<PathBuf> {
        dirs::config_dir().map(|mut p| {
            p.push(APP_ID);
            p.push("settings.json");
            p
        })
    }

    /// Load settings from disk, or return defaults if not found
    pub fn load() -> Self {
        let Some(path) = Self::settings_path() else {
            return Self::default();
        };

        match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save settings to disk
    pub fn save(&self) -> Result<(), String> {
        let path = Self::settings_path().ok_or("Could not determine config directory")?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {e}"))?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {e}"))?;

        std::fs::write(&path, json).map_err(|e| format!("Failed to write settings: {e}"))?;

        Ok(())
    }
}
