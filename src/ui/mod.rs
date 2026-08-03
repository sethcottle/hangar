// SPDX-License-Identifier: MPL-2.0

pub mod avatar_cache;
mod compose_dialog;
pub mod external;
pub mod inline_video;
mod login_dialog;
pub mod media_viewer;
pub mod post_row;
pub mod sidebar;
pub mod video_player;
mod window;

pub use compose_dialog::{ComposeDialog, QuoteContext, ReplyContext};
pub use login_dialog::LoginDialog;
pub use sidebar::NavItem;
pub use window::HangarWindow;

/// Run a test body on the one thread that owns GTK, or skip it if there is no
/// display to build widgets on.
///
/// GTK may only be used from the thread that initialised it, and the harness
/// gives every `#[test]` a thread of its own — `gtk4::init` from a second one
/// panics outright. The tests that predate this helper called `init` directly
/// and appeared to pass only because they raced: whichever thread lost failed
/// to acquire the default main context, got an `Err` back, and quietly skipped
/// itself as though the machine were headless. So every widget test now runs
/// here instead, on a single-thread pool that initialises GTK exactly once.
///
/// Skipping on no display is deliberate and is why this is not just
/// `#[gtk4::test]`: that macro panics when GTK cannot start, and headless CI
/// has nothing to assert rather than a failure to report.
#[cfg(test)]
pub(crate) fn with_gtk<F: FnOnce() + Send + 'static>(body: F) {
    use std::sync::{OnceLock, mpsc};

    static WORKER: OnceLock<Option<gtk4::glib::ThreadPool>> = OnceLock::new();

    let worker = WORKER.get_or_init(|| {
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
