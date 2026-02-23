// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

pub const APP_ID: &str = "io.github.sethcottle.Hangar";
pub const APP_NAME: &str = "Hangar";

#[cfg(feature = "devel")]
pub const IS_DEVEL: bool = true;
#[cfg(not(feature = "devel"))]
pub const IS_DEVEL: bool = false;

pub const DEFAULT_PDS: &str = "https://bsky.social";
