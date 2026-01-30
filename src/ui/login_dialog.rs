// SPDX-License-Identifier: MPL-2.0

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct LoginDialog {
        pub handle_entry: RefCell<Option<gtk4::Entry>>,
        pub password_entry: RefCell<Option<gtk4::PasswordEntry>>,
        pub login_button: RefCell<Option<gtk4::Button>>,
        pub spinner: RefCell<Option<gtk4::Spinner>>,
        pub error_label: RefCell<Option<gtk4::Label>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoginDialog {
        const NAME: &'static str = "HangarLoginDialog";
        type Type = super::LoginDialog;
        type ParentType = gtk4::Window;
    }

    impl ObjectImpl for LoginDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_ui();
        }
    }

    impl WidgetImpl for LoginDialog {}
    impl WindowImpl for LoginDialog {}
}

glib::wrapper! {
    pub struct LoginDialog(ObjectSubclass<imp::LoginDialog>)
        @extends gtk4::Window, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget,
                    gtk4::Native, gtk4::Root, gtk4::ShortcutManager;
}

impl LoginDialog {
    pub fn new(parent: &impl IsA<gtk4::Window>) -> Self {
        glib::Object::builder()
            .property("title", "Sign In to Bluesky")
            .property("modal", true)
            .property("transient-for", parent)
            .property("default-width", 400)
            .property("default-height", 320)
            .property("resizable", false)
            .build()
    }

    fn setup_ui(&self) {
        // Main content box
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Header bar
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

        content.append(&header);

        // Form content
        let form_box = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        form_box.set_margin_start(24);
        form_box.set_margin_end(24);
        form_box.set_margin_top(16);
        form_box.set_margin_bottom(24);

        // Description
        let desc = gtk4::Label::new(Some(
            "Enter your Bluesky handle and app password to sign in.",
        ));
        desc.set_wrap(true);
        desc.set_halign(gtk4::Align::Start);
        desc.add_css_class("dim-label");
        form_box.append(&desc);

        // Handle entry with label
        let handle_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        let handle_label = gtk4::Label::new(Some("Handle"));
        handle_label.set_halign(gtk4::Align::Start);
        handle_label.add_css_class("dim-label");
        handle_box.append(&handle_label);

        let handle_entry = gtk4::Entry::new();
        handle_entry.set_placeholder_text(Some("yourname.bsky.social"));
        handle_entry.set_input_purpose(gtk4::InputPurpose::Email);
        handle_box.append(&handle_entry);
        form_box.append(&handle_box);

        // Password entry with label
        let password_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        let password_label = gtk4::Label::new(Some("App Password"));
        password_label.set_halign(gtk4::Align::Start);
        password_label.add_css_class("dim-label");
        password_box.append(&password_label);

        let password_entry = gtk4::PasswordEntry::new();
        password_entry.set_show_peek_icon(true);
        password_box.append(&password_entry);
        form_box.append(&password_box);

        // Error label (hidden by default)
        let error_label = gtk4::Label::new(None);
        error_label.set_halign(gtk4::Align::Start);
        error_label.add_css_class("error");
        error_label.set_visible(false);
        error_label.set_wrap(true);
        form_box.append(&error_label);

        // Button box with spinner
        let button_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);
        button_box.set_margin_top(8);

        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        button_box.append(&spinner);

        let login_button = gtk4::Button::with_label("Sign In");
        login_button.add_css_class("suggested-action");
        login_button.set_sensitive(false);
        button_box.append(&login_button);

        form_box.append(&button_box);

        content.append(&form_box);

        // Connect entry changes to enable/disable login button
        let login_btn_weak = login_button.downgrade();
        let handle_entry_weak = handle_entry.downgrade();
        let password_entry_weak = password_entry.downgrade();

        let update_button_sensitivity = move || {
            if let (Some(btn), Some(handle), Some(pass)) = (
                login_btn_weak.upgrade(),
                handle_entry_weak.upgrade(),
                password_entry_weak.upgrade(),
            ) {
                let handle_text = handle.text();
                let pass_text = pass.text();
                btn.set_sensitive(!handle_text.is_empty() && !pass_text.is_empty());
            }
        };

        let update_fn = update_button_sensitivity.clone();
        handle_entry.connect_changed(move |_| update_fn());

        let update_fn = update_button_sensitivity;
        password_entry.connect_changed(move |_| update_fn());

        // Store references
        let imp = self.imp();
        imp.handle_entry.replace(Some(handle_entry));
        imp.password_entry.replace(Some(password_entry));
        imp.login_button.replace(Some(login_button));
        imp.spinner.replace(Some(spinner));
        imp.error_label.replace(Some(error_label));

        self.set_child(Some(&content));
    }

    pub fn handle(&self) -> String {
        self.imp()
            .handle_entry
            .borrow()
            .as_ref()
            .map(|e| e.text().to_string())
            .unwrap_or_default()
    }

    pub fn password(&self) -> String {
        self.imp()
            .password_entry
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

        if let Some(handle) = imp.handle_entry.borrow().as_ref() {
            handle.set_sensitive(!loading);
        }

        if let Some(password) = imp.password_entry.borrow().as_ref() {
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
        panic!("LoginDialog requires a parent window")
    }
}
