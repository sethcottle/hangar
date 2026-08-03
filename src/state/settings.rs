// SPDX-License-Identifier: MPL-2.0

use crate::config::APP_ID;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Font size as a scale factor (1.0 = default)
/// Extended range for low-vision accessibility (WCAG 1.4.4)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FontSize(pub f64);

impl FontSize {
    /// All discrete slider positions
    pub const STEPS: &'static [f64] = &[
        0.7, 0.75, 0.8, 0.85, 0.9, 0.95, 1.0, 1.05, 1.1, 1.15, 1.2, 1.25, 1.3, 1.35, 1.4, 1.45, 1.5,
    ];
    pub const MIN: f64 = 0.7;
    pub const MAX: f64 = 1.5;
    pub const STEP: f64 = 0.05;
    pub const DEFAULT: f64 = 1.0;

    pub fn scale_factor(self) -> f64 {
        self.0
    }

    pub fn label(self) -> &'static str {
        let val = self.0;
        if val <= 0.72 {
            "Smallest"
        } else if val <= 0.77 {
            "Tiny"
        } else if val <= 0.82 {
            "Smaller"
        } else if val <= 0.87 {
            "Small"
        } else if val <= 0.92 {
            "Compact"
        } else if val <= 0.97 {
            "Medium"
        } else if val <= 1.02 {
            "Default"
        } else if val <= 1.07 {
            "Large"
        } else if val <= 1.12 {
            "Larger"
        } else if val <= 1.17 {
            "Big"
        } else if val <= 1.22 {
            "Biggest"
        } else if val <= 1.27 {
            "Huge"
        } else if val <= 1.32 {
            "Huger"
        } else if val <= 1.37 {
            "Extra Large"
        } else if val <= 1.42 {
            "Largest"
        } else if val <= 1.47 {
            "Massive"
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

/// Video playback volume, 0.0..=1.0 on a perceptual scale.
///
/// A newtype for the same reason `FontSize` is one: a bare `f64` field would
/// take `f64`'s `Default` of 0.0 both from `AppSettings::default()` and from
/// serde's missing-field path, so every existing settings file would silently
/// mean silence. The hand-written `Default` below is what both use.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VideoVolume(pub f64);

impl VideoVolume {
    pub const MIN: f64 = 0.0;
    pub const MAX: f64 = 1.0;
    pub const STEP: f64 = 0.05;
    pub const DEFAULT: f64 = 1.0;

    pub fn value(self) -> f64 {
        self.0.clamp(Self::MIN, Self::MAX)
    }
}

impl Default for VideoVolume {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl Eq for VideoVolume {}

/// Whether videos in the main timeline start on their own, and with what audio.
///
/// Three variants rather than a bool because a bool has to silently pick a
/// meaning: "autoplay at the app-wide volume" makes scrolling the feed blast
/// audio, and "autoplay always muted" leaves the person who wanted sound with
/// no way to ask for it.
///
/// `Never` carries `#[default]`, which is what makes `AppSettings::default()`
/// and serde's missing-field path the *same function* — unlike a bare `bool` or
/// `f64` field, an enum with an explicit `#[default]` variant cannot have the
/// two disagree, which is exactly what [`VideoVolume`]'s hand-written `Default`
/// exists to prevent. The ordering is load-bearing for readers, not for
/// correctness: do not reorder without moving `#[default]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoAutoplay {
    #[default]
    Never,
    Muted,
    WithSound,
}

impl VideoAutoplay {
    /// Every variant, in the order the settings combo shows them.
    pub const ALL: &'static [Self] = &[Self::Never, Self::Muted, Self::WithSound];

    pub fn label(self) -> &'static str {
        match self {
            Self::Never => "Never",
            Self::Muted => "Muted",
            Self::WithSound => "With Sound",
        }
    }

    pub fn enabled(self) -> bool {
        !matches!(self, Self::Never)
    }

    pub fn starts_muted(self) -> bool {
        matches!(self, Self::Muted)
    }
}

/// User-preferred color scheme
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorScheme {
    #[default]
    System,
    Light,
    Dark,
}

