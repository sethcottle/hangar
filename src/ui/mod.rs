// SPDX-License-Identifier: MPL-2.0

pub mod actor_row;
pub mod avatar_cache;
mod compose_dialog;
pub mod edit_profile;
pub mod external;
mod follow_list_page;
pub mod inline_video;
mod login_dialog;
pub mod media_viewer;
mod message_page;
pub mod post_row;
pub mod progress_icon;
#[cfg(test)]
mod rebind_audit;
pub mod report_dialog;
pub(crate) mod rich_text;
pub mod sidebar;
pub mod video_player;
mod window;

pub use compose_dialog::{ComposeDialog, QuoteContext, ReplyContext};
pub use follow_list_page::{FollowListKind, FollowListPage};
pub use login_dialog::LoginDialog;
pub use message_page::{MessagePage, MessagePush};
pub use sidebar::NavItem;
pub use window::{FollowListPush, HangarWindow, ProfileFeedCtx};

/// Run a test body on the one GTK thread, or skip if there is no display.
///
/// GTK may only be used from the thread that initialised it, and the harness
/// gives every `#[test]` its own thread; `gtk4::init` from a second one panics.
/// Tests that called `init` directly raced: the loser got an `Err` and skipped
/// itself as though headless.
///
/// `#[gtk4::test]` was rejected here because it panics when GTK cannot start,
/// and headless CI has nothing to assert.
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
            let ready = gtk4::init().is_ok() && libadwaita::init().is_ok();
            if ready {
                // Tests see the same icon set the app registers.
                ensure_bundled_icons();
            }
            let _ = tx.send(ready);
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

/// Register the bundled symbolic icons with GTK, once.
///
/// The icons ride inside the binary as a gresource so no desktop's icon
/// theme can come up empty: KDE's Breeze lacks several of the GNOME
/// names this app uses and drew red placeholders in their place. The
/// active theme still wins where it has a name; the bundle only fills
/// the gaps.
pub fn ensure_bundled_icons() {
    use std::sync::OnceLock;
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        gtk4::gio::resources_register_include!("hangar.gresource")
            .expect("the icon bundle is compiled into the binary");
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::IconTheme::for_display(&display)
                .add_resource_path("/io/github/sethcottle/Hangar/icons");
        }
    });
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

#[cfg(test)]
mod icon_tests {
    /// Every bundled icon resolves from the bundle alone, the way it must
    /// on a desktop whose theme has never heard of these names. The
    /// system theme is cut out of the lookup entirely.
    #[test]
    fn bundled_icons_resolve_without_any_system_theme() {
        crate::ui::with_gtk(bundled_icons_resolve_without_any_system_theme_body);
    }

    fn bundled_icons_resolve_without_any_system_theme_body() {
        let bundled = gtk4::gio::resources_enumerate_children(
            "/io/github/sethcottle/Hangar/icons/scalable/actions",
            gtk4::gio::ResourceLookupFlags::NONE,
        )
        .expect("the icon bundle is registered");
        assert!(
            bundled.len() >= 40,
            "the bundle looks gutted: {} entries",
            bundled.len()
        );

        let theme = gtk4::IconTheme::new();
        theme.set_search_path(&[] as &[&std::path::Path]);
        theme.add_resource_path("/io/github/sethcottle/Hangar/icons");

        for file in &bundled {
            let name = file.trim_end_matches(".svg");
            assert!(
                theme.has_icon(name),
                "{name} does not resolve from the bundle"
            );
        }

        // The names KDE actually broke on stay in the bundle by name, so
        // an icon rename cannot quietly reopen the hole.
        for name in [
            "chat-message-new-symbolic",
            "emote-love-symbolic",
            "preferences-system-notifications-symbolic",
            "media-playlist-repeat-symbolic",
            "mail-reply-sender-symbolic",
            "view-reveal-symbolic",
        ] {
            assert!(
                bundled.iter().any(|f| f.trim_end_matches(".svg") == name),
                "{name} left the bundle"
            );
        }
    }
}
