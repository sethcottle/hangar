// SPDX-License-Identifier: MPL-2.0

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use std::cell::RefCell;
use std::sync::Arc;
use std::thread;

use crate::atproto::{Conversation, HangarClient, Notification, Post, Profile, SavedFeed, Session};
use crate::state::SessionManager;
use crate::ui::post_row::PostRow;
use crate::ui::{ComposeDialog, HangarWindow, LoginDialog, NavItem, QuoteContext, ReplyContext};

mod imp {
    use super::*;
    use libadwaita::subclass::prelude::*;

    #[derive(Default)]
    pub struct HangarApplication {
        pub client: RefCell<Option<Arc<HangarClient>>>,
        pub window: RefCell<Option<HangarWindow>>,
        pub timeline_cursor: RefCell<Option<String>>,
        pub loading_more: RefCell<bool>,
        /// The URI of the newest post we've shown the user (anchor for new posts detection)
        pub newest_post_uri: RefCell<Option<String>>,
        /// Pending new posts that arrived while user was scrolled away
        pub pending_new_posts: RefCell<Vec<Post>>,
        /// Whether we're currently checking for new posts
        pub checking_new_posts: RefCell<bool>,
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

            // Register custom icons
            let display = gtk4::gdk::Display::default().expect("Could not get default display");
            let icon_theme = gtk4::IconTheme::for_display(&display);

            // Add our bundled icons path - try multiple locations for development vs installed
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
            css_provider.load_from_data(include_str!("ui/style.css"));