impl ColorScheme {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

/// Persistent application settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    pub font_size: FontSize,
    /// When true, disable animations/transitions regardless of system setting
    #[serde(default)]
    pub reduce_motion: bool,
    /// Color scheme preference (System follows desktop theme)
    #[serde(default)]
    pub color_scheme: ColorScheme,
    /// Default post language (BCP 47 code, e.g. "en"). None = "en".
    #[serde(default)]
    pub default_post_language: Option<String>,
    /// Default threadgate config (who can reply). None = everyone.
    #[serde(default)]
    pub default_threadgate: Option<crate::atproto::ThreadgateConfig>,
    /// Default postgate config (quote controls). None = quoting allowed.
    #[serde(default)]
    pub default_postgate: Option<crate::atproto::PostgateConfig>,
    /// Hide replies in the main feed.
    ///
    /// Bluesky hides replies by default; Hangar has always shown them, and
    /// showing them stays the default so the feed does not quietly change for
    /// anyone already using the app. Phrased as "hide" rather than "show" so
    /// that `Default` and serde's missing-field default agree on false without
    /// needing a hand-written Default impl.
    #[serde(default)]
    pub hide_replies_in_feed: bool,
    /// Volume for in-app video, shared by every video rather than reset each
    /// time a viewer opens.
    #[serde(default)]
    pub video_volume: VideoVolume,
    /// Whether timeline videos start on their own. Off by default: a feed that
    /// starts making noise the moment it loads is a feed people turn off.
    ///
    /// `#[serde(default)]` is not optional here, and what it prevents is worse
    /// than one field defaulting wrong: [`Self::load`] is
    /// `from_str(..).unwrap_or_default()`, so a missing-field *parse error*
    /// would silently discard the user's whole settings file.
    #[serde(default)]
    pub video_autoplay: VideoAutoplay,
}

impl AppSettings {
    /// Get the settings file path (~/.config/<APP_ID>/settings.json)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Both paths into `video_autoplay` have to agree on "off".
    ///
    /// The legacy-blob case is the one that catches a forgotten
    /// `#[serde(default)]`: `AppSettings::load` swallows a parse error into
    /// `unwrap_or_default()`, so a required field would not fail loudly — it
    /// would quietly reset a real user's font size, colour scheme and threadgate
    /// defaults the first time they opened the app.
    #[test]
    fn autoplay_defaults_to_never_from_both_paths() {
        assert_eq!(AppSettings::default().video_autoplay, VideoAutoplay::Never);

        // `font_size` is the one field with no `#[serde(default)]`, so it is
        // the minimum a file has to carry; everything since has been optional
        // and autoplay is no exception.
        let minimal: AppSettings =
            serde_json::from_str(r#"{"font_size":1.0}"#).expect("a minimal settings file loads");
        assert_eq!(minimal.video_autoplay, VideoAutoplay::Never);

        let legacy: AppSettings = serde_json::from_str(
            r#"{"font_size":1.3,"reduce_motion":true,"video_volume":0.5,"hide_replies_in_feed":true}"#,
        )
        .expect("a settings file written before autoplay existed still loads");
        assert_eq!(legacy.video_autoplay, VideoAutoplay::Never);
        // The rest of the file survived, which is the actual stake.
        assert_eq!(legacy.font_size, FontSize(1.3));
        assert!(legacy.reduce_motion);
        assert_eq!(legacy.video_volume, VideoVolume(0.5));
        assert!(legacy.hide_replies_in_feed);
    }

    #[test]
    fn autoplay_round_trips_by_name() {
        for mode in VideoAutoplay::ALL {
            let json = serde_json::to_string(mode).expect("serialize");
            let back: VideoAutoplay = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, *mode, "{json} did not round-trip");
        }
        assert_eq!(
            serde_json::to_string(&VideoAutoplay::WithSound).unwrap(),
            "\"WithSound\""
        );
    }

    #[test]
    fn only_muted_starts_muted_and_only_never_is_off() {
        assert!(!VideoAutoplay::Never.enabled());
        assert!(VideoAutoplay::Muted.enabled());
        assert!(VideoAutoplay::WithSound.enabled());
        assert!(VideoAutoplay::Muted.starts_muted());
        assert!(!VideoAutoplay::WithSound.starts_muted());
    }
}
