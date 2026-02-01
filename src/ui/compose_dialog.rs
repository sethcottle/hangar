// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

/// Context for replying to a post
#[derive(Clone)]
pub struct ReplyContext {
    pub uri: String,
    pub cid: String,
    pub author_handle: String,
}

/// Context for quoting a post
#[derive(Clone)]
pub struct QuoteContext {
    pub uri: String,
    pub cid: String,
    pub author_handle: String,
    pub text: String,
}

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct ComposeDialog {
        pub text_view: RefCell<Option<gtk4::TextView>>,
        pub post_button: RefCell<Option<gtk4::Button>>,
        pub error_label: RefCell<Option<gtk4::Label>>,
        pub reply_context: RefCell<Option<ReplyContext>>,
        pub quote_context: RefCell<Option<QuoteContext>>,
        pub reply_label: RefCell<Option<gtk4::Label>>,
        pub quote_preview: RefCell<Option<gtk4::Box>>,
        pub post_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
        pub reply_callback: RefCell<Option<Box<dyn Fn(String, String, String) + 'static>>>,
        pub quote_callback: RefCell<Option<Box<dyn Fn(String, String, String) + 'static>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ComposeDialog {
        const NAME: &'static str = "HangarComposeDialog";
        type Type = super::ComposeDialog;
        type ParentType = adw::Dialog;
    }

    impl ObjectImpl for ComposeDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_ui();
        }
    }

    impl WidgetImpl for ComposeDialog {}
    impl AdwDialogImpl for ComposeDialog {}
}

glib::wrapper! {
    pub struct ComposeDialog(ObjectSubclass<imp::ComposeDialog>)
        @extends adw::Dialog, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl ComposeDialog {
    pub fn new() -> Self {
        let dialog: Self = glib::Object::builder().build();
        dialog.set_title("New Post");
        dialog.set_content_width(420);
        dialog.set_content_height(280);
        dialog
    }

    pub fn new_reply(context: ReplyContext) -> Self {
        let dialog: Self = glib::Object::builder().build();
        dialog.set_title("Reply");
        dialog.set_content_width(420);
        dialog.set_content_height(280);
        dialog.set_reply_context(context);
        dialog
    }

    pub fn new_quote(context: QuoteContext) -> Self {
        let dialog: Self = glib::Object::builder().build();
        dialog.set_title("Quote Post");
        dialog.set_content_width(420);
        dialog.set_content_height(340);
        dialog.set_quote_context(context);
        dialog
    }

    fn set_reply_context(&self, context: ReplyContext) {
        let imp = self.imp();
        if let Some(label) = imp.reply_label.borrow().as_ref() {
            label.set_text(&format!("Replying to @{}", context.author_handle));
            label.set_visible(true);
        }
        imp.reply_context.replace(Some(context));
    }

    fn set_quote_context(&self, context: QuoteContext) {
        let imp = self.imp();
        // Show quote preview card
        if let Some(preview) = imp.quote_preview.borrow().as_ref() {
            // Clear existing children
            while let Some(child) = preview.first_child() {
                preview.remove(&child);
            }

            let header = gtk4::Label::new(Some(&format!("@{}", context.author_handle)));
            header.set_halign(gtk4::Align::Start);
            header.add_css_class("dim-label");
            header.add_css_class("caption");
            preview.append(&header);

            // Show truncated text
            let text = if context.text.len() > 100 {
                format!("{}...", &context.text[..100])
            } else {
                context.text.clone()
            };
            let text_label = gtk4::Label::new(Some(&text));
            text_label.set_halign(gtk4::Align::Start);
            text_label.set_wrap(true);
            text_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            text_label.add_css_class("caption");
            preview.append(&text_label);

            preview.set_visible(true);
        }
        imp.quote_context.replace(Some(context));
    }

    fn setup_ui(&self) {
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_vexpand(true);

        let reply_label = gtk4::Label::new(None);
        reply_label.set_halign(gtk4::Align::Start);
        reply_label.add_css_class("dim-label");
        reply_label.set_visible(false);
        content.append(&reply_label);

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
        content.append(&scrolled);

        // Quote preview card (shown when quoting)
        let quote_preview = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        quote_preview.add_css_class("quote-card");
        quote_preview.set_visible(false);
        content.append(&quote_preview);

        let error_label = gtk4::Label::new(None);
        error_label.set_halign(gtk4::Align::Start);
        error_label.add_css_class("dim-label");
        error_label.add_css_class("error");
        error_label.set_visible(false);
        content.append(&error_label);

        // Button box at bottom
        let button_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        button_box.set_halign(gtk4::Align::End);
        button_box.set_margin_top(12);

        let cancel_btn = gtk4::Button::with_label("Cancel");
        cancel_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| {
                dialog.close();
            }
        ));
        button_box.append(&cancel_btn);

        let post_btn = gtk4::Button::with_label("Post");
        post_btn.add_css_class("suggested-action");
        button_box.append(&post_btn);

        content.append(&button_box);

        self.set_child(Some(&content));

        let imp = self.imp();
        imp.text_view.replace(Some(text_view));
        imp.post_button.replace(Some(post_btn));
        imp.error_label.replace(Some(error_label));
        imp.reply_label.replace(Some(reply_label));
        imp.quote_preview.replace(Some(quote_preview));

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

        let imp = self.imp();
        if let Some(ctx) = imp.reply_context.borrow().as_ref() {
            if let Some(cb) = imp.reply_callback.borrow().as_ref() {
                cb(text, ctx.uri.clone(), ctx.cid.clone());
            }
        } else if let Some(ctx) = imp.quote_context.borrow().as_ref() {
            if let Some(cb) = imp.quote_callback.borrow().as_ref() {
                cb(text, ctx.uri.clone(), ctx.cid.clone());
            }
        } else if let Some(cb) = imp.post_callback.borrow().as_ref() {
            cb(text);
        }
    }

    pub fn connect_post<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp().post_callback.replace(Some(Box::new(callback)));
    }

    pub fn connect_reply<F: Fn(String, String, String) + 'static>(&self, callback: F) {
        self.imp().reply_callback.replace(Some(Box::new(callback)));
    }

    pub fn connect_quote<F: Fn(String, String, String) + 'static>(&self, callback: F) {
        self.imp().quote_callback.replace(Some(Box::new(callback)));
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