            gtk4::style_context_add_provider_for_display(
                &display,
                &css_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        fn activate(&self) {
            let app = self.obj();

            // Create main window
            let window = HangarWindow::new(app.upcast_ref::<adw::Application>());
            self.window.replace(Some(window.clone()));

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
            .property("application-id", "io.github.sethcottle.Hangar")
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

    fn try_restore_session(&self) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Session, String>>();
        let client = self.client();

        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                let session = SessionManager::load().await.map_err(|e| e.to_string())?;
                client
                    .resume_session(&session)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(session)
            });
            let _ = tx.send(result);
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(session)) => {
                    if app.imp().window.borrow().as_ref().is_some() {
                        app.fetch_user_profile(&session.did);
                        app.fetch_saved_feeds();
                        app.fetch_timeline();
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        app.show_login_dialog(window);
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            }
        });
    }

    fn show_login_dialog(&self, window: &HangarWindow) {
        let dialog = LoginDialog::new(window);

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
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    let session = client
                        .login(&handle, &password)
                        .await
                        .map_err(|e| e.to_string())?;
                    if let Err(e) = SessionManager::store(&session).await {
                        eprintln!("Failed to persist session: {}", e);
                    }
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

        dialog.present();
    }

    fn fetch_user_profile(&self, did: &str) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Profile, String>>();

        let client = self.client();
        let did = did.to_string();
        let did_for_window = did.clone();
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async { client.get_profile(&did).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(profile)) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        let display_name =
                            profile.display_name.as_deref().unwrap_or(&profile.handle);
                        window.set_user_avatar(display_name, profile.avatar.as_deref());
                        // Set current user DID for filtering conversations
                        window.set_current_user_did(&did_for_window);
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
        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();

        let client = self.client();
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async { client.get_timeline(None).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((posts, next_cursor))) => {
                    app.imp().timeline_cursor.replace(next_cursor);
                    // Set anchor to the newest post
                    if let Some(first) = posts.first() {
                        app.imp().newest_post_uri.replace(Some(first.uri.clone()));
                    }
                    // Clear any pending new posts since we just refreshed
                    app.imp().pending_new_posts.replace(Vec::new());
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

        // Check if already liked - if so, unlike
        if let Some(like_uri) = &post.viewer_like {
            let like_uri = like_uri.clone();
            thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async { client.unlike(&like_uri).await });
                let _ = tx.send(result.map(|_| None).map_err(|e| e.to_string()));
            });
        } else {
            // Not liked yet, create like
            let uri = post.uri.clone();
            let cid = post.cid.clone();
            thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async { client.like(&uri, &cid).await });
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

    fn open_compose_dialog(&self) {
        let window = match self.imp().window.borrow().as_ref() {
            Some(w) => w.clone(),
            None => return,
        };
        let dialog = ComposeDialog::new(&window);

        let app = self.clone();
        let dialog_weak = dialog.downgrade();

        dialog.connect_post(move |text| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.set_loading(true);
                dialog.hide_error();
            }

            let app = app.clone();
            let dialog_weak = dialog_weak.clone();

            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let client = app.client();
            let text = text.to_string();

            thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async { client.create_post(&text).await });
                let _ = tx.send(result.map_err(|e| e.to_string()));
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

        dialog.present();
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

        let dialog = ComposeDialog::new_reply(&window, context);

        let app = self.clone();
        let dialog_weak = dialog.downgrade();

        dialog.connect_reply(move |text, parent_uri, parent_cid| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.set_loading(true);
                dialog.hide_error();
            }

            let app = app.clone();
            let dialog_weak = dialog_weak.clone();

            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let client = app.client();
            let text = text.to_string();

            thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt
                    .block_on(async { client.create_reply(&text, &parent_uri, &parent_cid).await });
                let _ = tx.send(result.map_err(|e| e.to_string()));
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

        dialog.present();
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

        let dialog = ComposeDialog::new_quote(&window, context);

        let app = self.clone();
        let dialog_weak = dialog.downgrade();

        dialog.connect_quote(move |text, quoted_uri, quoted_cid| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.set_loading(true);
                dialog.hide_error();
            }

            let app = app.clone();
            let dialog_weak = dialog_weak.clone();

            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let client = app.client();
            let text = text.to_string();

            thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    client
                        .create_quote_post(&text, &quoted_uri, &quoted_cid)
                        .await
                });
                let _ = tx.send(result.map_err(|e| e.to_string()));
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

        dialog.present();
    }

    fn toggle_repost(&self, post: &Post, post_row_weak: glib::WeakRef<PostRow>) {
        // Result type: Ok(Some(uri)) for repost success, Ok(None) for unrepost success, Err for failure
        let (tx, rx) = std::sync::mpsc::channel::<Result<Option<String>, String>>();
        let client = self.client();

        // Check if already reposted - if so, delete repost
        if let Some(repost_uri) = &post.viewer_repost {
            let repost_uri = repost_uri.clone();
            thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async { client.delete_repost(&repost_uri).await });
                let _ = tx.send(result.map(|_| None).map_err(|e| e.to_string()));
            });
        } else {
            // Not reposted yet, create repost
            let uri = post.uri.clone();
            let cid = post.cid.clone();
            thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async { client.repost(&uri, &cid).await });
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
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
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
                    app.imp().timeline_cursor.replace(next_cursor);
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
        let app = self.clone();
        // Poll every 30 seconds for new posts
        glib::timeout_add_seconds_local(30, move || {
            app.check_for_new_posts();
            glib::ControlFlow::Continue
        });
    }

    fn check_for_new_posts(&self) {
        // Don't check if we're already checking or if user is at top (they'll see new posts on refresh)
        if *self.imp().checking_new_posts.borrow() {
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
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
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

                    // Find posts newer than our anchor
                    let new_posts: Vec<Post> = posts
                        .into_iter()
                        .take_while(|p| p.uri != anchor_uri)
                        .collect();

                    if !new_posts.is_empty() {
                        // Check if user is at top - if so, just prepend and update anchor
                        let is_at_top = app
                            .imp()
                            .window
                            .borrow()
                            .as_ref()
                            .map(|w| w.is_at_top())
                            .unwrap_or(true);

                        if is_at_top {
                            // User is at top, show new posts immediately
                            if let Some(first) = new_posts.first() {
                                app.imp().newest_post_uri.replace(Some(first.uri.clone()));
                            }
                            if let Some(window) = app.imp().window.borrow().as_ref() {
                                window.prepend_posts(new_posts);
                            }
                        } else {
                            // User is scrolled away, accumulate and show banner
                            let mut pending = app.imp().pending_new_posts.borrow_mut();
                            for post in new_posts.into_iter().rev() {
                                // Check if we already have this post
                                if !pending.iter().any(|p| p.uri == post.uri) {
                                    pending.insert(0, post);
                                }
                            }
                            let count = pending.len();
                            drop(pending);

                            if let Some(window) = app.imp().window.borrow().as_ref() {
                                window.show_new_posts_banner(count);
                            }
                        }
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    app.imp().checking_new_posts.replace(false);
                    eprintln!("Failed to check for new posts: {}", e);
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
        let pending = self.imp().pending_new_posts.borrow().clone();
        if pending.is_empty() {
            return;
        }

        // Update anchor to the newest post
        if let Some(first) = pending.first() {
            self.imp().newest_post_uri.replace(Some(first.uri.clone()));
        }

        // Clear pending
        self.imp().pending_new_posts.replace(Vec::new());

        // Prepend posts and scroll to top
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.hide_new_posts_banner();
            window.prepend_posts(pending);
            window.scroll_to_top();
        }
    }

    /// Fetch the user's saved feeds and populate the feed selector
    fn fetch_saved_feeds(&self) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<SavedFeed>, String>>();
        let client = self.client();

        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async { client.get_saved_feeds().await });
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
        self.imp().pending_new_posts.replace(Vec::new());

        // Fetch the new feed
        self.fetch_current_feed();
    }

    /// Fetch posts for the current feed (home timeline or custom feed)
    fn fetch_current_feed(&self) {
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
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
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
                Ok(Ok((posts, next_cursor))) => {
                    app.imp().timeline_cursor.replace(next_cursor);
                    if let Some(first) = posts.first() {
                        app.imp().newest_post_uri.replace(Some(first.uri.clone()));
                    }
                    app.imp().pending_new_posts.replace(Vec::new());
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.hide_new_posts_banner();
                        window.set_posts(posts);
                    }
                    app.start_new_posts_polling();
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch feed: {}", e);
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
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async { client.get_thread(&post_uri).await });
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
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("Failed to fetch thread: connection lost");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Open the profile view for a user
    fn open_profile_view(&self, profile: Profile) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(Vec<Post>, Option<String>), String>>();
        let client = self.client();
        let actor = profile.did.clone();
        let profile_clone = profile.clone();

        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async { client.get_author_feed(&actor, None).await });
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok((posts, _cursor))) => {
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.push_profile_page(&profile_clone, posts);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to fetch profile feed: {}", e);
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
            // Other nav items not yet implemented
            _ => {}
        }
    }

    /// Open the mentions view
    fn open_mentions_view(&self) {
        // Switch to mentions page (instant, no animation)
        if let Some(window) = self.imp().window.borrow().as_ref() {
            window.show_mentions_page();
        }

        // Only fetch if we haven't loaded mentions yet
        if self.imp().mentions_cursor.borrow().is_none() {
            self.fetch_mentions();
        }
    }

    /// Fetch mentions (notifications filtered to mentions/replies/quotes)
    fn fetch_mentions(&self) {
        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<Notification>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async { client.get_notifications(None, true).await });
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

        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async { client.get_notifications(Some(&cursor), true).await });
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

        // Only fetch if we haven't loaded activity yet
        if self.imp().activity_cursor.borrow().is_none() {
            self.fetch_activity();
        }
    }

    /// Fetch activity (all notifications)
    fn fetch_activity(&self) {
        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<Notification>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            // Pass false for mentions_only to get all notifications
            let result = rt.block_on(async { client.get_notifications(None, false).await });
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

        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            // Pass false for mentions_only to get all notifications
            let result =
                rt.block_on(async { client.get_notifications(Some(&cursor), false).await });
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

        // Only fetch if we haven't loaded chat yet
        if self.imp().chat_cursor.borrow().is_none() {
            self.fetch_conversations();
        }
    }

    /// Fetch conversations
    fn fetch_conversations(&self) {
        let (tx, rx) =
            std::sync::mpsc::channel::<Result<(Vec<Conversation>, Option<String>), String>>();
        let client = self.client();

        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async { client.get_conversations(None).await });
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
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async { client.get_conversations(Some(&cursor)).await });
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

    /// Open a conversation view (placeholder for now)
    fn open_conversation_view(&self, conversation: Conversation) {
        // TODO: Implement conversation detail view with messages
        eprintln!(
            "Opening conversation: {} with {} members",
            conversation.id,
            conversation.members.len()
        );
    }
}

impl Default for HangarApplication {
    fn default() -> Self {
        Self::new()
    }
}
