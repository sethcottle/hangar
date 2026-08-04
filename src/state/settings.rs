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
/// A newtype because a bare `f64` would take 0.0 from both `Default` and
/// serde's missing-field path. The hand-written `Default` below is what both
/// use.
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

/// Whether timeline videos start on their own, and with what audio.
///
/// Three variants rather than a bool. Autoplay at the app volume means
/// scrolling the feed plays audio; autoplay always muted gives no way to turn
/// sound on.
///
/// `#[default]` on `Never` is what makes `AppSettings::default()` and serde's
/// missing-field path the same function. Do not reorder without moving it.
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
    /// Text scale factor. `FontSize::default()` is 1.0.
    ///
    /// This was the one field without `#[serde(default)]`. [`Self::load`]
    /// funnels parse errors into `unwrap_or_default()`, so a file missing this
    /// key reverted every other setting.
    /// [`no_single_missing_field_resets_the_others`] fails if a new field
    /// forgets it.
    #[serde(default)]
    pub font_size: FontSize,
    /// When true, disable animations/transitions regardless of system setting
    #[serde(default)]
    pub reduce_motion: bool,
    /// When true, posting waits until every attached image is described
    #[serde(default)]
    pub require_alt_text: bool,
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
    /// Bluesky hides replies by default; Hangar shows them. Phrased as "hide"
    /// so `Default` and serde agree on false without a hand-written impl.
    #[serde(default)]
    pub hide_replies_in_feed: bool,
    /// Volume for in-app video, shared by every video rather than reset each
    /// time a viewer opens.
    #[serde(default)]
    pub video_volume: VideoVolume,
    /// Whether timeline videos start on their own. Off by default.
    ///
    /// Requires `#[serde(default)]`; [`Self::font_size`] explains why.
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

    /// Both paths into `video_autoplay` agree on "off".
    #[test]
    fn autoplay_defaults_to_never_from_both_paths() {
        assert_eq!(AppSettings::default().video_autoplay, VideoAutoplay::Never);

        // Every field is optional now; this file carries only the one that used
        // to be mandatory.
        let minimal: AppSettings =
            serde_json::from_str(r#"{"font_size":1.0}"#).expect("a minimal settings file loads");
        assert_eq!(minimal.video_autoplay, VideoAutoplay::Never);

        let legacy: AppSettings = serde_json::from_str(
            r#"{"font_size":1.3,"reduce_motion":true,"video_volume":0.5,"hide_replies_in_feed":true}"#,
        )
        .expect("a settings file written before autoplay existed still loads");
        assert_eq!(legacy.video_autoplay, VideoAutoplay::Never);
        // The rest of the file survived.
        assert_eq!(legacy.font_size, FontSize(1.3));
        assert!(legacy.reduce_motion);
        assert_eq!(legacy.video_volume, VideoVolume(0.5));
        assert!(legacy.hide_replies_in_feed);
    }

    /// Dropping any one key must cost only that key.
    ///
    /// [`AppSettings::load`] funnels a parse error into `unwrap_or_default()`,
    /// so a field without `#[serde(default)]` resets every other setting.
    ///
    /// The field set comes from the serialized object, so a new field is
    /// covered the day it lands. Compared through `serde_json::Value` because
    /// `AppSettings` derives no `PartialEq`.
    #[test]
    fn no_single_missing_field_resets_the_others() {
        // Every field away from its default, so "reset" and "survived" cannot
        // be confused.
        let populated = AppSettings {
            font_size: FontSize(1.35),
            reduce_motion: true,
            require_alt_text: true,
            color_scheme: ColorScheme::Dark,
            default_post_language: Some("de".to_string()),
            default_threadgate: Some(crate::atproto::ThreadgateConfig {
                allow_rules: vec![crate::atproto::ThreadgateRule::FollowingRule],
            }),
            default_postgate: Some(crate::atproto::PostgateConfig {
                disable_quoting: true,
            }),
            hide_replies_in_feed: true,
            video_volume: VideoVolume(0.25),
            video_autoplay: VideoAutoplay::WithSound,
        };

        let full = serde_json::to_value(&populated).expect("settings serialize");
        let full = full.as_object().expect("settings serialize to an object");
        let keys: Vec<String> = full.keys().cloned().collect();
        assert_eq!(
            keys.len(),
            10,
            "field count changed; add the new field to `populated` above so it is \
             exercised with a non-default value: {keys:?}"
        );

        for dropped in &keys {
            let mut partial = full.clone();
            partial.remove(dropped);

            // Must be `Ok`, or `AppSettings::load` swallows it and hands back
            // defaults.
            let loaded: AppSettings = serde_json::from_value(serde_json::Value::Object(partial))
                .unwrap_or_else(|e| {
                    panic!(
                        "a settings file missing only `{dropped}` failed to parse ({e}); \
                         `AppSettings::load` would silently reset every other setting. \
                         Add #[serde(default)] to `{dropped}`."
                    )
                });

            let reloaded = serde_json::to_value(&loaded).expect("settings re-serialize");
            let reloaded = reloaded.as_object().expect("an object");

            for key in &keys {
                if key == dropped {
                    // The one field that legitimately falls back to its default.
                    continue;
                }
                assert_eq!(
                    reloaded.get(key),
                    full.get(key),
                    "dropping `{dropped}` also lost `{key}`"
                );
            }
        }
    }

    /// The missing-field default for `font_size` is 1.0.
    ///
    /// `#[serde(default)]` calls `FontSize::default()`; falling through to the
    /// inner `f64`'s `Default` would give a 0.0 scale factor.
    #[test]
    fn a_settings_file_with_no_font_size_reads_as_the_default_scale() {
        let empty: AppSettings = serde_json::from_str("{}").expect("an empty settings file loads");
        assert_eq!(empty.font_size, FontSize(FontSize::DEFAULT));
        assert_eq!(empty.font_size, FontSize(1.0));
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
