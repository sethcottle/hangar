// SPDX-License-Identifier: MPL-2.0

mod app;
mod atproto;
mod cache;
mod config;
mod runtime;
mod state;
mod ui;

use gtk4::prelude::*;

fn main() {
    let app = app::HangarApplication::new();
    app.run();
}
