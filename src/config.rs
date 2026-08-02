// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

// Nightly builds carry a distinct application ID so they install alongside a
// stable release rather than replacing it. APP_ID also selects the settings
// directory and the secret-service attributes, so the two channels keep
// separate configuration and credentials.
#[cfg(feature = "devel")]
pub const APP_ID: &str = "io.github.sethcottle.Hangar.Devel";
#[cfg(not(feature = "devel"))]
pub const APP_ID: &str = "io.github.sethcottle.Hangar";

#[cfg(feature = "devel")]
pub const APP_NAME: &str = "Hangar (Nightly)";
#[cfg(not(feature = "devel"))]
pub const APP_NAME: &str = "Hangar";

#[cfg(feature = "devel")]
pub const IS_DEVEL: bool = true;
#[cfg(not(feature = "devel"))]
pub const IS_DEVEL: bool = false;

pub const DEFAULT_PDS: &str = "https://bsky.social";
