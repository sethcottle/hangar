// SPDX-License-Identifier: MPL-2.0

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use std::cell::RefCell;
use std::sync::Arc;
use std::thread;

use crate::atproto::{HangarClient, Post, Session};
use crate::ui::{HangarWindow, LoginDialog};

mod imp {
    use super::*;
    use libadwaita::subclass::prelude::*;

    #[derive(Default)]
    pub struct HangarApplication {
        pub client: RefCell<Option<Arc<HangarClient>>>,
        pub window: RefCell<Option<HangarWindow>>,
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

            // Load CSS
            let css_provider = gtk4::CssProvider::new();
            css_provider.load_from_data(include_str!("ui/style.css"));

            gtk4::style_context_add_provider_for_display(
                &gtk4::gdk::Display::default().expect("Could not get default display"),
                &css_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        fn activate(&self) {
            let app = self.obj();

            // Create main window
            let window = HangarWindow::new(app.upcast_ref::<adw::Application>());
            self.window.replace(Some(window.clone()));

            window.present();

            // Show login dialog
            app.show_login_dialog(&window);
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

            // Spawn the async work on a separate thread with its own Tokio runtime
            let client = app.client();
            thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async { client.login(&handle, &password).await });
                let _ = tx.send(result.map_err(|e| e.to_string()));
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

    fn fetch_timeline(&self) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<Post>, String>>();

        let client = self.client();
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async { client.get_timeline(None).await });
            let _ = tx.send(result.map(|(posts, _)| posts).map_err(|e| e.to_string()));
        });

        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(Ok(posts)) => {
                    println!("Fetched {} posts", posts.len());
                    if let Some(window) = app.imp().window.borrow().as_ref() {
                        window.set_posts(posts);
                    }
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
}

impl Default for HangarApplication {
    fn default() -> Self {
        Self::new()
    }
}
