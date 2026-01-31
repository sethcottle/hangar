// SPDX-License-Identifier: MPL-2.0

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct ComposeDialog {
        pub text_view: RefCell<Option<gtk4::TextView>>,
        pub post_button: RefCell<Option<gtk4::Button>>,
        pub error_label: RefCell<Option<gtk4::Label>>,
        pub post_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ComposeDialog {
        const NAME: &'static str = "HangarComposeDialog";
        type Type = super::ComposeDialog;
        type ParentType = gtk4::Window;
    }

    impl ObjectImpl for ComposeDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_ui();
        }
    }

    impl WidgetImpl for ComposeDialog {}
    impl WindowImpl for ComposeDialog {}
}

glib::wrapper! {
    pub struct ComposeDialog(ObjectSubclass<imp::ComposeDialog>)
        @extends gtk4::Window, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget,
                    gtk4::Native, gtk4::Root, gtk4::ShortcutManager;
}

impl ComposeDialog {
    pub fn new(parent: &impl IsA<gtk4::Window>) -> Self {
        glib::Object::builder()
            .property("title", "New Post")
            .property("modal", true)
            .property("transient-for", parent)
            .property("default-width", 420)
            .property("default-height", 280)
            .property("resizable", true)
            .build()
    }

    fn setup_ui(&self) {
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

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

        let post_btn = gtk4::Button::with_label("Post");
        post_btn.add_css_class("suggested-action");
        header.pack_end(&post_btn);

        content.append(&header);

        let form_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        form_box.set_margin_start(12);
        form_box.set_margin_end(12);
        form_box.set_margin_top(12);
        form_box.set_margin_bottom(12);
        form_box.set_vexpand(true);

        let text_view = gtk4::TextView::new();
        text_view.set_wrap_mode(gtk4::WrapMode::WordChar);
        text_view.set_vexpand(true);
        text_view.set_left_margin(8);
        text_view.set_right_margin(8);
        text_view.set_top_margin(8);
        text_view.set_bottom_margin(8);

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_min_content_height(120);
        scrolled.set_child(Some(&text_view));

        form_box.append(&scrolled);

        let error_label = gtk4::Label::new(None);
        error_label.set_halign(gtk4::Align::Start);
        error_label.add_css_class("dim-label");
        error_label.add_css_class("error");
        error_label.set_visible(false);
        form_box.append(&error_label);

        content.append(&form_box);

        self.set_child(Some(&content));

        let imp = self.imp();
        imp.text_view.replace(Some(text_view));
        imp.post_button.replace(Some(post_btn));
        imp.error_label.replace(Some(error_label));

        let dialog_weak = self.downgrade();
        if let Some(btn) = imp.post_button.borrow().as_ref() {
            btn.connect_clicked(move |_| {
                if let Some(dialog) = dialog_weak.upgrade() {
                    dialog.emit_post();
                }
            });
        }
    }

    fn emit_post(&self) {
        let text = self
            .imp()
            .text_view
            .borrow()
            .as_ref()
            .map(|tv| {
                let buffer = tv.buffer();
                let start = buffer.start_iter();
                let end = buffer.end_iter();
                buffer.text(&start, &end, false).to_string()
            })
            .unwrap_or_default()
            .trim()
            .to_string();

        if text.is_empty() {
            return;
        }

        if let Some(cb) = self.imp().post_callback.borrow().as_ref() {
            cb(text);
        }
    }

    pub fn connect_post<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp()
            .post_callback
            .replace(Some(Box::new(callback)));
    }

    pub fn set_loading(&self, loading: bool) {
        if let Some(btn) = self.imp().post_button.borrow().as_ref() {
            btn.set_sensitive(!loading);
        }
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
}
