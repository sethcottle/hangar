// SPDX-License-Identifier: MPL-2.0

use crate::atproto::Post;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct PostRow {
        pub post: RefCell<Option<Post>>,
        pub avatar: RefCell<Option<adw::Avatar>>,
        pub display_name_label: RefCell<Option<gtk4::Label>>,
        pub handle_label: RefCell<Option<gtk4::Label>>,
        pub timestamp_label: RefCell<Option<gtk4::Label>>,
        pub content_label: RefCell<Option<gtk4::Label>>,
        pub reply_count_label: RefCell<Option<gtk4::Label>>,
        pub repost_count_label: RefCell<Option<gtk4::Label>>,
        pub like_count_label: RefCell<Option<gtk4::Label>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PostRow {
        const NAME: &'static str = "HangarPostRow";
        type Type = super::PostRow;
        type ParentType = gtk4::Box;
    }

    impl ObjectImpl for PostRow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_ui();
        }
    }

    impl WidgetImpl for PostRow {}
    impl BoxImpl for PostRow {}
}

glib::wrapper! {
    pub struct PostRow(ObjectSubclass<imp::PostRow>)
        @extends gtk4::Box, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
}

impl PostRow {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk4::Orientation::Vertical)
            .property("spacing", 8)
            .build()
    }

    fn setup_ui(&self) {
        self.set_margin_start(16);
        self.set_margin_end(16);
        self.set_margin_top(12);
        self.set_margin_bottom(12);
        self.add_css_class("card");

        // Header row: avatar + name + handle + time
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);

        let avatar = adw::Avatar::new(40, None, true);
        header.append(&avatar);

        let name_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        name_box.set_hexpand(true);

        let display_name = gtk4::Label::new(None);
        display_name.set_halign(gtk4::Align::Start);
        display_name.add_css_class("heading");
        display_name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        name_box.append(&display_name);

        let handle = gtk4::Label::new(None);
        handle.set_halign(gtk4::Align::Start);
        handle.add_css_class("dim-label");
        handle.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        name_box.append(&handle);

        header.append(&name_box);

        let timestamp = gtk4::Label::new(None);
        timestamp.add_css_class("dim-label");
        header.append(&timestamp);

        self.append(&header);

        // Post content
        let content = gtk4::Label::new(None);
        content.set_halign(gtk4::Align::Start);
        content.set_wrap(true);
        content.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        content.set_selectable(true);
        content.set_xalign(0.0);
        self.append(&content);

        // Action bar
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
        actions.set_margin_top(8);

        let (reply_btn, reply_count) = Self::create_action_button("comment-symbolic");
        actions.append(&reply_btn);

        let (repost_btn, repost_count) =
            Self::create_action_button("media-playlist-repeat-symbolic");
        actions.append(&repost_btn);

        let (like_btn, like_count) = Self::create_action_button("emblem-favorite-symbolic");
        actions.append(&like_btn);

        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        actions.append(&spacer);

        let menu_btn = gtk4::Button::from_icon_name("view-more-symbolic");
        menu_btn.add_css_class("flat");
        menu_btn.add_css_class("circular");
        actions.append(&menu_btn);

        self.append(&actions);

        // Store references
        let imp = self.imp();
        imp.avatar.replace(Some(avatar));
        imp.display_name_label.replace(Some(display_name));
        imp.handle_label.replace(Some(handle));
        imp.timestamp_label.replace(Some(timestamp));
        imp.content_label.replace(Some(content));
        imp.reply_count_label.replace(Some(reply_count));
        imp.repost_count_label.replace(Some(repost_count));
        imp.like_count_label.replace(Some(like_count));
    }

    fn create_action_button(icon_name: &str) -> (gtk4::Box, gtk4::Label) {
        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

        let btn = gtk4::Button::from_icon_name(icon_name);
        btn.add_css_class("flat");
        btn.add_css_class("circular");
        btn_box.append(&btn);

        let count_label = gtk4::Label::new(Some("0"));
        count_label.add_css_class("dim-label");
        btn_box.append(&count_label);

        (btn_box, count_label)
    }

    pub fn bind(&self, post: &Post) {
        let imp = self.imp();
        imp.post.replace(Some(post.clone()));

        let display_name = post
            .author
            .display_name
            .as_deref()
            .unwrap_or(&post.author.handle);

        if let Some(avatar) = imp.avatar.borrow().as_ref() {
            avatar.set_text(Some(display_name));
        }

        if let Some(label) = imp.display_name_label.borrow().as_ref() {
            label.set_text(display_name);
        }

        if let Some(label) = imp.handle_label.borrow().as_ref() {
            label.set_text(&format!("@{}", post.author.handle));
        }

        if let Some(label) = imp.timestamp_label.borrow().as_ref() {
            label.set_text(&Self::format_timestamp(&post.indexed_at));
        }

        if let Some(label) = imp.content_label.borrow().as_ref() {
            label.set_text(&post.text);
        }

        if let Some(label) = imp.reply_count_label.borrow().as_ref() {
            label.set_text(&Self::format_count(post.reply_count));
        }

        if let Some(label) = imp.repost_count_label.borrow().as_ref() {
            label.set_text(&Self::format_count(post.repost_count));
        }

        if let Some(label) = imp.like_count_label.borrow().as_ref() {
            label.set_text(&Self::format_count(post.like_count));
        }
    }

    fn format_count(count: Option<u32>) -> String {
        match count {
            Some(c) if c >= 1_000_000 => format!("{:.1}M", c as f64 / 1_000_000.0),
            Some(c) if c >= 1_000 => format!("{:.1}K", c as f64 / 1_000.0),
            Some(c) => c.to_string(),
            None => "0".to_string(),
        }
    }

    fn format_timestamp(indexed_at: &str) -> String {
        if indexed_at.is_empty() {
            return String::new();
        }

        let Ok(post_time) = chrono::DateTime::parse_from_rfc3339(indexed_at) else {
            return String::new();
        };

        let now = chrono::Utc::now();
        let duration = now.signed_duration_since(post_time);

        if duration.num_seconds() < 60 {
            "now".to_string()
        } else if duration.num_minutes() < 60 {
            format!("{}m", duration.num_minutes())
        } else if duration.num_hours() < 24 {
            format!("{}h", duration.num_hours())
        } else if duration.num_days() < 7 {
            format!("{}d", duration.num_days())
        } else {
            post_time.format("%b %d").to_string()
        }
    }
}

impl Default for PostRow {
    fn default() -> Self {
        Self::new()
    }
}
