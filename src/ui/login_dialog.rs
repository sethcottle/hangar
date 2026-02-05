// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::collapsible_if)]

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct LoginDialog {
        pub handle_row: RefCell<Option<adw::EntryRow>>,
        pub password_row: RefCell<Option<adw::PasswordEntryRow>>,
        pub login_button: RefCell<Option<gtk4::Button>>,
        pub spinner: RefCell<Option<gtk4::Spinner>>,
        pub error_label: RefCell<Option<gtk4::Label>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoginDialog {
        const NAME: &'static str = "HangarLoginDialog";
        type Type = super::LoginDialog;
        type ParentType = adw::Dialog;
    }

    impl ObjectImpl for LoginDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_ui();
        }
    }

    impl WidgetImpl for LoginDialog {}
    impl AdwDialogImpl for LoginDialog {}
}

glib::wrapper! {
    pub struct LoginDialog(ObjectSubclass<imp::LoginDialog>)
        @extends adw::Dialog, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl LoginDialog {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup_ui(&self) {
        self.set_title("Hangar");
        self.set_content_width(400);
        // Let dialog auto-size height based on content

        // Main content box
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(24);
        content.set_margin_bottom(24);

        // App title
        let title = gtk4::Label::new(Some("Sign In to Bluesky"));
        title.add_css_class("title-1");
        content.append(&title);

        // Description
        let desc = gtk4::Label::new(Some(
            "Enter your Bluesky handle and app password to sign in.",
        ));
        desc.set_wrap(true);
        desc.set_justify(gtk4::Justification::Center);
        desc.add_css_class("dim-label");
        content.append(&desc);

        // Preferences group with entry rows (GNOME HIG pattern)
        let prefs_group = adw::PreferencesGroup::new();

        let handle_row = adw::EntryRow::new();
        handle_row.set_title("Handle");
        handle_row.set_input_purpose(gtk4::InputPurpose::Email);
        handle_row.set_text("yourname.bsky.social");
        handle_row.set_show_apply_button(false);
        prefs_group.add(&handle_row);

        let password_row = adw::PasswordEntryRow::new();
        password_row.set_title("App Password");
        prefs_group.add(&password_row);

        content.append(&prefs_group);

        // App password help link
        let app_password_link = gtk4::Button::with_label("Create an App Password →");
        app_password_link.add_css_class("flat");
        app_password_link.add_css_class("link");
        app_password_link.set_halign(gtk4::Align::Center);
        app_password_link.connect_clicked(|_| {
            let _ = open::that("https://bsky.app/settings/app-passwords");
        });
        content.append(&app_password_link);

        // Error label (hidden by default)
        let error_label = gtk4::Label::new(None);
        error_label.set_halign(gtk4::Align::Center);
        error_label.add_css_class("error");
        error_label.set_visible(false);
        error_label.set_wrap(true);
        content.append(&error_label);

        // Button box with spinner
        let button_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        button_box.set_halign(gtk4::Align::Center);
        button_box.set_margin_top(12);

        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        button_box.append(&spinner);

        let cancel_btn = gtk4::Button::with_label("Cancel");
        cancel_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| {
                dialog.close();
            }
        ));
        button_box.append(&cancel_btn);

        let login_button = gtk4::Button::with_label("Sign In");
        login_button.add_css_class("suggested-action");
        login_button.set_sensitive(false);
        button_box.append(&login_button);

        content.append(&button_box);

        // Privacy link
        let privacy_link = gtk4::Button::with_label("Privacy & Security");
        privacy_link.add_css_class("flat");
        privacy_link.add_css_class("link");
        privacy_link.add_css_class("dim-label");
        privacy_link.set_halign(gtk4::Align::Center);
        privacy_link.set_margin_top(8);
        privacy_link.connect_clicked(|_| {
            let _ = open::that("https://hangar.blue/privacy/");
        });
        content.append(&privacy_link);

        // Connect entry changes to enable/disable login button
        let login_btn_weak = login_button.downgrade();
        let handle_row_weak = handle_row.downgrade();
        let password_row_weak = password_row.downgrade();

        let update_button_sensitivity = move || {
            if let (Some(btn), Some(handle), Some(pass)) = (
                login_btn_weak.upgrade(),
                handle_row_weak.upgrade(),
                password_row_weak.upgrade(),
            ) {
                let handle_text = handle.text();
                let pass_text = pass.text();
                btn.set_sensitive(!handle_text.is_empty() && !pass_text.is_empty());
            }
        };

        let update_fn = update_button_sensitivity.clone();
        handle_row.connect_changed(move |_| update_fn());

        let update_fn = update_button_sensitivity.clone();
        password_row.connect_changed(move |_| update_fn());

        // Also enable login on Enter key
        let login_btn_weak2 = login_button.downgrade();
        handle_row.connect_entry_activated(move |_| {
            if let Some(btn) = login_btn_weak2.upgrade() {
                if btn.is_sensitive() {
                    btn.emit_clicked();
                }
            }
        });

        let login_btn_weak3 = login_button.downgrade();
        password_row.connect_entry_activated(move |_| {
            if let Some(btn) = login_btn_weak3.upgrade() {
                if btn.is_sensitive() {
                    btn.emit_clicked();
                }
            }
        });

        // Store references
        let imp = self.imp();
        imp.handle_row.replace(Some(handle_row));
        imp.password_row.replace(Some(password_row));
        imp.login_button.replace(Some(login_button));
        imp.spinner.replace(Some(spinner));
        imp.error_label.replace(Some(error_label));

        self.set_child(Some(&content));
    }

    pub fn handle(&self) -> String {
        self.imp()
            .handle_row
            .borrow()
            .as_ref()
            .map(|e| e.text().to_string())
            .unwrap_or_default()
    }

    pub fn password(&self) -> String {
        self.imp()
            .password_row
            .borrow()
            .as_ref()
            .map(|e| e.text().to_string())
            .unwrap_or_default()
    }

    pub fn show_error(&self, message: &str) {
        if let Some(label) = self.imp().error_label.borrow().as_ref() {
            label.set_text(message);
            label.set_visible(true);
        }
    }

    pub fn hide_error(&self) {
        if let Some(label) = self.imp().error_label.borrow().as_ref() {
            label.set_visible(false);
        }
    }

    pub fn set_loading(&self, loading: bool) {
        let imp = self.imp();

        if let Some(spinner) = imp.spinner.borrow().as_ref() {
            spinner.set_visible(loading);
            if loading {
                spinner.start();
            } else {
                spinner.stop();
            }
        }

        if let Some(button) = imp.login_button.borrow().as_ref() {
            button.set_sensitive(!loading);
        }

        if let Some(handle) = imp.handle_row.borrow().as_ref() {
            handle.set_sensitive(!loading);
        }

        if let Some(password) = imp.password_row.borrow().as_ref() {
            password.set_sensitive(!loading);
        }
    }

    pub fn connect_login<F: Fn(&Self) + 'static>(&self, f: F) {
        if let Some(button) = self.imp().login_button.borrow().as_ref() {
            let dialog = self.clone();
            button.connect_clicked(move |_| {
                f(&dialog);
            });
        }
    }
}

impl Default for LoginDialog {
    fn default() -> Self {
        Self::new()
    }
}
