// SPDX-License-Identifier: MPL-2.0

use crate::atproto::Post;
use crate::ui::avatar_cache;
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
        pub like_btn: RefCell<Option<gtk4::Button>>,
        pub repost_btn: RefCell<Option<gtk4::Button>>,
        pub like_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub repost_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub reply_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub reply_btn: RefCell<Option<gtk4::Button>>,
        pub images_box: RefCell<Option<gtk4::FlowBox>>,
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

        // Images (embed thumbnails)
        let images_box = gtk4::FlowBox::new();
        images_box.set_selection_mode(gtk4::SelectionMode::None);
        images_box.set_max_children_per_line(3);
        images_box.set_min_children_per_line(1);
        images_box.set_homogeneous(false);
        images_box.set_row_spacing(8);
        images_box.set_column_spacing(8);
        images_box.add_css_class("post-images");
        self.append(&images_box);

        // Action bar
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
        actions.set_margin_top(8);

        let (reply_btn, reply_count, reply_btn_ref) =
            Self::create_action_button("mail-reply-sender-symbolic");
        actions.append(&reply_btn);

        let (repost_btn, repost_count, repost_btn_ref) =
            Self::create_action_button("media-playlist-repeat-symbolic");
        actions.append(&repost_btn);

        let (like_btn, like_count, like_btn_ref) =
            Self::create_action_button("heart-outline-symbolic");
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
        imp.like_btn.replace(Some(like_btn_ref));
        imp.repost_btn.replace(Some(repost_btn_ref));
        imp.reply_btn.replace(Some(reply_btn_ref));
        imp.images_box.replace(Some(images_box));
    }

    fn create_action_button(icon_name: &str) -> (gtk4::Box, gtk4::Label, gtk4::Button) {
        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

        let btn = gtk4::Button::from_icon_name(icon_name);
        btn.add_css_class("flat");
        btn.add_css_class("circular");
        btn_box.append(&btn);

        let count_label = gtk4::Label::new(Some("0"));
        count_label.add_css_class("dim-label");
        btn_box.append(&count_label);

        (btn_box, count_label, btn)
    }

    pub fn connect_like_clicked<F: Fn() + 'static>(&self, f: F) {
        let imp = self.imp();
        if let Some(id) = imp.like_handler_id.take() {
            if let Some(btn) = imp.like_btn.borrow().as_ref() {
                btn.disconnect(id);
            }
        }
        if let Some(btn) = imp.like_btn.borrow().as_ref() {
            let id = btn.connect_clicked(move |_| f());
            imp.like_handler_id.replace(Some(id));
        }
    }

    pub fn connect_repost_clicked<F: Fn() + 'static>(&self, f: F) {
        let imp = self.imp();
        if let Some(id) = imp.repost_handler_id.take() {
            if let Some(btn) = imp.repost_btn.borrow().as_ref() {
                btn.disconnect(id);
            }
        }
        if let Some(btn) = imp.repost_btn.borrow().as_ref() {
            let id = btn.connect_clicked(move |_| f());
            imp.repost_handler_id.replace(Some(id));
        }
    }

    pub fn connect_reply_clicked<F: Fn() + 'static>(&self, f: F) {
        let imp = self.imp();
        if let Some(id) = imp.reply_handler_id.take() {
            if let Some(btn) = imp.reply_btn.borrow().as_ref() {
                btn.disconnect(id);
            }
        }
        if let Some(btn) = imp.reply_btn.borrow().as_ref() {
            let id = btn.connect_clicked(move |_| f());
            imp.reply_handler_id.replace(Some(id));
        }
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

            if let Some(avatar_url) = &post.author.avatar {
                avatar_cache::load_avatar(avatar.clone(), avatar_url.clone());
            }
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

        // Clear and repopulate images
        if let Some(images_box) = imp.images_box.borrow().as_ref() {
            while let Some(child) = images_box.first_child() {
                images_box.remove(&child);
            }
            for url in &post.images {
                let picture = gtk4::Picture::new();
                picture.set_keep_aspect_ratio(true);
                picture.set_can_shrink(true);
                picture.set_size_request(200, 200);
                picture.add_css_class("post-embed-image");
                images_box.insert(&picture, -1);
                avatar_cache::load_image_into_picture(picture, url.clone());
            }
            images_box.set_visible(!post.images.is_empty());
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

        // Update like button state
        if let Some(btn) = imp.like_btn.borrow().as_ref() {
            if post.viewer_like.is_some() {
                btn.add_css_class("liked");
            } else {
                btn.remove_css_class("liked");
            }
        }

        // Update repost button state
        if let Some(btn) = imp.repost_btn.borrow().as_ref() {
            if post.viewer_repost.is_some() {
                btn.add_css_class("reposted");
            } else {
                btn.remove_css_class("reposted");
            }
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
