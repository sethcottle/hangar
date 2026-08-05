// SPDX-License-Identifier: MPL-2.0

// The version the app displays. CI exports HANGAR_VERSION (the release
// input for tagged builds, a dated sha for nightlies); without it the
// crate version stands in, so local builds need no setup.
fn main() {
    println!("cargo:rerun-if-env-changed=HANGAR_VERSION");
    let version =
        std::env::var("HANGAR_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=HANGAR_VERSION={version}");

    // Which channel this build ships through. A Flathub build sets
    // "flathub" in its manifest and the in-app update check stands down;
    // everything else is independent and keeps the check.
    println!("cargo:rerun-if-env-changed=HANGAR_DISTRIBUTION");
    let distribution =
        std::env::var("HANGAR_DISTRIBUTION").unwrap_or_else(|_| "independent".to_string());
    println!("cargo:rustc-env=HANGAR_DISTRIBUTION={distribution}");

    // The bundled symbolic icons, so no desktop's icon theme can come up
    // empty (KDE's Breeze lacks several GNOME names).
    glib_build_tools::compile_resources(
        &["assets"],
        "assets/hangar.gresource.xml",
        "hangar.gresource",
    );
}
