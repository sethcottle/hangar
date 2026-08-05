// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::collapsible_if)]

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use once_cell::sync::Lazy;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use tokio::sync::Semaphore;

use crate::atproto::client::{ClientError, UnreadCounts};
use crate::atproto::{
    ChatMessage, Conversation, HangarClient, Notification, Post, Profile, SavedFeed, Session,
};
use crate::cache::{CacheDb, FeedCache, FeedState, PostCache, ProfileCache};
use crate::config;
use crate::runtime;
use crate::state::SessionManager;
use crate::state::oauth::OAuthManager;
use crate::ui::avatar_cache;
use crate::ui::post_row::PostRow;
use crate::ui::{
    ComposeDialog, FollowListKind, FollowListPage, FollowListPush, HangarWindow, LoginDialog,
    MessagePage, MessagePush, NavItem, QuoteContext, ReplyContext,
};

/// Limit concurrent API requests to prevent overwhelming the server during rapid scrolling
static API_SEMAPHORE: Lazy<Arc<Semaphore>> = Lazy::new(|| Arc::new(Semaphore::new(4)));

/// Seconds between polls for new messages while a conversation is open.
const MESSAGE_POLL_SECS: u32 = 5;

/// Which turn of events owns some shared state.
///
/// Every reset bumps it. A fetch carries the token it started under and
/// drops its results if a bump happened since, so a slow response cannot
/// overwrite state the user has already moved past. Searches use one per
/// query; the badge clear uses one per section open.
#[derive(Default)]
pub(crate) struct Generation(std::cell::Cell<u64>);

impl Generation {
    /// Take ownership. Every earlier token goes stale.
    fn bump(&self) {
        self.0.set(self.0.get().wrapping_add(1));
    }

    /// The token a fetch spawned now should carry.
    fn token(&self) -> u64 {
        self.0.get()
    }

    /// Whether a fetch holding `token` may still apply its results.
    fn is_current(&self, token: u64) -> bool {
        self.0.get() == token
    }
}

/// Why a stored session could not be resumed on launch.
///
/// The receiving end used to discard the error and put up a bare login dialog
/// either way, so a permanently refused session looked like a first run.
enum RestoreFailure {
    /// Dead session: the tokens were refused, or the `client_id` they were
    /// issued under cannot be rebuilt. Clear the keyring too.
    Reauth,
    /// Everything else: nothing stored yet, the keyring unavailable, the
    /// network down mid-restore. The stored session may well still be good.
    Other(String),
}

mod imp {
    use super::*;
    use libadwaita::subclass::prelude::*;

    #[derive(Default)]
    pub struct HangarApplication {
        pub client: RefCell<Option<Arc<HangarClient>>>,
        pub cache: RefCell<Option<CacheDb>>,
        pub window: RefCell<Option<HangarWindow>>,
        pub timeline_cursor: RefCell<Option<String>>,
        pub loading_more: RefCell<bool>,
        /// The URI of the newest post we've shown the user (anchor for new posts detection)
        pub newest_post_uri: RefCell<Option<String>>,
        /// Pending new posts that arrived while user was scrolled away
        /// Whether we're currently checking for new posts
        pub checking_new_posts: RefCell<bool>,
        /// A poll already said it is failing; stay quiet until it recovers.
        pub new_posts_poll_failed: RefCell<bool>,
        pub unread_poll_failed: RefCell<bool>,
        /// The currently selected feed
        pub current_feed: RefCell<Option<SavedFeed>>,
        /// Mentions state
        pub mentions_cursor: RefCell<Option<String>>,
        pub mentions_loading_more: RefCell<bool>,
        /// Activity state
        pub activity_cursor: RefCell<Option<String>>,
        pub activity_loading_more: RefCell<bool>,
        /// Chat state
        pub chat_cursor: RefCell<Option<String>>,
        pub chat_loading_more: RefCell<bool>,
        /// Profile state
        /// Store the logged-in user's DID for fetching own profile
        pub user_did: RefCell<Option<String>>,
        /// Likes state
        pub likes_cursor: RefCell<Option<String>>,
        pub likes_loading_more: RefCell<bool>,
        /// Saved posts state. Saves happen from any list at any time, so the
        /// section refetches on every open; the generation keeps a page-two
        /// append from an earlier visit off the fresh list.
        pub bookmarks_cursor: RefCell<Option<String>>,
        pub bookmarks_loading_more: RefCell<bool>,
        pub(crate) bookmarks_generation: Generation,
        /// Search state
        pub search_query: RefCell<Option<String>>,
        pub search_cursor: RefCell<Option<String>>,
        pub search_loading_more: RefCell<bool>,
        /// People results page on its own cursor
        pub search_people_cursor: RefCell<Option<String>>,
        pub search_people_loading_more: RefCell<bool>,
        /// Which query the in-flight search fetches belong to
        pub(crate) search_generation: Generation,
        /// Same idea for the New Message picker's typeahead.
        pub(crate) message_target_generation: Generation,
        /// Mirrors gio's NetworkMonitor; the banner and poll guards read it.
        pub(crate) offline: RefCell<bool>,
        /// Whether new posts polling has been started
        pub polling_started: RefCell<bool>,
        /// Whether an unread-badge fetch is already in flight
        pub checking_unread: RefCell<bool>,
        /// Bumped when opening a section clears the notification badges, so
        /// an unread poll already in flight cannot relight them
        pub(crate) badge_clear_generation: Generation,
        /// Set once a request dies of an expired session. Parks the polls
        /// and the toast until the next successful sign-in
        pub session_dead: RefCell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HangarApplication {
        const NAME: &'static str = "HangarApplication";
        type Type = super::HangarApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for HangarApplication {
        fn constructed(&self) {
            self.parent_constructed();

            // Initialize the client
            let client = Arc::new(HangarClient::new());
            self.client.replace(Some(client));
        }
    }

    impl ApplicationImpl for HangarApplication {
        fn startup(&self) {
            self.parent_startup();

            self.obj().setup_gactions();

            // Register custom icons
            let display = gtk4::gdk::Display::default().expect("Could not get default display");
            let icon_theme = gtk4::IconTheme::for_display(&display);

            // Add the bundled icons path. Development and installed layouts differ, so try both.
            if let Ok(exe_path) = std::env::current_exe() {
                if let Some(exe_dir) = exe_path.parent() {
                    // For development: look relative to executable
                    let dev_icons = exe_dir.join("../assets/icons");
                    if dev_icons.exists() {
                        icon_theme.add_search_path(&dev_icons);
                    }
                    // Also try assets/icons from cwd
                    let cwd_icons = std::path::Path::new("assets/icons");
                    if cwd_icons.exists() {
                        icon_theme.add_search_path(cwd_icons);
                    }
                }
            }

            // Load CSS
            let css_provider = gtk4::CssProvider::new();
            css_provider.load_from_string(include_str!("ui/style.css"));

            gtk4::style_context_add_provider_for_display(
                &display,
                &css_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        fn activate(&self) {
            let app = self.obj();

            crate::ui::ensure_bundled_icons();

            // Create main window
            let window = HangarWindow::new(app.upcast_ref::<adw::Application>());
            if config::IS_DEVEL {
                window.add_css_class("devel");
            }
            self.window.replace(Some(window.clone()));

            // Connectivity: the banner and the poll guards follow gio's
            // monitor rather than waiting for requests to time out.
            let monitor = gio::NetworkMonitor::default();
            let app_clone = app.clone();
            monitor.connect_network_changed(move |_, available| {
                app_clone.set_network_available(available);
            });
            app.set_network_available(monitor.is_network_available());

            // The poll skips while the window is backgrounded; a regained
            // focus catches the feed up right away instead of waiting out
            // the timer.
            let app_clone = app.clone();
            window.connect_is_active_notify(move |window| {
                if window.is_active() {
                    app_clone.check_for_new_posts();
                }
            });

            // The update nudge, only on builds no store keeps current, a
            // little after launch so startup owns the first seconds.
            if crate::update::check_enabled() {
                let app_clone = app.clone();
                glib::timeout_add_seconds_local_once(15, move || {
                    app_clone.check_for_updates();
                });
            }

            let app_clone = app.clone();
            window.set_load_more_callback(move || {
                app_clone.fetch_timeline_more();
            });

            let app_clone = app.clone();
            window.set_refresh_callback(move || {
                app_clone.fetch_timeline();
            });

            let app_clone = app.clone();
            window.set_like_callback(move |post, post_row_weak| {
                app_clone.toggle_like(&post, post_row_weak);
            });

            let app_clone = app.clone();
            window.set_repost_callback(move |post, post_row_weak| {
                app_clone.toggle_repost(&post, post_row_weak);
            });

            let app_clone = app.clone();
            window.set_quote_callback(move |post| {
                app_clone.open_quote_dialog(post);
            });

            let app_clone = app.clone();
            window.set_reply_callback(move |post| {
                app_clone.open_reply_dialog(post);
            });

            // One handler serves every PostRow; see post_row.rs.
            let app_clone = app.clone();
            crate::ui::post_row::set_delete_post_handler(move |post| {
                app_clone.confirm_delete_post(post);
            });

            // Save/unsave, dispatched the same way.
            let app_clone = app.clone();
            crate::ui::post_row::set_bookmark_post_handler(move |post, row_weak| {
                app_clone.toggle_bookmark(post, row_weak);
            });

            let app_clone = app.clone();
            window.set_follow_callback(move |did, uri_cell, btn_weak| {
                app_clone.toggle_follow(did, uri_cell, btn_weak);
            });

            // Follow from people rows, one handler for every list.
            let app_clone = app.clone();
            crate::ui::actor_row::set_follow_handler(move |profile, row_weak| {
                app_clone.toggle_follow_row(profile, row_weak);
            });

            let app_clone = app.clone();
            window.set_message_callback(move |profile, btn_weak| {
                app_clone.open_direct_message(profile, Some(btn_weak));
            });

            let app_clone = app.clone();
            window.set_new_message_query_callback(move |query| {
                app_clone.search_message_targets(query);
            });

            let app_clone = app.clone();
            window.set_new_message_chosen_callback(move |profile| {
                app_clone.open_direct_message(profile, None);
            });

            let app_clone = app.clone();
            window.set_profile_tab_callback(move |ctx, first_page| {
                app_clone.fetch_profile_tab(ctx, first_page);
            });

            let app_clone = app.clone();
            window.set_edit_profile_callback(move || {
                app_clone.open_edit_profile();
            });

            let app_clone = app.clone();
            window.set_mute_callback(move |did, cell| {
                app_clone.toggle_mute(did, cell);
            });

            let app_clone = app.clone();
            window.set_block_callback(move |profile, cell| {
                app_clone.toggle_block(profile, cell);
            });

            let app_clone = app.clone();
            window.set_report_account_callback(move |profile| {
                app_clone.open_report_for_account(profile);
            });

            let app_clone = app.clone();
            crate::ui::post_row::set_report_post_handler(move |post| {
                app_clone.open_report_for_post(post);
            });

            // Feed-level moderation starts from a clean cell: mute always
            // mutes, block always confirms then blocks. The profile page
            // owns the stateful undo side.
            let app_clone = app.clone();
            crate::ui::post_row::set_mute_account_handler(move |did| {
                app_clone.toggle_mute(did, std::rc::Rc::new(std::cell::Cell::new(false)));
            });

            let app_clone = app.clone();
            crate::ui::post_row::set_block_account_handler(move |profile| {
                app_clone.toggle_block(profile, std::rc::Rc::new(RefCell::new(None)));
            });

            let app_clone = app.clone();
            window.set_compose_callback(move || {
                app_clone.open_compose_dialog();
            });

            let app_clone = app.clone();
            window.set_new_posts_callback(move || {
                app_clone.show_new_posts();
            });

            let app_clone = app.clone();
            window.set_feed_changed_callback(move |feed| {
                app_clone.switch_feed(feed);
            });

            let app_clone = app.clone();
            window.set_post_clicked_callback(move |post| {
                app_clone.open_thread_view(post);
            });

            let app_clone = app.clone();
            window.set_profile_clicked_callback(move |profile| {
                app_clone.open_profile_view(profile);
            });

            let app_clone = app.clone();
            window.set_follow_list_clicked_callback(move |profile, kind| {
                app_clone.open_follow_list(profile, kind);
            });

            let app_clone = app.clone();
            window.set_mention_clicked_callback(move |handle| {
                app_clone.open_profile_by_handle(handle);
            });

            let app_clone = app.clone();
            window.set_nav_changed_callback(move |item| {
                app_clone.handle_nav_change(item);
            });

            let app_clone = app.clone();
            window.set_mentions_load_more_callback(move || {
                app_clone.fetch_mentions_more();
            });

            let app_clone = app.clone();
            window.set_activity_load_more_callback(move || {
                app_clone.fetch_activity_more();
            });

            let app_clone = app.clone();
            window.set_chat_load_more_callback(move || {
                app_clone.fetch_chat_more();
            });

            let app_clone = app.clone();
            window.set_conversation_clicked_callback(move |conversation| {
                app_clone.open_conversation_view(conversation);
            });

            let app_clone = app.clone();
            window.set_likes_load_more_callback(move || {
                app_clone.fetch_likes_more();
            });

            let app_clone = app.clone();
            window.set_bookmarks_load_more_callback(move || {
                app_clone.fetch_bookmarks_more();
            });

            let app_clone = app.clone();
            window.set_search_load_more_callback(move || {
                app_clone.fetch_search_more();
            });

            let app_clone = app.clone();
            window.set_search_people_load_more_callback(move || {
                app_clone.fetch_search_people_more();
            });

            let app_clone = app.clone();
            window.set_search_callback(move |query| {
                app_clone.execute_search(query);
            });

            // Avatar menu callbacks: My Profile, Settings, Sign Out
            let app_clone = app.clone();
            window.set_my_profile_clicked_callback(move || {
                if let Some(window) = app_clone.imp().window.borrow().as_ref() {
                    window.select_nav(NavItem::Profile);
                }
                app_clone.open_own_profile_view();
            });

            let app_clone = app.clone();
            window.set_settings_clicked_callback(move || {
                if let Some(window) = app_clone.imp().window.borrow().as_ref() {
                    window.show_settings_page();
                }
            });

            let app_clone = app.clone();
            window.set_about_clicked_callback(move || {
                app_clone.show_about();
            });

            let app_clone = app.clone();
            window.set_sign_out_clicked_callback(move || {
                app_clone.sign_out();
            });

            // Apply saved settings on startup
            let saved_settings = crate::state::AppSettings::load();
            window.apply_font_size(saved_settings.font_size);
            window.apply_color_scheme(saved_settings.color_scheme);
            if saved_settings.reduce_motion {
                window.apply_reduce_motion(true);
            }

            window.present();

            app.try_restore_session();
        }
    }

    impl GtkApplicationImpl for HangarApplication {}
    impl AdwApplicationImpl for HangarApplication {}
}

glib::wrapper! {
    pub struct HangarApplication(ObjectSubclass<imp::HangarApplication>)
        @extends adw::Application, gtk4::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl HangarApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", config::APP_ID)
            .property("flags", gio::ApplicationFlags::FLAGS_NONE)
            .build()
    }

    fn client(&self) -> Arc<HangarClient> {
        self.imp()
            .client
            .borrow()
            .clone()
            .expect("client not initialized")
    }

    /// Application actions and their accelerators. Everything the keyboard
    /// can do routes through here, so the shortcuts window and any future
    /// menus read from one place.
    fn setup_gactions(&self) {
        let refresh = gio::ActionEntry::builder("refresh")
            .activate(|app: &Self, _, _| app.fetch_timeline())
            .build();

        let compose = gio::ActionEntry::builder("compose")
            .activate(|app: &Self, _, _| {
                if app.imp().user_did.borrow().is_some() {
                    app.open_compose_dialog();
                }
            })
            .build();

        let search = gio::ActionEntry::builder("search")
            .activate(|app: &Self, _, _| {
                if let Some(window) = app.imp().window.borrow().as_ref() {
                    window.select_nav(NavItem::Search);
                }
                app.open_search_view();
            })
            .build();

        let back = gio::ActionEntry::builder("back")
            .activate(|app: &Self, _, _| {
                if let Some(window) = app.imp().window.borrow().as_ref() {
                    window.go_back();
                }
            })
            .build();

        // One action, the section index as its target, so each rail row is
        // addressable as app.nav(n).
        let nav = gio::ActionEntry::builder("nav")
            .parameter_type(Some(&i32::static_variant_type()))
            .activate(|app: &Self, _, parameter| {
                let index = parameter.and_then(|v| v.get::<i32>()).unwrap_or(0);
                let Some(item) = NavItem::all().get(index as usize).copied() else {
                    return;
                };
                if let Some(window) = app.imp().window.borrow().as_ref() {
                    window.select_nav(item);
                }
                app.handle_nav_change(item);
            })
            .build();

        let shortcuts = gio::ActionEntry::builder("shortcuts")
            .activate(|app: &Self, _, _| {
                if let Some(window) = app.imp().window.borrow().as_ref() {
                    window.present_shortcuts_window();
                }
            })
            .build();

        let about = gio::ActionEntry::builder("about")
            .activate(|app: &Self, _, _| app.show_about())
            .build();

        self.add_action_entries([refresh, compose, search, back, nav, shortcuts, about]);

        self.set_accels_for_action("app.refresh", &["F5", "<primary>r"]);
        self.set_accels_for_action("app.compose", &["<primary>n"]);
        self.set_accels_for_action("app.search", &["<primary>k"]);
        self.set_accels_for_action("app.back", &["<alt>Left"]);
        for index in 0..NavItem::all().len() {
            self.set_accels_for_action(
                &format!("app.nav({index})"),
                &[&format!("<alt>{}", index + 1)],
            );
        }
        self.set_accels_for_action("app.shortcuts", &["<primary>question"]);
    }

    /// The About dialog, separate from presenting so tests can read it.
    fn build_about_dialog() -> adw::AboutDialog {
        let about = adw::AboutDialog::builder()
            .application_name("Hangar")
            .application_icon(config::APP_ID)
            .developer_name("Seth Cottle")
            .version(env!("HANGAR_VERSION"))
            .license_type(gtk4::License::Mpl20)
            .website("https://hangar.blue")
            .issue_url("https://github.com/sethcottle/hangar/issues")
            .build();
        // Standing on the shoulders of giants; each entry links home.
        about.add_acknowledgement_section(
            Some("Built On"),
            &[
                "GTK https://gtk.org",
                "Libadwaita https://gitlab.gnome.org/GNOME/libadwaita",
                "gtk-rs https://gtk-rs.org",
                "GStreamer https://gstreamer.freedesktop.org",
                "atrium https://github.com/atrium-rs/atrium",
                "Tokio https://tokio.rs",
                "SQLite https://sqlite.org",
            ],
        );
        about
    }

    fn show_about(&self) {
        if let Some(window) = self.imp().window.borrow().as_ref() {
            Self::build_about_dialog().present(Some(window));
        }
    }

    fn try_restore_session(&self) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Session, RestoreFailure>>();
        let client = self.client();

        thread::spawn(move || {
            // Use the shared runtime for network operations so the HTTP client
            // context is consistent across all API calls
            let result = runtime::block_on(async {
                let session = SessionManager::load()
                    .await
                    .map_err(|e| RestoreFailure::Other(e.to_string()))?;
                match client.resume_session(&session).await {
                    Ok(()) => Ok(session),
                    // Permanent: drop the stored session and ask for a new
                    // sign-in.
                    Err(ClientError::ReauthRequired) => Err(RestoreFailure::Reauth),
                    Err(e) => Err(RestoreFailure::Other(e.to_string())),
                }
            });
            let _ = tx.send(result);
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(session)) => {
                    // Initialize cache for this user
                    match CacheDb::open(&session.did) {
                        Ok(cache) => {
                            // Clean up stale entries in background to not block UI
                            let cache_for_cleanup = cache.clone();
                            std::thread::spawn(move || {
                                if let Err(e) = cache_for_cleanup.cleanup_stale() {
                                    eprintln!("Cache cleanup failed: {}", e);
                                }
                            });
                            // Initialize image cache with database reference
                            avatar_cache::init(Arc::new(cache.clone()));
                            // Run image cache cleanup in background
                            avatar_cache::cleanup_cache();
                            app.imp().cache.replace(Some(cache));
                        }
                        Err(e) => {
                            eprintln!("Failed to open cache: {}", e);
                        }
                    }

                    if app.imp().window.borrow().as_ref().is_some() {
                        app.fetch_user_profile(&session.did);
                        app.fetch_saved_feeds();
                        app.fetch_timeline();
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(failure)) => {
                    let notice = match &failure {
                        RestoreFailure::Reauth => {
                            // The OAuth row on disk is already gone, but the
                            // keyring copy makes the next launch land here
                            // again. Clear it, or this repeats forever.
                            thread::spawn(move || {
                                if let Err(e) = runtime::block_on(SessionManager::clear()) {
                                    eprintln!("hangar: could not clear the expired session: {e}");
                                }
                            });
                            Some(
                                "Your session expired and could not be renewed. \
                                 Please sign in again.",
                            )
                        }
                        RestoreFailure::Other(e) => {
                            // Often just "nothing stored yet", so the dialog
                            // says nothing. See the log.
                            eprintln!("hangar: could not restore the saved session: {e}");
                            None
                        }
                    };
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        app.show_login_dialog_with_notice(window, notice);
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        app.show_login_dialog(window);
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            }
        });
    }

    /// Tell the authorization server the grant is over, then drop the
    /// local credential.
    ///
    /// The delete runs whatever the revoke did: an offline sign-out must
    /// still leave nothing usable on disk, and a revoke that failed is not
    /// a reason to keep the key.
    fn revoke_and_forget(did_str: &str) {
        let Ok(did) = did_str.parse::<atrium_api::types::string::Did>() else {
            eprintln!("Sign out: unparseable DID, leaving the session file alone");
            return;
        };
        let store = crate::state::session_store::FileSessionStore::new();

        match OAuthManager::build_restore_client(store.clone(), &did) {
            Ok(client) => {
                if let Err(e) = runtime::block_on(client.revoke(&did)) {
                    // Expected offline, or when the grant is already gone.
                    eprintln!("Sign out: the server did not confirm the revoke: {e}");
                }
            }
            Err(e) => eprintln!("Sign out: no client to revoke with: {e}"),
        }

        if let Err(e) = store.forget(&did) {
            eprintln!("Sign out: failed to drop the stored credential: {e}");
        }
    }

    /// Sign out: clear session, close window, and restart fresh
    fn sign_out(&self) {
        // Drop the live agent first; closing the window does not. The client
        // and the 30-second poll both hang off the application, so the old
        // account's agent would keep refreshing tokens and writing its own row
        // over the session file the new sign-in is writing into.
        self.client().sign_out();

        // Read before the per-account state below clears it; the revoke
        // and the file row are both keyed by DID.
        let did_for_revoke = self.imp().user_did.borrow().clone();

        // Per-account state that outlives the window. The poll checks
        // `newest_post_uri` before doing anything, so clearing it quiets it.
        let imp = self.imp();
        imp.newest_post_uri.replace(None);
        imp.timeline_cursor.replace(None);
        imp.current_feed.replace(None);
        imp.user_did.replace(None);
        imp.cache.replace(None);
        // The next account must not inherit this one's Delete offers.
        crate::ui::post_row::set_current_user_did(None);
        crate::ui::actor_row::set_viewer_did(None);

        // Clear the stored session: the keyring row, the server-side
        // grant, and the OAuth credential file. The keyring alone is not
        // enough. For an OAuth login it carries only the DID and handle
        // while the DPoP key and refresh token live in oauth-sessions.json,
        // so stopping there left a working credential on disk.
        let signed_out_did = did_for_revoke;
        thread::spawn(move || {
            if let Err(e) = runtime::block_on(SessionManager::clear()) {
                eprintln!("Failed to clear the keyring entry: {e}");
            }
            if let Some(did) = signed_out_did {
                Self::revoke_and_forget(&did);
            }
        });

        // Closing the current window drops all UI state.
        if let Some(window) = self.imp().window.take() {
            window.close();
        }

        // Re-activate the app, which creates a fresh window and
        // runs try_restore_session (which will fail → login dialog)
        gio::prelude::ApplicationExt::activate(self.upcast_ref::<gio::Application>());
    }

    /// Say so once when a request died of an expired session.
    ///
    /// The client latches a refusal it could not refresh past: nothing
    /// downstream of `e.to_string()` can tell that apart from the server being
    /// briefly unreachable. Every failing request re-arms that latch, so the
    /// `session_dead` flag is what holds this to one toast per dead session.
    /// The same flag parks the polls; `fetch_user_profile` lifts it on the
    /// next successful sign-in.
    fn report_session_expiry(&self) {
        if !self.client().take_session_expired() {
            return;
        }
        if *self.imp().session_dead.borrow() {
            return;
        }
        self.imp().session_dead.replace(true);
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.show_toast("Your session expired. Sign out and sign in again");
        }
    }

