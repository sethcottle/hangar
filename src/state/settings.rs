// SPDX-License-Identifier: MPL-2.0

use crate::config::APP_ID;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

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
    /// hands back defaults for the whole session on a parse error, so a file
    /// missing this key reverted every other setting.
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
        Self::load_from(&path)
    }

    /// Read the file, falling back to defaults for this session.
    ///
    /// The fallback is reported rather than swallowed. It used to be a bare
    /// `unwrap_or_default()`, which is why an unreadable file looked exactly
    /// like a fresh install.
    fn load_from(path: &Path) -> Self {
        Self::read_from(path).unwrap_or_else(|e| {
            eprintln!("hangar: {e}; using default settings for this session");
            Self::default()
        })
    }

    /// Parse the file. Only "there is no file yet" counts as defaults; the
    /// bytes on disk are the sole copy of these preferences, so every other
    /// failure has to reach the caller.
    fn read_from(path: &Path) -> Result<Self, String> {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("could not read {}: {e}", path.display())),
        };

        serde_json::from_str(&contents)
            .map_err(|e| format!("{} did not parse as settings: {e}", path.display()))
    }

    /// Save settings to disk
    pub fn save(&self) -> Result<(), String> {
        let path = Self::settings_path().ok_or("Could not determine config directory")?;
        self.save_to(&path)
    }

    /// Replace the settings file atomically, the way `FileSessionStore` writes
    /// its own.
    ///
    /// A plain `fs::write` truncates in place, so ENOSPC or EIO partway
    /// through leaves a zero-length or half-written file, and since the font
    /// slider and the video volume rewrite this on every change the odds are
    /// not academic.
    /// Whether a file that failed to parse still holds something worth
    /// keeping. Empty or all-NUL is the torn-write signature: no content,
    /// nothing to lose.
    fn is_salvageable(path: &Path) -> bool {
        match std::fs::read(path) {
            Ok(bytes) => bytes.is_empty() || bytes.iter().all(|b| *b == 0),
            // Unreadable is not the same as empty; leave it alone.
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        }
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        // There is no canonical in-memory settings object: every call site is
        // a disk read-modify-write, so if the file did not parse then what is
        // about to be written is defaults wearing the user's preferences.
        // Refuse, and leave the original for them to recover.
        //
        // Unless there is nothing to preserve. An empty or all-NUL file is
        // exactly what the interrupted-write failure looks like, and it
        // holds no preferences, so refusing there would lock the user out
        // of ever saving again with no way back.
        if let Err(e) = Self::read_from(path) {
            if !Self::is_salvageable(path) {
                return Err(format!("not overwriting settings: {e}"));
            }
            eprintln!("hangar: settings file held nothing to preserve, rewriting it");
        }

        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {e}"))?;

        // Per process, so two running copies cannot write through each other's
        // half-finished temp file and rename the result into place.
        let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
        let written = (|| -> std::io::Result<()> {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(json.as_bytes())?;
            // The whole point: a full disk or a bad block surfaces here, while
            // the previous settings are still the ones on disk.
            file.sync_all()
        })();
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Failed to write settings: {e}"));
        }

        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Failed to replace settings: {e}")
        })?;

        // A power cut can otherwise lose the rename itself, and with it the
        // file it pointed at.
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// A torn write leaves an empty or NUL-filled file. That holds no
    /// preferences, so saving over it must be allowed: refusing would
    /// leave the user unable to change a setting ever again. A file with
    /// real but unparseable content is still protected.
    #[test]
    fn a_torn_settings_file_recovers_but_real_content_is_protected() {
        let dir = std::env::temp_dir().join(format!("hangar-torn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let settings = AppSettings::default();

        // Torn: zero length, and NUL filled. Both must be rewritable.
        for (name, body) in [("empty.json", vec![]), ("nulls.json", vec![0u8; 512])] {
            let path = dir.join(name);
            std::fs::write(&path, &body).unwrap();
            settings
                .save_to(&path)
                .unwrap_or_else(|e| panic!("{name} should be recoverable: {e}"));
            assert!(
                AppSettings::read_from(&path).is_ok(),
                "{name} did not come back readable"
            );
        }

        // Real content that does not parse is somebody's settings after a
        // partial write or a bad edit. Do not destroy it.
        let precious = dir.join("garbled.json");
        std::fs::write(&precious, b"{\"font_size\": 1.25, ").unwrap();
        assert!(
            settings.save_to(&precious).is_err(),
            "unparseable but non-empty settings must not be overwritten"
        );
        assert_eq!(
            std::fs::read(&precious).unwrap(),
            b"{\"font_size\": 1.25, ",
            "the original was modified"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

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
    /// [`AppSettings::load`] falls back to defaults on a parse error, so a
    /// field without `#[serde(default)]` resets every other setting.
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

            // Must be `Ok`, or `AppSettings::load` hands back defaults and the
            // file becomes unwritable.
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

    /// A file that does not parse is left exactly as it is.
    ///
    /// `load` used to funnel every parse error into `unwrap_or_default`, and
    /// because every call site is a disk read-modify-write, the next
    /// preference toggle persisted those defaults over whatever had survived.
    /// A user with a truncated file lost their font size and reduce-motion
    /// choice with nothing on screen to say so.
    #[test]
    fn a_settings_file_that_does_not_parse_survives_the_next_save() {
        let dir = std::env::temp_dir().join(format!("hangar-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");

        // What a truncated `fs::write` leaves behind when the disk fills.
        const CORRUPT: &str = r#"{"font_size":1.35,"reduce_mot"#;
        std::fs::write(&path, CORRUPT).unwrap();

        let loaded = AppSettings::load_from(&path);
        assert_eq!(loaded.font_size, FontSize(FontSize::DEFAULT));
        assert!(!loaded.reduce_motion);

        let err = loaded
            .save_to(&path)
            .expect_err("a save of defaults over an unreadable file must be refused");
        assert!(err.contains("not overwriting"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            CORRUPT,
            "the unparseable original was destroyed anyway"
        );

        // A readable file lifts the refusal, and the save lands whole.
        std::fs::write(&path, r#"{"font_size":1.35}"#).unwrap();
        let mut loaded = AppSettings::load_from(&path);
        assert_eq!(loaded.font_size, FontSize(1.35));
        loaded.reduce_motion = true;
        loaded
            .save_to(&path)
            .expect("a save over a readable file lands");

        let reread = AppSettings::load_from(&path);
        assert_eq!(reread.font_size, FontSize(1.35));
        assert!(reread.reduce_motion);

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|name| name != "settings.json")
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
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
