// SPDX-License-Identifier: MPL-2.0

//! The interim update check, for installs no store keeps current.
//!
//! AppImages and Flatpak bundles have no live update channel, so those
//! builds ask GitHub for the latest release once per launch and offer it
//! in a toast. Local, Nix, and nightly builds stay quiet. A Flathub
//! build will too once that channel exists: its manifest sets
//! HANGAR_DISTRIBUTION=flathub and the store takes over.

/// Whether this build should look for updates at all.
pub fn check_enabled() -> bool {
    if crate::config::IS_DEVEL {
        return false;
    }
    if env!("HANGAR_DISTRIBUTION") == "flathub" {
        return false;
    }
    // A version that is not a plain release number (git-abc1234, a local
    // crate build) has nothing meaningful to compare against.
    if parse_semver(env!("HANGAR_VERSION")).is_none() {
        return false;
    }
    // The AppImage runtime exports APPIMAGE; Flatpak mounts its info file.
    std::env::var_os("APPIMAGE").is_some() || std::path::Path::new("/.flatpak-info").exists()
}

/// "v1.2.3" or "1.2.3" as a comparable triple; anything else is None.
pub fn parse_semver(version: &str) -> Option<(u64, u64, u64)> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// True when `latest_tag` names a release newer than `current`. Refuses
/// to guess: either side failing to parse means no.
pub fn newer_available(current: &str, latest_tag: &str) -> bool {
    match (parse_semver(current), parse_semver(latest_tag)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

/// The latest stable release: its tag and its page. The API's `latest`
/// endpoint skips drafts and prereleases, so nightlies never count.
pub async fn fetch_latest() -> Result<(String, String), String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
        html_url: String,
    }
    let body = reqwest::Client::new()
        .get("https://api.github.com/repos/sethcottle/hangar/releases/latest")
        .header("User-Agent", concat!("hangar/", env!("HANGAR_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    let release: Release = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    Ok((release.tag_name, release.html_url))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Release tags parse with or without the v; channel stamps and
    /// anything else refuse to.
    #[test]
    fn only_release_numbers_parse() {
        assert_eq!(parse_semver("0.7.0"), Some((0, 7, 0)));
        assert_eq!(parse_semver("v1.12.3"), Some((1, 12, 3)));
        assert_eq!(parse_semver("nightly-20260804-abc1234"), None);
        assert_eq!(parse_semver("git-abc1234"), None);
        assert_eq!(parse_semver("0.1.0-dev"), None);
        assert_eq!(parse_semver("1.2"), None);
        assert_eq!(parse_semver("1.2.3.4"), None);
        assert_eq!(parse_semver(""), None);
    }

    /// Newer means numerically newer, never a guess.
    #[test]
    fn newer_never_guesses() {
        assert!(newer_available("0.7.0", "v0.7.1"));
        assert!(newer_available("0.7.9", "v0.8.0"));
        assert!(newer_available("0.9.9", "v1.0.0"));
        assert!(!newer_available("0.7.0", "v0.7.0"));
        assert!(!newer_available("0.8.0", "v0.7.9"));
        assert!(!newer_available("git-abc1234", "v9.9.9"));
        assert!(!newer_available("0.7.0", "nightly"));
    }
}