    fn show_login_dialog(&self, window: &HangarWindow) {
        self.show_login_dialog_with_notice(window, None);
    }

    /// Show the login dialog, with a reason when there is one.
    fn show_login_dialog_with_notice(&self, window: &HangarWindow, notice: Option<&str>) {
        let dialog = LoginDialog::new();

        let app = self.clone();
        let dialog_weak = dialog.downgrade();

        dialog.connect_login(move |dlg| {
            let handle = dlg.handle();
            let password = dlg.password();

            if handle.is_empty() || password.is_empty() {
                return;
            }

            dlg.set_loading(true);
            dlg.hide_error();

            // Get a channel for sending results back
            let (tx, rx) = std::sync::mpsc::channel::<Result<Session, String>>();

            let client = app.client();
            thread::spawn(move || {
                // Use the shared runtime for network operations so the HTTP client
                // context is consistent across all API calls
                let result = runtime::block_on(async {
                    let session = client
                        .login(&handle, &password)
                        .await
                        .map_err(|e| e.to_string())?;
                    // Store session in a separate task to not block login
                    // SecretService can be slow, so we fire and forget
                    let session_for_store = session.clone();
                    tokio::spawn(async move {
                        if let Err(e) = SessionManager::store(&session_for_store).await {
                            // Sign-in itself succeeded, so the window opens as
                            // normal and the failure only shows up as being
                            // logged out on next launch. Say so plainly.
                            eprintln!(
                                "hangar: signed in, but the session could not be saved \
                                 and you will have to sign in again next launch.\n  {}",
                                e
                            );
                        }
                    });
                    Ok(session)
                });
                let _ = tx.send(result);
            });

            // Poll for results on GTK main thread
            let app = app.clone();
            let dialog_weak = dialog_weak.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(Ok(session)) => {
                        println!("Logged in as: {} ({})", session.handle, session.did);

                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.close();
                        }

                        // Initialize cache for this user
                        match CacheDb::open(&session.did) {
                            Ok(cache) => {
                                // Clean up stale entries in background to not block UI
                                let cache_for_cleanup = cache.clone();
                                std::thread::spawn(move || {
                                    if let Err(e) = cache_for_cleanup.cleanup_stale() {
                                        eprintln!("Cache cleanup failed: {}", e);
                                    }
                                });
                                // Initialize image cache with database reference
                                avatar_cache::init(Arc::new(cache.clone()));
                                avatar_cache::cleanup_cache();
                                app.imp().cache.replace(Some(cache));
                            }
                            Err(e) => {
                                eprintln!("Failed to open cache: {}", e);
                            }
                        }

                        // Fetch user profile for sidebar avatar
                        app.fetch_user_profile(&session.did);

                        // Fetch saved feeds for feed selector
                        app.fetch_saved_feeds();

                        // Fetch timeline
                        app.fetch_timeline();
                        glib::ControlFlow::Break
                    }
                    Ok(Err(e)) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.show_error(&format!("Login failed: {}", e));
                        }
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        // Still waiting
                        glib::ControlFlow::Continue
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.show_error("Login failed: connection lost");
                        }
                        glib::ControlFlow::Break
                    }
                }
            });
        });

        // OAuth login handler
        let app2 = self.clone();
        let dialog_weak2 = dialog.downgrade();

        dialog.connect_oauth_login(move |dlg| {
            let handle = dlg.handle();
            if handle.is_empty() {
                return;
            }

            dlg.set_oauth_waiting(true);
            dlg.hide_error();

            let (tx, rx) = std::sync::mpsc::channel::<Result<Session, String>>();
            let hangar_client = app2.client();

            thread::spawn(move || {
                let result = runtime::block_on(async {
                    // Start authorization: bind the callback server and create the OAuth client.
                    let session_store = crate::state::session_store::FileSessionStore::new();
                    let (auth_url, oauth_client, callback_rx) =
                        OAuthManager::start_auth(&handle, session_store)
                            .await
                            .map_err(|e| format!("Authorization failed: {e}"))?;

                    // Open browser for user to authenticate
                    open::that(&auth_url).map_err(|e| format!("Failed to open browser: {e}"))?;

                    // Wait for callback from browser (blocking on this thread)
                    let callback_params = callback_rx
                        .recv_timeout(std::time::Duration::from_secs(300))
                        .map_err(|_| "Sign-in timed out. Please try again.".to_string())?;

                    // Exchange code for session
                    let oauth_session = OAuthManager::complete_auth(&oauth_client, callback_params)
                        .await
                        .map_err(|e| format!("Authentication failed: {e}"))?;

                    // Set the OAuth session on the client
                    let session = hangar_client.set_oauth_session(oauth_session).await;

                    // Store session in keyring
                    let session_for_store = session.clone();
                    tokio::spawn(async move {
                        if let Err(e) = SessionManager::store(&session_for_store).await {
                            // Sign-in itself succeeded, so the window opens as
                            // normal and the failure only shows up as being
                            // logged out on next launch. Say so plainly.
                            eprintln!(
                                "hangar: signed in, but the session could not be saved \
                                 and you will have to sign in again next launch.\n  {}",
                                e
                            );
                        }
                    });

                    Ok(session)
                });
                let _ = tx.send(result);
            });

            // Poll for results on GTK main thread
            let app = app2.clone();
            let dialog_weak = dialog_weak2.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(Ok(session)) => {
                        println!("OAuth login success: {}", session.did);

                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.close();
                        }

                        // Initialize cache for this user
                        match CacheDb::open(&session.did) {
                            Ok(cache) => {
                                let cache_for_cleanup = cache.clone();
                                std::thread::spawn(move || {
                                    if let Err(e) = cache_for_cleanup.cleanup_stale() {
                                        eprintln!("Cache cleanup failed: {}", e);
                                    }
                                });
                                avatar_cache::init(Arc::new(cache.clone()));
                                avatar_cache::cleanup_cache();
                                app.imp().cache.replace(Some(cache));
                            }
                            Err(e) => {
                                eprintln!("Failed to open cache: {}", e);
                            }
                        }

                        app.fetch_user_profile(&session.did);
                        app.fetch_saved_feeds();
                        app.fetch_timeline();
                        glib::ControlFlow::Break
                    }
                    Ok(Err(e)) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_oauth_waiting(false);
                            dialog.show_error(&format!("Login failed: {}", e));
                        }
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_oauth_waiting(false);
                            dialog.show_error("Login failed: connection lost");
                        }
                        glib::ControlFlow::Break
                    }
                }
            });
        });

        // OAuth cancel handler. Resets the UI to its initial state.
        dialog.connect_oauth_cancel(move |dlg| {
            dlg.set_oauth_waiting(false);
        });

        if let Some(notice) = notice {
            dialog.show_error(notice);
        }

        dialog.present(Some(window));
    }

    fn fetch_user_profile(&self, did: &str) {
        // Store the user DID for later use
        self.imp().user_did.replace(Some(did.to_string()));
        // Rows check post authorship against this before offering Delete
        crate::ui::post_row::set_current_user_did(Some(did));
        // People rows hide the follow button on your own row
        crate::ui::actor_row::set_viewer_did(Some(did));

        // Every path here follows a successful sign-in, so the polls a dead
        // session parked may run again.
        self.imp().session_dead.replace(false);

        // Badges should not wait out the first 30-second tick.
        self.check_unread_counts();

        // Try cache first for instant display
        let mut skip_fetch = false;
        if let Some(cache) = self.imp().cache.borrow().as_ref() {
            let profile_cache = ProfileCache::new(cache);
            if let Ok(cached_profile) = profile_cache.get(did) {
                if let Some(window) = self.imp().window.borrow().as_ref() {
                    let display_name = cached_profile
                        .display_name
                        .as_deref()
                        .unwrap_or(&cached_profile.handle);
                    window.set_user_avatar(display_name, cached_profile.avatar.as_deref());
                    window.set_current_user_did(did);
                    window.update_profile_header(&cached_profile);
                }
                // Skip network fetch if cache is fresh (< 5 minutes)
                if profile_cache.has_fresh_full(did, 300).unwrap_or(false) {
                    skip_fetch = true;
                }
            }
        }

        // Don't fetch if we have fresh data
        if skip_fetch {
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel::<Result<Profile, String>>();

        let client = self.client();
        let did = did.to_string();
        let did_for_window = did.clone();
        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_profile(&did).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(profile)) => {
                    // Store in cache
                    if let Some(cache) = app.imp().cache.borrow().as_ref() {
                        let profile_cache = ProfileCache::new(cache);
                        let _ = profile_cache.store_full(&profile);
                    }

                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        let display_name =
                            profile.display_name.as_deref().unwrap_or(&profile.handle);
                        window.set_user_avatar(display_name, profile.avatar.as_deref());
                        // Set current user DID for filtering conversations
                        window.set_current_user_did(&did_for_window);
                        // Update the profile page header
                        window.update_profile_header(&profile);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch profile: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch profile: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn fetch_timeline(&self) {
        // Try cache first for instant display
        if let Some(cache) = self.imp().cache.borrow().as_ref() {
            let feed_cache = FeedCache::new(cache);
            if let Ok(cached_posts) = feed_cache.get_page("home", 0, 50) {
                if !cached_posts.is_empty() {
                    // Restore feed state from cache
                    if let Ok(state) = feed_cache.get_state("home") {
                        self.imp()
                            .timeline_cursor
                            .replace(state.oldest_cursor.clone());
                        self.imp()
                            .newest_post_uri
                            .replace(state.newest_post_uri.clone());
                    }
                    if let Some(window) = self.imp().window.borrow().as_ref() {
                        window.set_posts(cached_posts);
                    }
                    // Continue to fetch fresh data in background
                }
            }
        }

        // With no cached page shown, the wait gets skeleton rows.
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_timeline_loading(true);
        }

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();

        let client = self.client();
        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_timeline(None).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((posts, next_cursor))) => {
                    app.imp().timeline_cursor.replace(next_cursor.clone());
                    // Set anchor to the newest post
                    if let Some(first) = posts.first() {
                        app.imp().newest_post_uri.replace(Some(first.uri.clone()));
                    }
                    // Clear any pending new posts since we just refreshed

                    // Store in cache
                    if let Some(cache) = app.imp().cache.borrow().as_ref() {
                        let post_cache = PostCache::new(cache);
                        let feed_cache = FeedCache::new(cache);
                        // Clear old feed items and store new ones
                        let _ = feed_cache.clear_feed("home");
                        let _ = post_cache.store_batch(&posts);
                        let _ = feed_cache.store_page("home", &posts, 0);
                        // Update feed state
                        let state = FeedState {
                            oldest_cursor: next_cursor,
                            has_more: true,
                            newest_post_uri: posts.first().map(|p| p.uri.clone()),
                            newest_sort_timestamp: posts.first().map(|p| p.indexed_at.clone()),
                            last_refresh_at: Some(CacheDb::now()),
                        };
                        let _ = feed_cache.set_state("home", &state);
                    }

                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.hide_new_posts_banner();
                        window.set_posts(posts);
                    }
                    // Start background polling for new posts
                    app.start_new_posts_polling();
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch timeline: {}", e);
                    app.toast_unless_offline("Couldn't load the feed");
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_timeline_load_failed();
                    }
                    app.report_session_expiry();
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch timeline: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn toggle_like(&self, post: &Post, post_row_weak: glib::WeakRef<PostRow>) {
        // Result type: Ok(Some(uri)) for like success, Ok(None) for unlike success, Err for failure
        let (tx, rx) = std::sync::mpsc::channel::<Result<Option<String>, String>>();
        let client = self.client();

        // If already liked, this press unlikes.
        if let Some(like_uri) = &post.viewer_like {
            let like_uri = like_uri.clone();
            thread::spawn(move || {
                let result = runtime::block_on(async { client.unlike(&like_uri).await });
                let _ = tx.send(result.map(|_| None).map_err(|e| e.to_string()));
            });
        } else {
            // Not liked yet, create like
            let uri = post.uri.clone();
            let cid = post.cid.clone();
            thread::spawn(move || {
                let result = runtime::block_on(async { client.like(&uri, &cid).await });
                let _ = tx.send(result.map(Some).map_err(|e| e.to_string()));
            });
        }

        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(new_like_uri)) => {
                    // Update the PostRow's like URI state if it still exists
                    if let Some(post_row) = post_row_weak.upgrade() {
                        post_row.set_viewer_like_uri(new_like_uri);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Like/unlike failed: {}", e);
                    // TODO: Revert visual state on failure
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Ask before deleting; there is no undo.
    fn confirm_delete_post(&self, post: Post) {
        let window = match self.imp().window.borrow().as_ref() {
            Some(w) => w.clone(),
            None => return,
        };

        let dialog = adw::AlertDialog::new(Some("Delete post?"), Some("This can't be undone."));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let app = self.clone();
        dialog.connect_response(Some("delete"), move |_, _| {
            app.delete_post(post.clone());
        });
        dialog.present(Some(&window));
    }

    fn delete_post(&self, post: Post) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let client = self.client();
        let uri = post.uri.clone();
        thread::spawn(move || {
            let result = runtime::block_on(async { client.delete_post(&uri).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        let uri = post.uri;
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.remove_post(&uri);
                        window.show_toast("Post deleted");
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to delete post: {}", e);
                    app.report_session_expiry();
                    // The post stays where it was; nothing was removed early.
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Couldn't delete post");
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Wire up mention typeahead search on a compose dialog.
    fn setup_link_card_fetch(&self, dialog: &ComposeDialog) {
        let dialog_weak = dialog.downgrade();

        dialog.connect_link_preview_fetch(move |url| {
            let dialog_weak = dialog_weak.clone();
            let url = url.clone();

            let (tx, rx) =
                std::sync::mpsc::channel::<Result<crate::atproto::LinkCardData, String>>();

            thread::spawn(move || {
                let result = runtime::block_on(async {
                    crate::atproto::client::fetch_link_card_meta(&url).await
                });
                let _ = tx.send(result.map_err(|e| e.to_string()));
            });

            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(Ok(data)) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_link_card_data(data);
                        }
                        glib::ControlFlow::Break
                    }
                    Ok(Err(_)) => glib::ControlFlow::Break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                }
            });
        });
    }

    /// Give a compose dialog its video upload flow. The dialog hands
    /// over bytes at attach time; state comes back through its video_*
    /// methods, addressed by token so slot shuffles cannot misroute it.
    fn setup_video_upload(&self, dialog: &ComposeDialog) {
        let app = self.clone();
        let dialog_weak = dialog.downgrade();
        dialog.set_video_upload_callback(move |token, data, mime, sent, cancelled| {
            enum Msg {
                Event(crate::atproto::client::VideoUploadEvent),
                Done(serde_json::Value),
                Failed(String),
            }

            let (tx, rx) = std::sync::mpsc::channel::<Msg>();
            let client = app.client();
            let progress_tx = tx.clone();
            thread::spawn(move || {
                let result = runtime::block_on(async {
                    client
                        .upload_video(data, &mime, sent, cancelled, |event| {
                            let _ = progress_tx.send(Msg::Event(event));
                        })
                        .await
                });
                let _ = tx.send(match result {
                    Ok(blob) => Msg::Done(blob),
                    Err(e) => Msg::Failed(e.to_string()),
                });
            });

            let app = app.clone();
            let dialog_weak = dialog_weak.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                let Some(dialog) = dialog_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                loop {
                    match rx.try_recv() {
                        Ok(Msg::Event(event)) => {
                            use crate::atproto::client::VideoUploadEvent;
                            match event {
                                VideoUploadEvent::UploadDone => {
                                    dialog.video_processing(token, None)
                                }
                                VideoUploadEvent::Processing(pct) => {
                                    dialog.video_processing(token, pct)
                                }
                            }
                        }
                        Ok(Msg::Done(blob)) => {
                            dialog.video_ready(token, blob);
                            return glib::ControlFlow::Break;
                        }
                        Ok(Msg::Failed(e)) => {
                            eprintln!("Video upload failed: {}", e);
                            app.report_session_expiry();
                            dialog.video_failed(token, &e);
                            return glib::ControlFlow::Break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            return glib::ControlFlow::Continue;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            return glib::ControlFlow::Break;
                        }
                    }
                }
            });
        });
    }

    fn setup_mention_search(&self, dialog: &ComposeDialog) {
        let client = self.client();
        let dialog_weak = dialog.downgrade();

        dialog.connect_mention_search(move |query| {
            let client = client.clone();
            let query = query.clone();
            let dialog_weak = dialog_weak.clone();

            let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<Profile>, String>>();

            thread::spawn(move || {
                let result =
                    runtime::block_on(async { client.search_actors_typeahead(&query, 6).await });
                let _ = tx.send(result.map_err(|e| e.to_string()));
            });

            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(Ok(profiles)) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_mention_results(profiles);
                        }
                        glib::ControlFlow::Break
                    }
                    Ok(Err(_)) => glib::ControlFlow::Break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                }
            });
        });
    }

    fn open_compose_dialog(&self) {
        let window = match self.imp().window.borrow().as_ref() {
            Some(w) => w.clone(),
            None => return,
        };
        let dialog = ComposeDialog::new();

        let app = self.clone();
        let dialog_weak = dialog.downgrade();

        dialog.connect_post(move |data| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.set_loading(true);
                dialog.hide_error();
            }

            let app = app.clone();
            let dialog_weak = dialog_weak.clone();

            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let client = app.client();

            thread::spawn(move || {
                let result = runtime::block_on(async {
                    client.create_post_with_data(&data, None, None).await
                });
                let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string()));
            });

            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.close();
                        }
                        app.fetch_timeline();
                        glib::ControlFlow::Break
                    }
                    Ok(Err(e)) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.show_error(&e);
                        }
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                        }
                        glib::ControlFlow::Break
                    }
                }
            });
        });

        // Thread callback
        let app2 = self.clone();
        let dialog_weak2 = dialog.downgrade();
        dialog.connect_thread(move |posts| {
            if let Some(dialog) = dialog_weak2.upgrade() {
                dialog.set_loading(true);
                dialog.hide_error();
            }

            let app = app2.clone();
            let dialog_weak = dialog_weak2.clone();

            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let client = app.client();

            thread::spawn(move || {
                let result = runtime::block_on(async { client.create_thread(&posts, None).await });
                let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string()));
            });

            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.close();
                        }
                        app.fetch_timeline();
                        glib::ControlFlow::Break
                    }
                    Ok(Err(e)) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.show_error(&e);
                        }
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                        }
                        glib::ControlFlow::Break
                    }
                }
            });
        });

        self.setup_mention_search(&dialog);
        self.setup_video_upload(&dialog);
        self.setup_link_card_fetch(&dialog);
        dialog.present(Some(&window));
        dialog.focus_composer();
    }

    fn open_reply_dialog(&self, parent_post: Post) {
        let window = match self.imp().window.borrow().as_ref() {
            Some(w) => w.clone(),
            None => return,
        };

        let context = ReplyContext {
            uri: parent_post.uri.clone(),
            cid: parent_post.cid.clone(),
            author_handle: parent_post.author.handle.clone(),
        };

        let dialog = ComposeDialog::new_reply(context);

        let app = self.clone();
        let dialog_weak = dialog.downgrade();

        dialog.connect_reply(move |data, parent_uri, parent_cid| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.set_loading(true);
                dialog.hide_error();
            }

            let app = app.clone();
            let dialog_weak = dialog_weak.clone();

            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let client = app.client();

            thread::spawn(move || {
                let result = runtime::block_on(async {
                    let reply = crate::atproto::ReplyRef {
                        root_uri: parent_uri.clone(),
                        root_cid: parent_cid.clone(),
                        parent_uri,
                        parent_cid,
                    };
                    client.create_post_with_data(&data, Some(reply), None).await
                });
                let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string()));
            });

            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.close();
                        }
                        app.fetch_timeline();
                        glib::ControlFlow::Break
                    }
                    Ok(Err(e)) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.show_error(&e);
                        }
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                        }
                        glib::ControlFlow::Break
                    }
                }
            });
        });

        // Thread callback for replies
        let app2 = self.clone();
        let dialog_weak2 = dialog.downgrade();
        let reply_uri = parent_post.uri.clone();
        let reply_cid = parent_post.cid.clone();
        dialog.connect_thread(move |posts| {
            if let Some(dialog) = dialog_weak2.upgrade() {
                dialog.set_loading(true);
                dialog.hide_error();
            }

            let app = app2.clone();
            let dialog_weak = dialog_weak2.clone();
            let reply_uri = reply_uri.clone();
            let reply_cid = reply_cid.clone();

            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let client = app.client();

            thread::spawn(move || {
                let result = runtime::block_on(async {
                    let reply_ref = crate::atproto::ReplyRef {
                        root_uri: reply_uri.clone(),
                        root_cid: reply_cid.clone(),
                        parent_uri: reply_uri,
                        parent_cid: reply_cid,
                    };
                    client.create_thread(&posts, Some(reply_ref)).await
                });
                let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string()));
            });

            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.close();
                        }
                        app.fetch_timeline();
                        glib::ControlFlow::Break
                    }
                    Ok(Err(e)) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.show_error(&e);
                        }
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                        }
                        glib::ControlFlow::Break
                    }
                }
            });
        });

        self.setup_mention_search(&dialog);
        self.setup_video_upload(&dialog);
        self.setup_link_card_fetch(&dialog);
        dialog.present(Some(&window));
        dialog.focus_composer();
    }

    fn open_quote_dialog(&self, quoted_post: Post) {
        let window = match self.imp().window.borrow().as_ref() {
            Some(w) => w.clone(),
            None => return,
        };

        let context = QuoteContext {
            uri: quoted_post.uri.clone(),
            cid: quoted_post.cid.clone(),
            author_handle: quoted_post.author.handle.clone(),
            text: quoted_post.text.clone(),
        };

        let dialog = ComposeDialog::new_quote(context);

        let app = self.clone();
        let dialog_weak = dialog.downgrade();

        dialog.connect_quote(move |data, quoted_uri, quoted_cid| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.set_loading(true);
                dialog.hide_error();
            }

            let app = app.clone();
            let dialog_weak = dialog_weak.clone();

            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let client = app.client();

            thread::spawn(move || {
                let result = runtime::block_on(async {
                    client
                        .create_post_with_data(&data, None, Some((&quoted_uri, &quoted_cid)))
                        .await
                });
                let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string()));
            });

            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.close();
                        }
                        app.fetch_timeline();
                        glib::ControlFlow::Break
                    }
                    Ok(Err(e)) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.show_error(&e);
                        }
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                        }
                        glib::ControlFlow::Break
                    }
                }
            });
        });

        // Thread callback for quotes
        let app2 = self.clone();
        let dialog_weak2 = dialog.downgrade();
        dialog.connect_thread(move |posts| {
            if let Some(dialog) = dialog_weak2.upgrade() {
                dialog.set_loading(true);
                dialog.hide_error();
            }

            let app = app2.clone();
            let dialog_weak = dialog_weak2.clone();

            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let client = app.client();

            thread::spawn(move || {
                let result = runtime::block_on(async { client.create_thread(&posts, None).await });
                let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string()));
            });

            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.close();
                        }
                        app.fetch_timeline();
                        glib::ControlFlow::Break
                    }
                    Ok(Err(e)) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                            dialog.show_error(&e);
                        }
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(dialog) = dialog_weak.upgrade() {
                            dialog.set_loading(false);
                        }
                        glib::ControlFlow::Break
                    }
                }
            });
        });

        self.setup_mention_search(&dialog);
        self.setup_video_upload(&dialog);
        self.setup_link_card_fetch(&dialog);
        dialog.present(Some(&window));
        dialog.focus_composer();
    }

    fn toggle_repost(&self, post: &Post, post_row_weak: glib::WeakRef<PostRow>) {
        // Result type: Ok(Some(uri)) for repost success, Ok(None) for unrepost success, Err for failure
        let (tx, rx) = std::sync::mpsc::channel::<Result<Option<String>, String>>();
        let client = self.client();

        // If already reposted, this press deletes the repost.
        if let Some(repost_uri) = &post.viewer_repost {
            let repost_uri = repost_uri.clone();
            thread::spawn(move || {
                let result = runtime::block_on(async { client.delete_repost(&repost_uri).await });
                let _ = tx.send(result.map(|_| None).map_err(|e| e.to_string()));
            });
        } else {
            // Not reposted yet, create repost
            let uri = post.uri.clone();
            let cid = post.cid.clone();
            thread::spawn(move || {
                let result = runtime::block_on(async { client.repost(&uri, &cid).await });
                let _ = tx.send(result.map(Some).map_err(|e| e.to_string()));
            });
        }

        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(new_repost_uri)) => {
                    // Update the PostRow's repost URI state if it still exists
                    if let Some(post_row) = post_row_weak.upgrade() {
                        post_row.set_viewer_repost_uri(new_repost_uri);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Repost/unrepost failed: {}", e);
                    // TODO: Revert visual state on failure
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    pub fn fetch_timeline_more(&self) {
        if *self.imp().loading_more.borrow() {
            return;
        }
        let cursor = match self.imp().timeline_cursor.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };
        self.imp().loading_more.replace(true);

        // Show loading spinner
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_loading_more(true);
        }

        // Get current feed URI if not home
        let current_feed = self.imp().current_feed.borrow().clone();
        let feed_uri = current_feed.as_ref().and_then(|f| {
            if f.is_home() {
                None
            } else {
                Some(f.uri.clone())
            }
        });

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();
        let semaphore = API_SEMAPHORE.clone();
        thread::spawn(move || {
            let result = runtime::block_on(async {
                // Acquire permit to limit concurrent API requests
                let _permit = semaphore.acquire().await;
                match feed_uri {
                    Some(uri) => client.get_feed(&uri, Some(&cursor)).await,
                    None => client.get_timeline(Some(&cursor)).await,
                }
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((posts, next_cursor))) => {
                    app.imp().loading_more.replace(false);
                    app.imp().timeline_cursor.replace(next_cursor.clone());

                    // Store in cache (append to existing feed)
                    if let Some(cache) = app.imp().cache.borrow().as_ref() {
                        let current_feed = app.imp().current_feed.borrow();
                        let feed_key = current_feed
                            .as_ref()
                            .map(|f| {
                                if f.is_home() {
                                    "home".to_string()
                                } else {
                                    f.uri.clone()
                                }
                            })
                            .unwrap_or_else(|| "home".to_string());
                        drop(current_feed);

                        let post_cache = PostCache::new(cache);
                        let feed_cache = FeedCache::new(cache);
                        let start_pos = feed_cache.count(&feed_key).unwrap_or(0) as i64;
                        let _ = post_cache.store_batch(&posts);
                        let _ = feed_cache.store_page(&feed_key, &posts, start_pos);
                        // Update cursor in feed state
                        if let Ok(mut state) = feed_cache.get_state(&feed_key) {
                            state.oldest_cursor = next_cursor;
                            state.has_more = state.oldest_cursor.is_some();
                            let _ = feed_cache.set_state(&feed_key, &state);
                        }
                    }

                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_loading_more(false);
                        if !posts.is_empty() {
                            window.append_posts(posts);
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    app.imp().loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_loading_more(false);
                    }
                    eprintln!("Failed to fetch more timeline: {}", e);
                    app.toast_unless_offline("Couldn't load more posts");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.imp().loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_loading_more(false);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn start_new_posts_polling(&self) {
        // Only start polling once
        if *self.imp().polling_started.borrow() {
            return;
        }
        self.imp().polling_started.replace(true);

        let app = self.clone();
        // One timer drives every badge: new posts and unread counts alike
        glib::timeout_add_seconds_local(30, move || {
            app.check_for_new_posts();
            app.check_unread_counts();
            glib::ControlFlow::Continue
        });
    }

    fn check_for_new_posts(&self) {
        // Don't check if we're already checking
        if *self.imp().checking_new_posts.borrow() {
            return;
        }

        // A dead session would fail here every 30 seconds; wait for the
        // next sign-in instead.
        if *self.imp().session_dead.borrow() {
            return;
        }

        // Offline says why every request would fail; the banner covers it.
        if *self.imp().offline.borrow() {
            return;
        }

        // A backgrounded window would prepend into a feed nobody is
        // reading, every 30 seconds, for as long as it sits there; the
        // model and the image caches grow with every pass and the kernel
        // eventually ends it. Catch up on focus instead.
        let window_active = self
            .imp()
            .window
            .borrow()
            .as_ref()
            .is_some_and(|w| w.is_active());
        if !window_active {
            return;
        }

        // Only check if we have an anchor
        let anchor_uri = match self.imp().newest_post_uri.borrow().clone() {
            Some(uri) => uri,
            None => return,
        };

        self.imp().checking_new_posts.replace(true);

        // Get current feed URI if not home
        let current_feed = self.imp().current_feed.borrow().clone();
        let feed_uri = current_feed.as_ref().and_then(|f| {
            if f.is_home() {
                None
            } else {
                Some(f.uri.clone())
            }
        });

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                match feed_uri {
                    Some(uri) => client.get_feed(&uri, None).await,
                    None => client.get_timeline(None).await,
                }
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((posts, _))) => {
                    app.imp().checking_new_posts.replace(false);
                    app.imp().new_posts_poll_failed.replace(false);

                    // Find posts newer than our anchor
                    let new_posts: Vec<Post> = posts
                        .into_iter()
                        .take_while(|p| p.uri != anchor_uri)
                        .collect();

                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        // Always refresh timestamps on poll
                        window.refresh_timestamps();

                        if !new_posts.is_empty() {
                            let count = new_posts.len();

                            // Update anchor to newest post
                            if let Some(newest) = new_posts.first() {
                                app.imp().newest_post_uri.replace(Some(newest.uri.clone()));
                            }

                            // Insert new posts at the top of the timeline
                            // User can scroll up to see them
                            window.insert_posts_at_top(new_posts);

                            // Show banner to let user know there are new posts above
                            window.show_new_posts_banner(count);
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    app.imp().checking_new_posts.replace(false);
                    // A flaky upstream fails poll after poll; say so once.
                    if !*app.imp().new_posts_poll_failed.borrow() {
                        app.imp().new_posts_poll_failed.replace(true);
                        eprintln!("Failed to check for new posts: {}", e);
                    }
                    // The poll is the only thing talking to the server on an
                    // idle window, so a session dying mid-use shows up here
                    // first.
                    app.report_session_expiry();
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.imp().checking_new_posts.replace(false);
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn show_new_posts(&self) {
        // The posts are already in the model. Scroll to top to reveal them.
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.hide_new_posts_banner();
            window.scroll_to_top();
        }
    }

    /// Refresh the Mentions, Activity, and Chat badges from the server.
    ///
    /// Rides the same 30-second timer as `check_for_new_posts`: one thread,
    /// one round trip, three badges.
    fn check_unread_counts(&self) {
        let signed_in = self.imp().user_did.borrow().is_some();
        let in_flight = *self.imp().checking_unread.borrow();
        let expired = *self.imp().session_dead.borrow();
        if !unread_poll_allowed(signed_in, in_flight, expired) {
            return;
        }
        // Same offline guard as the new-posts poll.
        if *self.imp().offline.borrow() {
            return;
        }
        self.imp().checking_unread.replace(true);

        // A badge clear while this fetch is out makes its counts stale.
        let clear_token = self.imp().badge_clear_generation.token();

        let (tx, rx) = std::sync::mpsc::channel::<Result<UnreadCounts, String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_unread_counts().await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(counts)) => {
                    app.imp().checking_unread.replace(false);
                    app.imp().unread_poll_failed.replace(false);
                    if let Some(sidebar) = app.sidebar() {
                        sidebar.set_badge(NavItem::Chat, counts.chat);
                        // Opening a section may have cleared these two while
                        // the fetch was out; stale counts would relight them.
                        if app.imp().badge_clear_generation.is_current(clear_token) {
                            sidebar.set_badge(NavItem::Mentions, counts.mentions);
                            sidebar.set_badge(NavItem::Activity, counts.activity);
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    app.imp().checking_unread.replace(false);
                    // A flaky upstream fails poll after poll; say so once.
                    if !*app.imp().unread_poll_failed.borrow() {
                        app.imp().unread_poll_failed.replace(true);
                        eprintln!("Failed to check unread counts: {}", e);
                    }
                    // On expiry this parks the poll until the next sign-in.
                    app.report_session_expiry();
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.imp().checking_unread.replace(false);
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// The sidebar, while a window is up.
    fn sidebar(&self) -> Option<crate::ui::sidebar::Sidebar> {
        self.imp()
            .window
            .borrow()
            .as_ref()
            .and_then(|w| w.sidebar())
    }

    /// What `item`'s badge currently holds.
    fn badge_count(&self, item: NavItem) -> u32 {
        self.sidebar().map(|s| s.badge_count(item)).unwrap_or(0)
    }

    /// Clear both notification badges and tell the server.
    ///
    /// `seenAt` is account-wide, so opening either Mentions or Activity
    /// marks everything seen. The badges clear together to say so honestly.
    fn clear_notification_badges(&self) {
        if self.badge_count(NavItem::Mentions) == 0 && self.badge_count(NavItem::Activity) == 0 {
            return;
        }
        // Strand any unread poll already in flight; it counted before this
        // clear and would relight the badges.
        self.imp().badge_clear_generation.bump();
        if let Some(sidebar) = self.sidebar() {
            sidebar.set_badge(NavItem::Mentions, 0);
            sidebar.set_badge(NavItem::Activity, 0);
        }
        // Fire and forget. If it fails, the next poll raises the badges again.
        let client = self.client();
        thread::spawn(move || {
            if let Err(e) = runtime::block_on(async { client.update_notifications_seen().await }) {
                eprintln!("Failed to mark notifications seen: {}", e);
            }
        });
    }

    /// Fetch the user's saved feeds and populate the feed selector
    fn fetch_saved_feeds(&self) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<SavedFeed>, String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_saved_feeds().await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(feeds)) => {
                    // Set default feed if not already set
                    if app.imp().current_feed.borrow().is_none() {
                        if let Some(first) = feeds.first() {
                            app.imp().current_feed.replace(Some(first.clone()));
                        }
                    }
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        // Set the current feed name/uri first so checkmark shows correctly
                        if let Some(current) = app.imp().current_feed.borrow().as_ref() {
                            window.set_current_feed_name(&current.display_name, &current.uri);
                        }
                        window.set_saved_feeds(feeds);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch saved feeds: {}", e);
                    // Still set up a default home feed
                    let home_feed = vec![SavedFeed::home()];
                    app.imp().current_feed.replace(Some(SavedFeed::home()));
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_saved_feeds(home_feed);
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch saved feeds: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Switch to a different feed
    fn switch_feed(&self, feed: SavedFeed) {
        // Update current feed
        let feed_name = feed.display_name.clone();
        let feed_uri = feed.uri.clone();
        self.imp().current_feed.replace(Some(feed.clone()));

        // Update UI
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_current_feed_name(&feed_name, &feed_uri);
            window.hide_new_posts_banner();
        }

        // Clear state
        self.imp().timeline_cursor.replace(None);
        self.imp().newest_post_uri.replace(None);

        // Fetch the new feed
        self.fetch_current_feed();
    }

    /// Fetch posts for the current feed (home timeline or custom feed)
    fn fetch_current_feed(&self) {
        let current_feed = self.imp().current_feed.borrow().clone();
        let feed_key = current_feed
            .as_ref()
            .map(|f| {
                if f.is_home() {
                    "home".to_string()
                } else {
                    f.uri.clone()
                }
            })
            .unwrap_or_else(|| "home".to_string());
        let feed_uri = current_feed.as_ref().and_then(|f| {
            if f.is_home() {
                None
            } else {
                Some(f.uri.clone())
            }
        });

        // Try cache first for instant display
        if let Some(cache) = self.imp().cache.borrow().as_ref() {
            let feed_cache = FeedCache::new(cache);
            if let Ok(cached_posts) = feed_cache.get_page(&feed_key, 0, 50) {
                if !cached_posts.is_empty() {
                    // Restore feed state from cache
                    if let Ok(state) = feed_cache.get_state(&feed_key) {
                        self.imp()
                            .timeline_cursor
                            .replace(state.oldest_cursor.clone());
                        self.imp()
                            .newest_post_uri
                            .replace(state.newest_post_uri.clone());
                    }
                    if let Some(window) = self.imp().window.borrow().as_ref() {
                        window.set_posts(cached_posts);
                    }
                }
            }
        }

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();
        let feed_uri_clone = feed_uri.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                match feed_uri_clone {
                    Some(uri) => client.get_feed(&uri, None).await,
                    None => client.get_timeline(None).await,
                }
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        let feed_key_for_cache = feed_key.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((posts, next_cursor))) => {
                    app.imp().timeline_cursor.replace(next_cursor.clone());
                    if let Some(first) = posts.first() {
                        app.imp().newest_post_uri.replace(Some(first.uri.clone()));
                    }

                    // Store in cache
                    if let Some(cache) = app.imp().cache.borrow().as_ref() {
                        let post_cache = PostCache::new(cache);
                        let feed_cache = FeedCache::new(cache);
                        let _ = feed_cache.clear_feed(&feed_key_for_cache);
                        let _ = post_cache.store_batch(&posts);
                        let _ = feed_cache.store_page(&feed_key_for_cache, &posts, 0);
                        let state = FeedState {
                            oldest_cursor: next_cursor,
                            has_more: true,
                            newest_post_uri: posts.first().map(|p| p.uri.clone()),
                            newest_sort_timestamp: posts.first().map(|p| p.indexed_at.clone()),
                            last_refresh_at: Some(CacheDb::now()),
                        };
                        let _ = feed_cache.set_state(&feed_key_for_cache, &state);
                    }

                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.hide_new_posts_banner();
                        window.set_posts(posts);
                    }
                    app.start_new_posts_polling();
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch feed: {}", e);
                    app.toast_unless_offline("Couldn't load this feed");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch feed: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Open the thread view for a post
    fn open_thread_view(&self, post: Post) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<Post>, String>>();
        let client = self.client();
        let post_uri = post.uri.clone();
        let main_post = post.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_thread(&post_uri).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(posts)) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.push_thread_page(&main_post, posts);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch thread: {}", e);
                    app.toast_unless_offline("Couldn't load the thread");
                    // Clicking a quote of a deleted post lands here. Silence
                    // read as a dead click.
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Couldn't load this post");
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch thread: connection lost");
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Couldn't load this post");
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Open the profile view for a user by their handle (e.g., from @mention click)
    fn open_profile_by_handle(&self, handle: String) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Profile, String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_profile(&handle).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(profile)) => {
                    // Now open the profile view with the resolved profile
                    app.open_profile_view(profile);
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch profile for handle: {}", e);
                    app.toast_unless_offline("Couldn't open the profile");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch profile: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Open the profile view for a user
    fn open_profile_view(&self, profile: Profile) {
        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Profile, Vec<Post>, Option<String>), String>>();
        let client = self.client();
        let actor = profile.did.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                let feed = client
                    .get_author_feed(&actor, None, Some("posts_and_author_threads"))
                    .await;
                // Avatar clicks arrive with the post author, a minimal
                // profile missing its bio, and search results come without
                // counts. Fetch the full one so the pushed page can show
                // both; if that fails, push what we had.
                let full_profile =
                    if profile.description.is_none() || profile.followers_count.is_none() {
                        client.get_profile(&actor).await.ok()
                    } else {
                        None
                    };
                feed.map(|(posts, cursor)| (full_profile.unwrap_or(profile), posts, cursor))
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((profile, posts, cursor))) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.push_profile_page(&profile, posts, cursor);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch profile feed: {}", e);
                    app.toast_unless_offline("Couldn't open the profile");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch profile feed: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Open the followers or following list for a profile
    fn open_follow_list(&self, profile: Profile, kind: FollowListKind) {
        let pushed = {
            let window = self.imp().window.borrow();
            let Some(window) = window.as_ref() else {
                return;
            };
            window.push_follow_list_page(&profile, kind)
        };
        let Some(pushed) = pushed else {
            return;
        };

        let page = match pushed {
            FollowListPush::Pushed(page) => {
                let app = self.clone();
                let did = profile.did.clone();
                let page_weak = page.downgrade();
                page.set_load_more_callback(move || {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    // No cursor means either the first page is still in
                    // flight or the server said the list is done.
                    let Some(cursor) = page.cursor() else {
                        return;
                    };
                    app.fetch_follow_list(&page, did.clone(), kind, Some(cursor));
                });

                // Retry resumes from the stored cursor: a failed first page
                // starts over, a failed later page picks up where it was.
                let app = self.clone();
                let did = profile.did.clone();
                let page_weak = page.downgrade();
                page.set_retry_callback(move || {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    app.fetch_follow_list(&page, did.clone(), kind, page.cursor());
                });

                page
            }
            FollowListPush::PoppedBack(page) => {
                // A healthy page keeps what it has. One whose first load
                // failed gets another try on reopen.
                if !page.needs_reload() {
                    return;
                }
                page
            }
        };

        self.fetch_follow_list(&page, profile.did.clone(), kind, page.cursor());
    }

    /// Fetch one page of a follow list and hand it to the pushed page.
    ///
    /// The page owns its cursor and in-flight flag; stacked lists must not
    /// share state, and a popped page takes its fetch bookkeeping with it.
    fn fetch_follow_list(
        &self,
        page: &FollowListPage,
        did: String,
        kind: FollowListKind,
        cursor: Option<String>,
    ) {
        if page.is_fetching() {
            return;
        }
        page.set_fetching(true);

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Profile>, Option<String>), String>>();
        let client = self.client();
        let semaphore = API_SEMAPHORE.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                let _permit = semaphore.acquire().await;
                match kind {
                    FollowListKind::Followers => {
                        client.get_followers(&did, cursor.as_deref()).await
                    }
                    FollowListKind::Following => client.get_follows(&did, cursor.as_deref()).await,
                }
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        let page_weak = page.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((profiles, next_cursor))) => {
                    if let Some(page) = page_weak.upgrade() {
                        page.set_cursor(next_cursor);
                        page.append_profiles(profiles);
                        page.set_fetching(false);
                        // Filtered or short pages can leave the viewport
                        // unfilled with a cursor still stored.
                        page.backfill_if_short();
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch follow list: {}", e);
                    if let Some(page) = page_weak.upgrade() {
                        page.set_fetching(false);
                        page.show_load_failed();
                    }
                    app.report_session_expiry();
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch follow list: connection lost");
                    if let Some(page) = page_weak.upgrade() {
                        page.set_fetching(false);
                        page.show_load_failed();
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Handle navigation item changes from the sidebar
    fn handle_nav_change(&self, item: NavItem) {
        match item {
            NavItem::Home => {
                if let Some(window) = self.imp().window.borrow().as_ref() {
                    window.show_home_page();
                }
            }
            NavItem::Mentions => {
                self.open_mentions_view();
            }
            NavItem::Activity => {
                self.open_activity_view();
            }
            NavItem::Chat => {
                self.open_chat_view();
            }
            NavItem::Profile => {
                self.open_own_profile_view();
            }
            NavItem::Likes => {
                self.open_likes_view();
            }
            NavItem::Bookmarks => {
                self.open_bookmarks_view();
            }
            NavItem::Search => {
                self.open_search_view();
            }
        }
    }

    /// Open the mentions view
    fn open_mentions_view(&self) {
        // Switch to mentions page (instant, no animation)
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.show_mentions_page();
        }

        // A lit badge means the server has mentions this list does not.
        let stale = self.badge_count(NavItem::Mentions) > 0;
        self.clear_notification_badges();

        // Fetch on first open, or when the badge said the list moved
        if self.imp().mentions_cursor.borrow().is_none() || stale {
            self.fetch_mentions();
        }
    }

    /// Fetch mentions (notifications filtered to mentions/replies/quotes)
    fn fetch_mentions(&self) {
        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<Notification>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_notifications(None, true).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((notifications, next_cursor))) => {
                    app.imp().mentions_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_mentions(notifications);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch mentions: {}", e);
                    app.toast_unless_offline("Couldn't load mentions");
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_mentions_load_failed();
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch mentions: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Fetch more mentions for infinite scroll
    fn fetch_mentions_more(&self) {
        if *self.imp().mentions_loading_more.borrow() {
            return;
        }
        let cursor = match self.imp().mentions_cursor.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };
        self.imp().mentions_loading_more.replace(true);

        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_mentions_loading(true);
        }

        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<Notification>, Option<String>), String>>();
        let client = self.client();
        let semaphore = API_SEMAPHORE.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                let _permit = semaphore.acquire().await;
                client.get_notifications(Some(&cursor), true).await
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((notifications, next_cursor))) => {
                    app.imp().mentions_loading_more.replace(false);
                    app.imp().mentions_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_mentions_loading(false);
                        if !notifications.is_empty() {
                            window.append_mentions(notifications);
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    app.imp().mentions_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_mentions_loading(false);
                    }
                    eprintln!("Failed to fetch more mentions: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.imp().mentions_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_mentions_loading(false);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Open the activity view
    fn open_activity_view(&self) {
        // Switch to activity page (instant, no animation)
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.show_activity_page();
        }

        // Activity lists every notification, so unread under either badge
        // means this list is behind.
        let stale =
            self.badge_count(NavItem::Mentions) > 0 || self.badge_count(NavItem::Activity) > 0;
        self.clear_notification_badges();

        // Fetch on first open, or when the badge said the list moved
        if self.imp().activity_cursor.borrow().is_none() || stale {
            self.fetch_activity();
        }
    }

    /// Fetch activity (all notifications)
    fn fetch_activity(&self) {
        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<Notification>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            // Pass false for mentions_only to get all notifications
            let result = runtime::block_on(async { client.get_notifications(None, false).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((notifications, next_cursor))) => {
                    app.imp().activity_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_activity(notifications);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch activity: {}", e);
                    app.toast_unless_offline("Couldn't load activity");
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_activity_load_failed();
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch activity: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Fetch more activity for infinite scroll
    fn fetch_activity_more(&self) {
        if *self.imp().activity_loading_more.borrow() {
            return;
        }
        let cursor = match self.imp().activity_cursor.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };
        self.imp().activity_loading_more.replace(true);

        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_activity_loading(true);
        }

        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<Notification>, Option<String>), String>>();
        let client = self.client();
        let semaphore = API_SEMAPHORE.clone();

        thread::spawn(move || {
            // Pass false for mentions_only to get all notifications
            let result = runtime::block_on(async {
                let _permit = semaphore.acquire().await;
                client.get_notifications(Some(&cursor), false).await
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((notifications, next_cursor))) => {
                    app.imp().activity_loading_more.replace(false);
                    app.imp().activity_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_activity_loading(false);
                        if !notifications.is_empty() {
                            window.append_activity(notifications);
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    app.imp().activity_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_activity_loading(false);
                    }
                    eprintln!("Failed to fetch more activity: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.imp().activity_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_activity_loading(false);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Open the chat view
    fn open_chat_view(&self) {
        // Switch to chat page (instant, no animation)
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.show_chat_page();
        }

        // The chat badge stays lit until a conversation is actually read;
        // clearing it from the list would claim messages nobody has seen.
        // The list itself still refreshes when the badge says it moved.
        let stale = self.badge_count(NavItem::Chat) > 0;
        if self.imp().chat_cursor.borrow().is_none() || stale {
            self.fetch_conversations();
        }
    }

    /// Fetch conversations
    fn fetch_conversations(&self) {
        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<Conversation>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_conversations(None).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((conversations, next_cursor))) => {
                    app.imp().chat_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_conversations(conversations);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch conversations: {}", e);
                    app.toast_unless_offline("Couldn't load conversations");
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_conversations_load_failed();
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch conversations: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Fetch more conversations for infinite scroll
    fn fetch_chat_more(&self) {
        if *self.imp().chat_loading_more.borrow() {
            return;
        }
        let cursor = match self.imp().chat_cursor.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };
        self.imp().chat_loading_more.replace(true);

        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_chat_loading(true);
        }

        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<Conversation>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_conversations(Some(&cursor)).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((conversations, next_cursor))) => {
                    app.imp().chat_loading_more.replace(false);
                    app.imp().chat_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_chat_loading(false);
                        if !conversations.is_empty() {
                            window.append_conversations(conversations);
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    app.imp().chat_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_chat_loading(false);
                    }
                    eprintln!("Failed to fetch more conversations: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.imp().chat_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_chat_loading(false);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Open a conversation in the message view, pushed onto the chat stack.
    /// Message someone, from a profile button or the New Message picker.
    /// The server decides whether messaging is allowed; DMs off, blocks,
    /// and account restrictions all come back as a plain no, so the toast
    /// says so without guessing the reason. The button, when there is one,
    /// is already locked by the click.
    fn open_direct_message(&self, profile: Profile, btn_weak: Option<glib::WeakRef<gtk4::Button>>) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Option<Conversation>, String>>();
        let client = self.client();
        let did = profile.did.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.start_conversation(&did).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            let btn = btn_weak.as_ref().and_then(|weak| weak.upgrade());
            match rx.try_recv() {
                Ok(Ok(Some(conversation))) => {
                    if let Some(btn) = btn {
                        btn.set_sensitive(true);
                    }
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        // The message page lives on the chat stack; go there
                        // with the rail highlight in tow.
                        window.select_nav(NavItem::Chat);
                        window.switch_to_page("chat");
                    }
                    app.open_conversation_view(conversation);
                    glib::ControlFlow::Break
                }
                Ok(Ok(None)) => {
                    // Stays locked: asking again will not change the answer.
                    if let Some(btn) = btn {
                        btn.set_tooltip_text(Some("This account can't receive your messages"));
                    }
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("This account can't receive your messages");
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to check messaging: {}", e);
                    app.report_session_expiry();
                    if let Some(btn) = btn {
                        btn.set_sensitive(true);
                    }
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Couldn't open messaging");
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Typeahead for the New Message picker. The generation strands slow
    /// answers so a fast typist never sees results for an older query.
    fn search_message_targets(&self, query: String) {
        self.imp().message_target_generation.bump();
        let generation = self.imp().message_target_generation.token();

        if query.trim().is_empty() {
            if let Some(window) = self.imp().window.borrow().as_ref() {
                window.set_new_message_results(vec![]);
            }
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<Profile>, String>>();
        let client = self.client();
        thread::spawn(move || {
            let result =
                runtime::block_on(async { client.search_actors_typeahead(&query, 8).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if !app.imp().message_target_generation.is_current(generation) {
                return glib::ControlFlow::Break;
            }
            match rx.try_recv() {
                Ok(Ok(profiles)) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_new_message_results(profiles);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to search people: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    fn open_conversation_view(&self, conversation: Conversation) {
        let pushed = {
            let window = self.imp().window.borrow();
            let Some(window) = window.as_ref() else {
                return;
            };
            window.push_message_page(&conversation)
        };
        let Some(pushed) = pushed else {
            return;
        };

        let page = match pushed {
            MessagePush::Pushed(page) => {
                let app = self.clone();
                let page_weak = page.downgrade();
                page.set_load_older_callback(move || {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    app.fetch_older_messages(&page);
                });

                // Retry redoes the first load; only a page with nothing
                // listed ever shows the button.
                let app = self.clone();
                let page_weak = page.downgrade();
                page.set_retry_callback(move || {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    app.fetch_first_messages(&page);
                });

                let app = self.clone();
                let page_weak = page.downgrade();
                page.set_send_callback(move |text| {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    app.send_conversation_message(&page, text);
                });

                let app = self.clone();
                let page_weak = page.downgrade();
                page.set_react_callback(move |message_id, value, add| {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    app.toggle_message_reaction(&page, message_id, value, add);
                });

                let app = self.clone();
                let page_weak = page.downgrade();
                page.set_delete_message_callback(move |message_id| {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    app.confirm_delete_message(&page, message_id);
                });

                self.start_message_poll(&page);
                page
            }
            MessagePush::PoppedBack(page) => {
                // A healthy page keeps what it has, and its poll is still
                // running. One whose first load failed gets another try.
                if !page.needs_reload() {
                    return;
                }
                page
            }
        };

        self.fetch_first_messages(&page);
    }

    /// Fetch the newest page of a conversation's history.
    ///
    /// The page is its own generation token: results apply through a weak
    /// ref, so a popped page drops whatever was still in flight for it.
    fn fetch_first_messages(&self, page: &MessagePage) {
        if page.is_fetching() {
            return;
        }
        page.set_fetching(true);
        let convo_id = page.convo_id();

        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<ChatMessage>, Option<String>), String>>();
        let client = self.client();
        let semaphore = API_SEMAPHORE.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                let _permit = semaphore.acquire().await;
                client.get_messages(&convo_id, None).await
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        let page_weak = page.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((messages, cursor))) => {
                    if let Some(page) = page_weak.upgrade() {
                        page.set_cursor(cursor);
                        page.set_initial_messages(messages);
                        page.set_fetching(false);
                        page.backfill_if_short();
                        // The conversation is on screen now, so the server
                        // can honestly call it read.
                        app.mark_conversation_read(page.convo_id());
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch messages: {}", e);
                    if let Some(page) = page_weak.upgrade() {
                        page.set_fetching(false);
                        page.show_load_failed();
                    }
                    app.report_session_expiry();
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch messages: connection lost");
                    if let Some(page) = page_weak.upgrade() {
                        page.set_fetching(false);
                        page.show_load_failed();
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Fetch the page of history older than what is shown.
    fn fetch_older_messages(&self, page: &MessagePage) {
        if page.is_fetching() {
            return;
        }
        let Some(cursor) = page.cursor() else {
            return;
        };
        page.set_fetching(true);
        let convo_id = page.convo_id();

        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<ChatMessage>, Option<String>), String>>();
        let client = self.client();
        let semaphore = API_SEMAPHORE.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                let _permit = semaphore.acquire().await;
                client.get_messages(&convo_id, Some(&cursor)).await
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        let page_weak = page.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((messages, cursor))) => {
                    if let Some(page) = page_weak.upgrade() {
                        page.set_cursor(cursor);
                        page.prepend_older(messages);
                        page.set_fetching(false);
                        // Filtered pages can leave the viewport unfilled
                        // with a cursor still stored.
                        page.backfill_if_short();
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch older messages: {}", e);
                    if let Some(page) = page_weak.upgrade() {
                        page.set_fetching(false);
                    }
                    // Rows are on screen, so the status page is out; say it
                    // out loud instead.
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Couldn't load older messages");
                    }
                    app.report_session_expiry();
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(page) = page_weak.upgrade() {
                        page.set_fetching(false);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Send the composer's draft. On failure the draft stays in the entry
    /// and the toast says to try again.
    fn send_conversation_message(&self, page: &MessagePage, text: String) {
        if page.is_sending() {
            return;
        }
        page.set_sending(true);
        let convo_id = page.convo_id();

        let (tx, rx) = std::sync::mpsc::channel::<Result<ChatMessage, String>>();
        let client = self.client();

        thread::spawn(move || {
            let result =
                runtime::block_on(async { client.send_chat_message(&convo_id, &text).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        let page_weak = page.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(message)) => {
                    if let Some(page) = page_weak.upgrade() {
                        page.sent_ok(message);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to send message: {}", e);
                    if let Some(page) = page_weak.upgrade() {
                        page.send_failed();
                    }
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("The message didn't send. Try again.");
                    }
                    app.report_session_expiry();
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(page) = page_weak.upgrade() {
                        page.send_failed();
                    }
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("The message didn't send. Try again.");
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Poll for new messages while the conversation page is up.
    ///
    /// The timer holds a weak ref and dies with the page. Ticks skip while
    /// the page is covered or off screen, while any fetch of its own is
    /// already out, and for the rest of a dead session.
    fn start_message_poll(&self, page: &MessagePage) {
        let app = self.clone();
        let page_weak = page.downgrade();
        glib::timeout_add_seconds_local(MESSAGE_POLL_SECS, move || {
            let Some(page) = page_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let busy = page.is_fetching() || page.is_polling() || !page.loaded_once();
            if message_poll_allowed(page.is_mapped(), busy, *app.imp().session_dead.borrow()) {
                app.poll_conversation(&page);
            }
            glib::ControlFlow::Continue
        });
    }

    /// One poll round: fetch the newest page and fold in whatever is new.
    fn poll_conversation(&self, page: &MessagePage) {
        page.set_polling(true);
        let convo_id = page.convo_id();

        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<ChatMessage>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_messages(&convo_id, None).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        let page_weak = page.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((messages, _cursor))) => {
                    if let Some(page) = page_weak.upgrade() {
                        page.set_polling(false);
                        let added = page.merge_new(messages);
                        // Only a page still on screen can honestly claim
                        // the user saw what just arrived.
                        if added > 0 && page.is_mapped() {
                            app.mark_conversation_read(page.convo_id());
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    // Transient failures self-heal on the next tick; an
                    // expiry parks the poll through the flag it raises.
                    eprintln!("Failed to poll messages: {}", e);
                    if let Some(page) = page_weak.upgrade() {
                        page.set_polling(false);
                    }
                    app.report_session_expiry();
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(page) = page_weak.upgrade() {
                        page.set_polling(false);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Add or remove one emoji reaction. The server answers with the
    /// updated message and the page swaps it in; nothing is guessed.
    fn toggle_message_reaction(
        &self,
        page: &MessagePage,
        message_id: String,
        value: String,
        add: bool,
    ) {
        let convo_id = page.convo_id();
        let (tx, rx) = std::sync::mpsc::channel::<Result<ChatMessage, String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                if add {
                    client
                        .add_chat_reaction(&convo_id, &message_id, &value)
                        .await
                } else {
                    client
                        .remove_chat_reaction(&convo_id, &message_id, &value)
                        .await
                }
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        let page_weak = page.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(message)) => {
                    if let Some(page) = page_weak.upgrade() {
                        page.update_message(message);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to update reaction: {}", e);
                    app.report_session_expiry();
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Couldn't update the reaction");
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Delete for Me is quiet but permanent for this account; ask first.
    fn confirm_delete_message(&self, page: &MessagePage, message_id: String) {
        let window = match self.imp().window.borrow().as_ref() {
            Some(w) => w.clone(),
            None => return,
        };

        let dialog = adw::AlertDialog::new(
            Some("Delete for you?"),
            Some("The message disappears from your view only. The other side keeps their copy."),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let app = self.clone();
        let page_weak = page.downgrade();
        dialog.connect_response(Some("delete"), move |_, _| {
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            app.delete_message_for_me(&page, message_id.clone());
        });
        dialog.present(Some(&window));
    }

    fn delete_message_for_me(&self, page: &MessagePage, message_id: String) {
        let convo_id = page.convo_id();
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let client = self.client();
        let id_for_call = message_id.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                client.delete_chat_message(&convo_id, &id_for_call).await
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        let page_weak = page.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    if let Some(page) = page_weak.upgrade() {
                        page.remove_message(&message_id);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to delete message: {}", e);
                    app.report_session_expiry();
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Couldn't delete the message");
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Tell the server the conversation is read, then let the UI follow:
    /// the row in the chat list drops its badge and the sidebar re-tallies.
    /// On failure both stay lit, which is the truth; the next successful
    /// open clears them.
    fn mark_conversation_read(&self, convo_id: String) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let client = self.client();
        let id = convo_id.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.mark_convo_read(&id).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_conversation_read(&convo_id);
                    }
                    app.check_unread_counts();
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to mark conversation read: {}", e);
                    app.report_session_expiry();
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Open the own profile view
    fn open_own_profile_view(&self) {
        // Switch to profile page (instant, no animation)
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.show_profile_page();
        }

        // The first open loads; after that the page keeps what it has.
        let loaded = self
            .imp()
            .window
            .borrow()
            .as_ref()
            .and_then(|w| w.own_profile_feed_ctx())
            .is_some_and(|ctx| !ctx.did.borrow().is_empty());
        if !loaded {
            self.fetch_profile_posts();
        }
    }

    /// Fetch posts for the logged-in user's profile
    fn fetch_profile_posts(&self) {
        let Some(user_did) = self.imp().user_did.borrow().clone() else {
            eprintln!("Cannot fetch profile posts: no user DID");
            return;
        };
        let Some(window) = self.imp().window.borrow().clone() else {
            return;
        };
        let Some(ctx) = window.own_profile_feed_ctx() else {
            return;
        };
        // A reopen starts the active tab over from the top.
        ctx.did.replace(user_did);
        ctx.begin_refresh();
        self.fetch_profile_tab(ctx, true);
    }

    /// Open the likes view
    fn open_likes_view(&self) {
        // Switch to likes page (instant, no animation)
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.show_likes_page();
        }

        // Only fetch if we haven't loaded likes yet
        if self.imp().likes_cursor.borrow().is_none() {
            self.fetch_likes();
        }
    }

    /// Fetch liked posts for the logged-in user
    fn fetch_likes(&self) {
        let user_did = match self.imp().user_did.borrow().clone() {
            Some(did) => did,
            None => {
                eprintln!("Cannot fetch likes: no user DID");
                return;
            }
        };

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_actor_likes(&user_did, None).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((posts, next_cursor))) => {
                    app.imp().likes_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_likes(posts);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch likes: {}", e);
                    app.toast_unless_offline("Couldn't load likes");
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_likes_load_failed();
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch likes: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Fetch more liked posts for infinite scroll
    fn fetch_likes_more(&self) {
        if *self.imp().likes_loading_more.borrow() {
            return;
        }
        let cursor = match self.imp().likes_cursor.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };
        let user_did = match self.imp().user_did.borrow().clone() {
            Some(did) => did,
            None => return,
        };
        self.imp().likes_loading_more.replace(true);

        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_likes_loading(true);
        }

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let result =
                runtime::block_on(async { client.get_actor_likes(&user_did, Some(&cursor)).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((posts, next_cursor))) => {
                    app.imp().likes_loading_more.replace(false);
                    app.imp().likes_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_likes_loading(false);
                        if !posts.is_empty() {
                            window.append_likes(posts);
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    app.imp().likes_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_likes_loading(false);
                    }
                    eprintln!("Failed to fetch more likes: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.imp().likes_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_likes_loading(false);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Open the Saved view. Posts get saved and unsaved from every list, and
    /// from other clients, so this refetches on every open rather than
    /// trusting a list loaded earlier.
    fn open_bookmarks_view(&self) {
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.show_bookmarks_page();
        }
        self.fetch_bookmarks();
    }

    /// Fetch the first page of saved posts, replacing the list
    fn fetch_bookmarks(&self) {
        // Take ownership of the list: anything still in flight from an
        // earlier visit belongs to the page this replaces.
        self.imp().bookmarks_generation.bump();
        let token = self.imp().bookmarks_generation.token();
        self.imp().bookmarks_loading_more.replace(false);

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_bookmarks(None).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(result) => {
                    if !app.imp().bookmarks_generation.is_current(token) {
                        return glib::ControlFlow::Break;
                    }
                    match result {
                        Ok((posts, next_cursor)) => {
                            app.imp().bookmarks_cursor.replace(next_cursor);
                            if let Some(window) = app.imp().window.borrow().as_ref() {
                                window.set_bookmarks(posts);
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to fetch saved posts: {}", e);
                            app.report_session_expiry();
                            if let Some(window) = app.imp().window.borrow().as_ref() {
                                window.show_toast("Couldn't load saved posts");
                                window.set_bookmarks_load_failed();
                            }
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch saved posts: connection lost");
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Couldn't load saved posts");
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Fetch more saved posts for infinite scroll
    fn fetch_bookmarks_more(&self) {
        if *self.imp().bookmarks_loading_more.borrow() {
            return;
        }
        let cursor = match self.imp().bookmarks_cursor.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };
        self.imp().bookmarks_loading_more.replace(true);
        let token = self.imp().bookmarks_generation.token();

        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_bookmarks_loading(true);
        }

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_bookmarks(Some(&cursor)).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(result) => {
                    app.imp().bookmarks_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_bookmarks_loading(false);
                    }
                    if !app.imp().bookmarks_generation.is_current(token) {
                        return glib::ControlFlow::Break;
                    }
                    match result {
                        Ok((posts, next_cursor)) => {
                            app.imp().bookmarks_cursor.replace(next_cursor);
                            if !posts.is_empty() {
                                if let Some(window) = app.imp().window.borrow().as_ref() {
                                    window.append_bookmarks(posts);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to fetch more saved posts: {}", e);
                            app.report_session_expiry();
                            if let Some(window) = app.imp().window.borrow().as_ref() {
                                window.show_toast("Couldn't load more saved posts");
                            }
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.imp().bookmarks_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_bookmarks_loading(false);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Save or unsave a post, whichever the menu offered
    fn toggle_bookmark(&self, post: Post, row_weak: glib::WeakRef<PostRow>) {
        let was_saved = post.viewer_bookmarked == Some(true);
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let client = self.client();
        let uri = post.uri.clone();
        let cid = post.cid.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                if was_saved {
                    client.delete_bookmark(&uri).await
                } else {
                    client.create_bookmark(&uri, &cid).await
                }
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        let uri = post.uri;
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    // Flip the row, but only if it is still showing this post;
                    // a pooled row may have been rebound while we waited.
                    if let Some(row) = row_weak.upgrade() {
                        if row.post_uri().as_deref() == Some(uri.as_str()) {
                            row.set_viewer_bookmarked(!was_saved);
                        }
                    }
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        if was_saved {
                            // Unsaving from the Saved page takes the row out
                            // now; from any other list this is a no-op.
                            window.remove_bookmark(&uri);
                            window.show_toast("Removed from Saved");
                        } else {
                            window.show_toast("Post saved");
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to update bookmark: {}", e);
                    app.report_session_expiry();
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast(if was_saved {
                            "Couldn't remove from Saved"
                        } else {
                            "Couldn't save post"
                        });
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Track connectivity. Going offline raises the banner and parks the
    /// polls; coming back lowers it and refreshes the feed, since whatever
    /// is on screen predates the outage.
    fn set_network_available(&self, available: bool) {
        let was_offline = *self.imp().offline.borrow();
        self.imp().offline.replace(!available);
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_offline(!available);
        }
        if available && was_offline && self.imp().user_did.borrow().is_some() {
            self.fetch_timeline();
        }
    }

    /// A failure toast, unless the offline banner already explains it.
    fn toast_unless_offline(&self, message: &str) {
        if *self.imp().offline.borrow() {
            return;
        }
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.show_toast(message);
        }
    }

    /// Flip a follow on the server and hand the outcome to `done` on the
    /// main thread: Ok(Some(uri)) followed, Ok(None) unfollowed. A failure
    /// toasts and reports session expiry before `done` runs.
    fn follow_flip<F: Fn(Result<Option<String>, String>) + 'static>(
        &self,
        did: String,
        current_uri: Option<String>,
        done: F,
    ) {
        let unfollowing = current_uri.is_some();
        let (tx, rx) = std::sync::mpsc::channel::<Result<Option<String>, String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                match current_uri {
                    Some(uri) => client.unfollow(&uri).await.map(|_| None),
                    None => client.follow(&did).await.map(Some),
                }
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(result) => {
                    if let Err(e) = &result {
                        eprintln!("Failed to update follow: {}", e);
                        app.report_session_expiry();
                        if let Some(window) = app.imp().window.borrow().as_ref() {
                            window.show_toast(if unfollowing {
                                "Couldn't unfollow"
                            } else {
                                "Couldn't follow"
                            });
                        }
                    }
                    done(result);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Follow or unfollow from a profile page, decided by whether the page
    /// holds a record URI.
    ///
    /// The button already shows the hoped-for state and is insensitive; on
    /// success the cell takes the new URI, on failure the button goes back.
    fn toggle_follow(
        &self,
        did: String,
        uri_cell: Rc<RefCell<Option<String>>>,
        btn_weak: glib::WeakRef<gtk4::Button>,
    ) {
        let current_uri = uri_cell.borrow().clone();
        self.follow_flip(did, current_uri, move |result| {
            if let Ok(new_uri) = result {
                uri_cell.replace(new_uri);
            }
            if let Some(btn) = btn_weak.upgrade() {
                HangarWindow::sync_follow_button(&btn, uri_cell.borrow().is_some());
            }
        });
    }

    /// Follow or unfollow from a people row. The result only lands if the
    /// recycled row still shows the same profile; the model object behind
    /// it is updated either way through the row.
    fn toggle_follow_row(
        &self,
        profile: Profile,
        row_weak: glib::WeakRef<crate::ui::actor_row::ActorRow>,
    ) {
        let did = profile.did.clone();
        let current_uri = profile.viewer_following.clone();
        self.follow_flip(profile.did, current_uri.clone(), move |result| {
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            if row.profile_did().as_deref() != Some(did.as_str()) {
                return;
            }
            match result {
                Ok(new_uri) => row.set_viewer_following(new_uri),
                // The button showed the hoped-for state; put it back.
                Err(_) => row.set_viewer_following(current_uri.clone()),
            }
        });
    }

    /// What a tab's page actually lists. The server's posts_with_replies
    /// is a superset with standalone posts included; the Replies tab wants
    /// only the replies, so the rest of the page stays behind.
    fn tab_posts(filter: &str, mut posts: Vec<Post>) -> Vec<Post> {
        if filter == "posts_with_replies" {
            posts.retain(|post| post.reply_context.is_some());
        }
        posts
    }

    /// One page of a profile tab's feed. First pages were cleared by the
    /// tab switch; later pages append. The context's generation strands a
    /// fetch whose tab was switched away while it was out.
    fn fetch_profile_tab(&self, ctx: std::rc::Rc<crate::ui::ProfileFeedCtx>, first_page: bool) {
        if ctx.fetching.get() {
            return;
        }
        let did = ctx.did.borrow().clone();
        // The own page's context exists before sign-in does.
        if did.is_empty() {
            return;
        }
        ctx.fetching.set(true);
        let generation = ctx.generation.get();
        let filter = ctx.filter.get();
        let cursor = if first_page {
            None
        } else {
            ctx.cursor.borrow().clone()
        };

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();
        thread::spawn(move || {
            let result = runtime::block_on(async {
                client
                    .get_author_feed(&did, cursor.as_deref(), Some(filter))
                    .await
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if ctx.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            match rx.try_recv() {
                Ok(Ok((posts, next_cursor))) => {
                    ctx.fetching.set(false);
                    ctx.cursor.replace(next_cursor);
                    ctx.append_posts(Self::tab_posts(filter, posts));
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    ctx.fetching.set(false);
                    eprintln!("Failed to fetch profile tab: {}", e);
                    app.toast_unless_offline("Couldn't load these posts");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    ctx.fetching.set(false);
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Ask GitHub for the latest release and offer it in a toast when it
    /// beats the running build. Quiet on every failure; an update nudge
    /// that can error is worse than none.
    fn check_for_updates(&self) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(String, String), String>>();
        thread::spawn(move || {
            let result = runtime::block_on(crate::update::fetch_latest());
            let _ = tx.send(result);
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            match rx.try_recv() {
                Ok(Ok((tag, url))) => {
                    if crate::update::newer_available(env!("HANGAR_VERSION"), &tag)
                        && let Some(window) = app.imp().window.borrow().as_ref()
                    {
                        let version = tag.trim_start_matches('v').to_string();
                        let window_weak = window.downgrade();
                        window.show_toast_with_action(
                            &format!("Hangar {version} is available"),
                            "Get It",
                            move || {
                                if let Some(window) = window_weak.upgrade() {
                                    crate::ui::external::open_url(
                                        &window,
                                        &url,
                                        "the release page",
                                    );
                                }
                            },
                        );
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Update check failed: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Mute or unmute, decided by the page's cell; the cell only flips
    /// once the server agrees.
    fn toggle_mute(&self, did: String, cell: std::rc::Rc<std::cell::Cell<bool>>) {
        let muted = cell.get();
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let client = self.client();
        let did_for_call = did.clone();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                if muted {
                    client.unmute_actor(&did_for_call).await
                } else {
                    client.mute_actor(&did_for_call).await
                }
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    cell.set(!muted);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast(if muted {
                            "Account unmuted"
                        } else {
                            "Account muted. Their posts drop from your feeds."
                        });
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to update mute: {}", e);
                    app.report_session_expiry();
                    app.toast_unless_offline("Couldn't update the mute");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Block after a confirmation; unblock straight away. The cell holds
    /// the block record URI and only changes once the server agrees.
    fn toggle_block(&self, profile: Profile, cell: std::rc::Rc<RefCell<Option<String>>>) {
        let existing = cell.borrow().clone();
        if let Some(uri) = existing {
            self.run_unblock(uri, cell);
            return;
        }

        let window = match self.imp().window.borrow().as_ref() {
            Some(w) => w.clone(),
            None => return,
        };
        let dialog = adw::AlertDialog::new(
            Some(&format!("Block @{}?", profile.handle)),
            Some(
                "They won't be able to see your posts, follow you, or message you, and their content disappears from your view.",
            ),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("block", "Block");
        dialog.set_response_appearance("block", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let app = self.clone();
        dialog.connect_response(Some("block"), move |_, _| {
            app.run_block(profile.did.clone(), cell.clone());
        });
        dialog.present(Some(&window));
    }

    fn run_block(&self, did: String, cell: std::rc::Rc<RefCell<Option<String>>>) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        let client = self.client();
        thread::spawn(move || {
            let result = runtime::block_on(async { client.block(&did).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(uri)) => {
                    cell.replace(Some(uri));
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Account blocked");
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to block: {}", e);
                    app.report_session_expiry();
                    app.toast_unless_offline("Couldn't block the account");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    fn run_unblock(&self, uri: String, cell: std::rc::Rc<RefCell<Option<String>>>) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let client = self.client();
        let uri_for_call = uri.clone();
        thread::spawn(move || {
            let result = runtime::block_on(async { client.unblock(&uri_for_call).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    cell.replace(None);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Account unblocked");
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to unblock: {}", e);
                    app.report_session_expiry();
                    app.toast_unless_offline("Couldn't unblock the account");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    fn open_report_for_account(&self, profile: Profile) {
        let Some(window) = self.imp().window.borrow().clone() else {
            return;
        };
        let app = self.clone();
        let subject = format!("Reporting @{}", profile.handle);
        crate::ui::report_dialog::present(&window, &subject, move |reason, details| {
            app.submit_report(
                reason,
                details,
                crate::atproto::client::ReportSubject::Account(profile.did.clone()),
            );
        });
    }

    fn open_report_for_post(&self, post: Post) {
        let Some(window) = self.imp().window.borrow().clone() else {
            return;
        };
        let app = self.clone();
        let subject = format!("Reporting a post by @{}", post.author.handle);
        crate::ui::report_dialog::present(&window, &subject, move |reason, details| {
            app.submit_report(
                reason,
                details,
                crate::atproto::client::ReportSubject::Post {
                    uri: post.uri.clone(),
                    cid: post.cid.clone(),
                },
            );
        });
    }

    fn submit_report(
        &self,
        reason: &'static str,
        details: String,
        subject: crate::atproto::client::ReportSubject,
    ) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let client = self.client();
        thread::spawn(move || {
            let result =
                runtime::block_on(async { client.create_report(reason, &details, subject).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Report submitted. Thank you.");
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to submit report: {}", e);
                    app.report_session_expiry();
                    app.toast_unless_offline("Couldn't submit the report");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Edit Profile from the own page. The dialog prefills from what the
    /// header shows and hands back only what changed.
    fn open_edit_profile(&self) {
        let Some(window) = self.imp().window.borrow().clone() else {
            return;
        };
        let Some(profile) = window.current_profile() else {
            // Nothing loaded yet; nothing truthful to prefill with.
            return;
        };
        let app = self.clone();
        crate::ui::edit_profile::present(&window, &profile, move |edits| {
            app.save_profile(edits);
        });
    }

    fn save_profile(&self, edits: crate::ui::edit_profile::ProfileEdits) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                client
                    .update_profile(
                        &edits.display_name,
                        &edits.description,
                        edits.avatar,
                        edits.banner,
                    )
                    .await
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Profile updated");
                    }
                    app.refresh_own_profile();
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to update profile: {}", e);
                    app.report_session_expiry();
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.show_toast("Couldn't update your profile");
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// The header, sidebar avatar, and cache, refetched past the cache's
    /// freshness window; a profile edit is exactly the moment it lies.
    fn refresh_own_profile(&self) {
        let Some(did) = self.imp().user_did.borrow().clone() else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel::<Result<Profile, String>>();
        let client = self.client();
        let did_for_fetch = did.clone();
        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_profile(&did_for_fetch).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(profile)) => {
                    if let Some(cache) = app.imp().cache.borrow().as_ref() {
                        let _ = ProfileCache::new(cache).store_full(&profile);
                    }
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        let display_name =
                            profile.display_name.as_deref().unwrap_or(&profile.handle);
                        window.set_user_avatar(display_name, profile.avatar.as_deref());
                        window.update_profile_header(&profile);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to refresh profile: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    /// Open the search view
    fn open_search_view(&self) {
        // Switch to search page
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.show_search_page();
            window.focus_search_entry();
        }
    }

    /// Execute a search with the given query
    fn execute_search(&self, query: String) {
        // Strand any fetch still out for the previous query
        self.imp().search_generation.bump();

        // Clear previous results and reset state, both pages
        self.imp().search_query.replace(Some(query.clone()));
        self.imp().search_cursor.replace(None);
        self.imp().search_loading_more.replace(false);
        self.imp().search_people_cursor.replace(None);
        self.imp().search_people_loading_more.replace(false);

        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.clear_search_results();
            window.set_search_loading(true);
            window.set_search_people_loading(true);
        }

        self.fetch_search(&query);
        self.fetch_search_people(&query);
    }

    /// Fetch search results
    fn fetch_search(&self, query: &str) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();
        let query = query.to_string();
        let generation = self.imp().search_generation.token();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.search_posts(&query, None).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if !app.imp().search_generation.is_current(generation) {
                // A newer search owns the list now; drop these results.
                return glib::ControlFlow::Break;
            }
            match rx.try_recv() {
                Ok(Ok((posts, next_cursor))) => {
                    app.imp().search_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_loading(false);
                        window.set_search_results(posts);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_loading(false);
                    }
                    eprintln!("Failed to search: {}", e);
                    app.toast_unless_offline("Search failed");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_loading(false);
                    }
                    eprintln!("Failed to search: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Fetch more search results for infinite scroll
    fn fetch_search_more(&self) {
        if *self.imp().search_loading_more.borrow() {
            return;
        }
        let cursor = match self.imp().search_cursor.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };
        let query = match self.imp().search_query.borrow().as_ref() {
            Some(q) => q.clone(),
            None => return,
        };
        self.imp().search_loading_more.replace(true);

        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_search_loading(true);
        }

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();
        let semaphore = API_SEMAPHORE.clone();
        let generation = self.imp().search_generation.token();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                let _permit = semaphore.acquire().await;
                client.search_posts(&query, Some(&cursor)).await
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if !app.imp().search_generation.is_current(generation) {
                // A newer search reset the loading flags already.
                return glib::ControlFlow::Break;
            }
            match rx.try_recv() {
                Ok(Ok((posts, next_cursor))) => {
                    app.imp().search_loading_more.replace(false);
                    app.imp().search_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_loading(false);
                        if !posts.is_empty() {
                            window.append_search_results(posts);
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    app.imp().search_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_loading(false);
                    }
                    eprintln!("Failed to fetch more search results: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.imp().search_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_loading(false);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Fetch people results for the search page
    fn fetch_search_people(&self, query: &str) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Profile>, Option<String>), String>>();
        let client = self.client();
        let query = query.to_string();
        let generation = self.imp().search_generation.token();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.search_actors(&query, None).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if !app.imp().search_generation.is_current(generation) {
                // A newer search owns the list now; drop these results.
                return glib::ControlFlow::Break;
            }
            match rx.try_recv() {
                Ok(Ok((profiles, next_cursor))) => {
                    app.imp().search_people_cursor.replace(next_cursor);
                    let nobody = profiles.is_empty();
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_people_loading(false);
                        window.set_search_people_results(profiles);
                    }
                    // A dead end gets suggestions instead of a shrug.
                    if nobody {
                        app.fetch_search_suggestions(generation);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_people_loading(false);
                    }
                    eprintln!("Failed to search people: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_people_loading(false);
                    }
                    eprintln!("Failed to search people: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Fetch more people results for infinite scroll
    /// Suggested accounts for an empty people search. The generation token
    /// belongs to the search that came up dry; a newer search drops these.
    fn fetch_search_suggestions(&self, generation: u64) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<Profile>, String>>();
        let client = self.client();

        thread::spawn(move || {
            let result = runtime::block_on(async { client.get_suggestions(8).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if !app.imp().search_generation.is_current(generation) {
                return glib::ControlFlow::Break;
            }
            match rx.try_recv() {
                Ok(Ok(profiles)) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_suggestions(profiles);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    // The no-results page stands on its own; suggestions
                    // failing to load is not worth a toast.
                    eprintln!("Failed to fetch suggestions: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    fn fetch_search_people_more(&self) {
        if *self.imp().search_people_loading_more.borrow() {
            return;
        }
        let cursor = match self.imp().search_people_cursor.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };
        let query = match self.imp().search_query.borrow().as_ref() {
            Some(q) => q.clone(),
            None => return,
        };
        self.imp().search_people_loading_more.replace(true);

        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.set_search_people_loading(true);
        }

        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Profile>, Option<String>), String>>();
        let client = self.client();
        let semaphore = API_SEMAPHORE.clone();
        let generation = self.imp().search_generation.token();

        thread::spawn(move || {
            let result = runtime::block_on(async {
                let _permit = semaphore.acquire().await;
                client.search_actors(&query, Some(&cursor)).await
            });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if !app.imp().search_generation.is_current(generation) {
                // A newer search reset the loading flags already.
                return glib::ControlFlow::Break;
            }
            match rx.try_recv() {
                Ok(Ok((profiles, next_cursor))) => {
                    app.imp().search_people_loading_more.replace(false);
                    app.imp().search_people_cursor.replace(next_cursor);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_people_loading(false);
                        if !profiles.is_empty() {
                            window.append_search_people_results(profiles);
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    app.imp().search_people_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_people_loading(false);
                    }
                    eprintln!("Failed to fetch more people results: {}", e);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.imp().search_people_loading_more.replace(false);
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_search_people_loading(false);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }
}

impl Default for HangarApplication {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether an unread poll may start: someone is signed in, the last one
/// came back, and the session has not expired. The timer outlives sign-out
/// and expiry alike, so this is what quiets it.
fn unread_poll_allowed(signed_in: bool, in_flight: bool, expired: bool) -> bool {
    signed_in && !in_flight && !expired
}

/// Whether a message-poll tick may fetch: the conversation page is on
/// screen, none of its fetches are already out and its first load landed,
/// and the session has not expired. The timer itself only dies with the
/// page; this is what quiets it in between.
fn message_poll_allowed(mapped: bool, busy: bool, expired: bool) -> bool {
    mapped && !busy && !expired
}

#[cfg(test)]
mod tests {
    use super::Generation;
    use super::message_poll_allowed;
    use super::unread_poll_allowed;

    /// The badge poll must stay quiet when signed out, must not stack
    /// fetches when the network is slow, and must park for the rest of a
    /// dead session once an expiry is reported.
    #[test]
    fn the_unread_poll_waits_for_sign_in_and_for_itself() {
        assert!(unread_poll_allowed(true, false, false));
        assert!(
            !unread_poll_allowed(false, false, false),
            "signed out: nothing to count"
        );
        assert!(
            !unread_poll_allowed(true, true, false),
            "one fetch at a time"
        );
        assert!(!unread_poll_allowed(false, true, false));
        assert!(
            !unread_poll_allowed(true, false, true),
            "an expired session parks the poll until the next sign-in"
        );
        assert!(!unread_poll_allowed(true, true, true));
        assert!(!unread_poll_allowed(false, false, true));
    }

    /// The message poll must stay quiet while its page is off screen,
    /// while any of the page's fetches are already out, and for the rest
    /// of a dead session. Only the page being dropped stops the timer;
    /// this gate is what holds each tick.
    #[test]
    fn the_message_poll_waits_for_its_page_and_for_itself() {
        assert!(message_poll_allowed(true, false, false));
        assert!(
            !message_poll_allowed(false, false, false),
            "a covered or hidden page is not being read"
        );
        assert!(
            !message_poll_allowed(true, true, false),
            "one fetch per page at a time"
        );
        assert!(
            !message_poll_allowed(true, false, true),
            "an expired session parks the poll"
        );
        assert!(!message_poll_allowed(false, true, false));
        assert!(!message_poll_allowed(false, false, true));
        assert!(!message_poll_allowed(true, true, true));
        assert!(!message_poll_allowed(false, true, true));
    }

    /// A late page from the old query must not splice into the new list.
    #[test]
    fn a_new_search_strands_fetches_from_the_old_one() {
        let generation = Generation::default();

        let old = generation.token();
        assert!(
            generation.is_current(old),
            "a token is current until a new search runs"
        );

        generation.bump();
        assert!(
            !generation.is_current(old),
            "the old query's fetch must be dropped"
        );

        let new = generation.token();
        assert!(
            generation.is_current(new),
            "the new query's fetch still applies"
        );

        generation.bump();
        assert!(
            !generation.is_current(new),
            "every bump strands earlier fetches"
        );
    }

    /// The Replies tab lists only replies; the server's filter is a
    /// superset with standalone posts mixed in. Other tabs pass through.
    #[test]
    fn the_replies_tab_keeps_only_replies() {
        use crate::atproto::{Post, Profile, ReplyContext};

        let post = |uri: &str, reply: bool| {
            let author = Profile::minimal("did:plc:a".into(), "a.bsky.social".into(), None, None);
            Post {
                uri: uri.into(),
                cid: "cid".into(),
                author: author.clone(),
                text: "words".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                indexed_at: "2026-01-01T00:00:00Z".into(),
                like_count: None,
                repost_count: None,
                reply_count: None,
                embed: None,
                viewer_like: None,
                viewer_repost: None,
                viewer_bookmarked: None,
                repost_reason: None,
                reply_context: reply.then(|| ReplyContext {
                    parent_author: author.clone(),
                    root_author: author,
                }),
            }
        };

        let page = vec![post("at://a/1", false), post("at://a/2", true)];
        let replies = super::HangarApplication::tab_posts("posts_with_replies", page.clone());
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].uri, "at://a/2");

        let posts = super::HangarApplication::tab_posts("posts_and_author_threads", page.clone());
        assert_eq!(posts.len(), 2, "other tabs list the page as served");
        let media = super::HangarApplication::tab_posts("posts_with_media", page);
        assert_eq!(media.len(), 2);
    }

    /// The About dialog states the build's version: what CI exported
    /// through HANGAR_VERSION, or the crate version when nothing did. A
    /// release cannot leave a stale number behind.
    #[test]
    fn the_about_dialog_reads_from_the_crate() {
        crate::ui::with_gtk(the_about_dialog_reads_from_the_crate_body);
    }

    fn the_about_dialog_reads_from_the_crate_body() {
        let about = super::HangarApplication::build_about_dialog();
        assert_eq!(about.version(), env!("HANGAR_VERSION"));
        assert_eq!(about.license_type(), gtk4::License::Mpl20);
        assert_eq!(about.application_icon(), crate::config::APP_ID);
    }
}
