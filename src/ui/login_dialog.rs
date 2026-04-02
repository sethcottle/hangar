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
        pub oauth_button: RefCell<Option<gtk4::Button>>,
        pub login_button: RefCell<Option<gtk4::Button>>,
        pub spinner: RefCell<Option<gtk4::Spinner>>,
        pub error_label: RefCell<Option<gtk4::Label>>,
        pub oauth_status: RefCell<Option<gtk4::Box>>,
        pub oauth_cancel_button: RefCell<Option<gtk4::Button>>,
        pub app_password_expander: RefCell<Option<gtk4::Expander>>,
        pub main_content: RefCell<Option<gtk4::Box>>,
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
        self.set_title("Sign In to Bluesky");
        self.set_content_width(400);

        // Header bar with Cancel (start) — GNOME HIG pattern
        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let cancel_btn = gtk4::Button::with_label("Cancel");
        cancel_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| {
                dialog.close();
            }
        ));
        header.pack_start(&cancel_btn);

        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        spinner.update_property(&[gtk4::accessible::Property::Label("Signing in")]);
        header.pack_end(&spinner);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);

        // Main content box
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(24);
        content.set_margin_bottom(24);

        // Description
        let desc = gtk4::Label::new(Some("Enter your Bluesky handle to sign in."));
        desc.set_wrap(true);
        desc.set_justify(gtk4::Justification::Center);
        desc.add_css_class("dim-label");
        content.append(&desc);

        // Handle entry (used for both OAuth and app password)
        let handle_group = adw::PreferencesGroup::new();
        let handle_row = adw::EntryRow::new();
        handle_row.set_title("Handle");
        handle_row.set_input_purpose(gtk4::InputPurpose::Email);
        handle_row.set_show_apply_button(false);
        handle_group.add(&handle_row);
        content.append(&handle_group);

        // Primary OAuth button
        let oauth_button = gtk4::Button::with_label("Sign in with Bluesky");
        oauth_button.add_css_class("suggested-action");
        oauth_button.set_sensitive(false);
        oauth_button.set_tooltip_text(Some("Sign in with Bluesky (opens browser)"));
        oauth_button.update_property(&[gtk4::accessible::Property::Label(
            "Sign in with Bluesky (opens browser)",
        )]);
        content.append(&oauth_button);

        // OAuth waiting status (hidden by default)
        let oauth_status = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        oauth_status.set_visible(false);
        oauth_status.set_halign(gtk4::Align::Center);

        let status_spinner = gtk4::Spinner::new();
        status_spinner.start();
        status_spinner.set_width_request(32);
        status_spinner.set_height_request(32);
        oauth_status.append(&status_spinner);

        let status_label = gtk4::Label::new(Some("Complete sign-in in your browser..."));
        status_label.add_css_class("dim-label");
        status_label.update_property(&[gtk4::accessible::Property::Label(
            "Waiting for sign-in to complete in browser",
        )]);
        oauth_status.append(&status_label);

        let oauth_cancel_btn = gtk4::Button::with_label("Cancel");
        oauth_cancel_btn.add_css_class("destructive-action");
        oauth_cancel_btn.set_margin_top(4);
        oauth_cancel_btn.set_tooltip_text(Some("Cancel sign-in"));
        oauth_cancel_btn.update_property(&[gtk4::accessible::Property::Label("Cancel sign-in")]);
        oauth_status.append(&oauth_cancel_btn);

        content.append(&oauth_status);

        // Error label (hidden by default)
        let error_label = gtk4::Label::new(None);
        error_label.set_halign(gtk4::Align::Center);
        error_label.add_css_class("error");
        error_label.set_visible(false);
        error_label.set_wrap(true);
        content.append(&error_label);

        // App Password fallback (collapsible)
        let expander = gtk4::Expander::new(Some("Use App Password instead"));
        expander.add_css_class("dim-label");

        let password_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        password_box.set_margin_top(8);

        let password_group = adw::PreferencesGroup::new();
        let password_row = adw::PasswordEntryRow::new();
        password_row.set_title("App Password");
        password_group.add(&password_row);
        password_box.append(&password_group);

        let login_button = gtk4::Button::with_label("Sign In");
        login_button.add_css_class("suggested-action");
        login_button.set_sensitive(false);
        login_button.set_tooltip_text(Some("Sign in with app password"));
        login_button.update_property(&[gtk4::accessible::Property::Label(
            "Sign in with app password",
        )]);
        password_box.append(&login_button);

        // App password help link
        let app_password_link = gtk4::Button::with_label("Create an App Password →");
        app_password_link.add_css_class("flat");
        app_password_link.add_css_class("link");
        app_password_link.set_halign(gtk4::Align::Start);
        app_password_link.update_property(&[gtk4::accessible::Property::Label(
            "Create an App Password (opens in browser)",
        )]);
        app_password_link.connect_clicked(|_| {
            let _ = open::that("https://bsky.app/settings/app-passwords");
        });
        password_box.append(&app_password_link);

        expander.set_child(Some(&password_box));
        content.append(&expander);

        // Privacy link at bottom
        let privacy_link = gtk4::Button::with_label("Privacy & Security →");
        privacy_link.add_css_class("flat");
        privacy_link.add_css_class("link");
        privacy_link.set_halign(gtk4::Align::Center);
        privacy_link.update_property(&[gtk4::accessible::Property::Label(
            "Privacy & Security (opens in browser)",
        )]);
        privacy_link.connect_clicked(|_| {
            let _ = open::that("https://hangar.blue/privacy/");
        });
        content.append(&privacy_link);

        toolbar.set_content(Some(&content));

        // Enable OAuth button when handle is filled
        let oauth_btn_weak = oauth_button.downgrade();
        let handle_row_weak = handle_row.downgrade();
        handle_row.connect_changed(move |_| {
            if let (Some(btn), Some(handle)) = (oauth_btn_weak.upgrade(), handle_row_weak.upgrade())
            {
                btn.set_sensitive(!handle.text().is_empty());
            }
        });

        // Enable app password Sign In button when both handle and password filled
        let login_btn_weak = login_button.downgrade();
        let handle_row_weak2 = handle_row.downgrade();
        let password_row_weak = password_row.downgrade();
        let update_password_btn = move || {
            if let (Some(btn), Some(handle), Some(pass)) = (
                login_btn_weak.upgrade(),
                handle_row_weak2.upgrade(),
                password_row_weak.upgrade(),
            ) {
                btn.set_sensitive(!handle.text().is_empty() && !pass.text().is_empty());
            }
        };
        let update_fn = update_password_btn.clone();
        handle_row.connect_changed(move |_| update_fn());
        let update_fn = update_password_btn.clone();
        password_row.connect_changed(move |_| update_fn());

        // Enter key on handle activates OAuth button
        let oauth_btn_weak2 = oauth_button.downgrade();
        handle_row.connect_entry_activated(move |_| {
            if let Some(btn) = oauth_btn_weak2.upgrade() {
                if btn.is_sensitive() {
                    btn.emit_clicked();
                }
            }
        });

        // Enter key on password activates Sign In button
        let login_btn_weak2 = login_button.downgrade();
        password_row.connect_entry_activated(move |_| {
            if let Some(btn) = login_btn_weak2.upgrade() {
                if btn.is_sensitive() {
                    btn.emit_clicked();
                }
            }
        });

        // Store references
        let imp = self.imp();
        imp.handle_row.replace(Some(handle_row));
        imp.password_row.replace(Some(password_row));
        imp.oauth_button.replace(Some(oauth_button));
        imp.login_button.replace(Some(login_button));
        imp.spinner.replace(Some(spinner));
        imp.error_label.replace(Some(error_label));
        imp.oauth_status.replace(Some(oauth_status));
        imp.oauth_cancel_button.replace(Some(oauth_cancel_btn));
        imp.app_password_expander.replace(Some(expander));
        imp.main_content.replace(Some(content));

        self.set_child(Some(&toolbar));
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

    /// Show/hide the OAuth waiting state (spinner + status message).
    pub fn set_oauth_waiting(&self, waiting: bool) {
        let imp = self.imp();

        if let Some(status) = imp.oauth_status.borrow().as_ref() {
            status.set_visible(waiting);
        }

        if let Some(btn) = imp.oauth_button.borrow().as_ref() {
            btn.set_sensitive(!waiting);
            btn.set_visible(!waiting);
        }

        if let Some(handle) = imp.handle_row.borrow().as_ref() {
            handle.set_sensitive(!waiting);
        }

        if let Some(expander) = imp.app_password_expander.borrow().as_ref() {
            expander.set_sensitive(!waiting);
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

    /// Connect callback for OAuth login (handle only).
    pub fn connect_oauth_login<F: Fn(&Self) + 'static>(&self, f: F) {
        if let Some(button) = self.imp().oauth_button.borrow().as_ref() {
            let dialog = self.clone();
            button.connect_clicked(move |_| {
                f(&dialog);
            });
        }
    }

    /// Connect callback for cancelling an in-progress OAuth flow.
    pub fn connect_oauth_cancel<F: Fn(&Self) + 'static>(&self, f: F) {
        if let Some(button) = self.imp().oauth_cancel_button.borrow().as_ref() {
            let dialog = self.clone();
            button.connect_clicked(move |_| {
                f(&dialog);
            });
        }
    }

    /// Connect callback for app-password login (handle + password).
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
