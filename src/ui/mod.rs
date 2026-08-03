// SPDX-License-Identifier: MPL-2.0

pub mod avatar_cache;
mod compose_dialog;
pub mod external;
pub mod inline_video;
mod login_dialog;
pub mod media_viewer;
pub mod post_row;
#[cfg(test)]
mod rebind_audit;
pub mod sidebar;
pub mod video_player;
mod window;

pub use compose_dialog::{ComposeDialog, QuoteContext, ReplyContext};
pub use login_dialog::LoginDialog;
pub use sidebar::NavItem;
pub use window::HangarWindow;

/// Run a test body on the one GTK thread, or skip if there is no display.
///
/// GTK may only be used from the thread that initialised it, and the harness
/// gives every `#[test]` its own - `gtk4::init` from a second panics. Tests that
/// called `init` directly raced: the loser got an `Err` and skipped itself as
/// though headless.
///
/// Not `#[gtk4::test]`: that panics when GTK cannot start, and headless CI has
/// nothing to assert.
#[cfg(test)]
pub(crate) fn with_gtk<F: FnOnce() + Send + 'static>(body: F) {
    use std::sync::{OnceLock, mpsc};

    static WORKER: OnceLock<Option<gtk4::glib::ThreadPool>> = OnceLock::new();

    let worker = WORKER.get_or_init(|| {
        // These tests open real windows. GTK prefers Wayland, so if
        // WAYLAND_DISPLAY is set they end up on the actual desktop even under
        // xvfb-run. Use ./scripts/test, or HANGAR_TEST_ON_THIS_DISPLAY=1 to
        // watch them run.
        if std::env::var_os("HANGAR_TEST_ON_THIS_DISPLAY").is_none()
            && (std::env::var_os("WAYLAND_DISPLAY").is_some()
                || std::env::var("DISPLAY").is_ok_and(|d| d == ":0" || d == ":0.0"))
        {
            eprintln!(
                "refusing to open test windows on your session. \
                 use ./scripts/test, or HANGAR_TEST_ON_THIS_DISPLAY=1 to override"
            );
            return None;
        }

        let pool = gtk4::glib::ThreadPool::exclusive(1).ok()?;
        let (tx, rx) = mpsc::sync_channel(1);
        pool.push(move || {
            let _ = tx.send(gtk4::init().is_ok() && libadwaita::init().is_ok());
        })
        .ok()?;
        rx.recv().ok()?.then_some(pool)
    });

    let Some(worker) = worker else {
        eprintln!("skipping: no display available");
        return;
    };

    let (tx, rx) = mpsc::sync_channel(1);
    worker
        .push(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
            let _ = tx.send(result);
        })
        .expect("schedule the test body on the GTK thread");

    // Re-raised here so the failure is reported against this test rather than
    // taking the shared worker down with it.
    if let Err(panic) = rx.recv().expect("test body result from the GTK thread") {
        std::panic::resume_unwind(panic);
    }
}

/// Stable replacement for the nightly-only `str::floor_char_boundary`.
pub fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        s.len()
    } else {
        let mut i = index;
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}
