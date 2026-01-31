// SPDX-License-Identifier: MPL-2.0

use crate::atproto::{Embed, ImageEmbed, Post};
use crate::ui::avatar_cache;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;

use crate::atproto::Profile;

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
        pub repost_btn: RefCell<Option<gtk4::MenuButton>>,
        pub like_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub repost_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub quote_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub reply_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub reply_btn: RefCell<Option<gtk4::Button>>,
        pub repost_item: RefCell<Option<gtk4::Button>>,
        pub repost_item_label: RefCell<Option<gtk4::Label>>,
        pub quote_item: RefCell<Option<gtk4::Button>>,
        // New fields for rich embeds and context
        pub repost_row: RefCell<Option<gtk4::Box>>,
        pub reply_indicator: RefCell<Option<gtk4::Label>>,
        pub embed_container: RefCell<Option<gtk4::Box>>,
        // Track current like/repost state (may differ from original post after user actions)
        pub is_liked: RefCell<bool>,
        pub is_reposted: RefCell<bool>,
        pub viewer_like_uri: RefCell<Option<String>>,
        pub viewer_repost_uri: RefCell<Option<String>>,
        // Navigation callbacks
        pub post_clicked_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
        pub profile_clicked_callback: RefCell<Option<Box<dyn Fn(Profile) + 'static>>>,
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
        self.add_css_class("post-row");

        // Repost attribution row (above header, shows "Reposted by X")
        let repost_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        repost_row.set_margin_bottom(8);
        repost_row.add_css_class("repost-attribution");
        let repost_icon = gtk4::Image::from_icon_name("media-playlist-repeat-symbolic");
        repost_icon.add_css_class("dim-label");
        repost_icon.set_pixel_size(14);
        repost_row.append(&repost_icon);
        let repost_label = gtk4::Label::new(None);
        repost_label.add_css_class("dim-label");
        repost_label.add_css_class("caption");
        repost_row.append(&repost_label);
        repost_row.set_visible(false);
        self.append(&repost_row);

        // Header row: avatar + name + handle + time
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);

        // Wrap avatar in a button-like container for clickability
        let avatar_btn = gtk4::Button::new();
        avatar_btn.add_css_class("flat");
        avatar_btn.add_css_class("circular");
        avatar_btn.set_cursor_from_name(Some("pointer"));
        let avatar = adw::Avatar::new(40, None, true);
        avatar_btn.set_child(Some(&avatar));
        header.append(&avatar_btn);

        // Connect avatar click to profile navigation
        let post_row = self.clone();
        avatar_btn.connect_clicked(move |_| {
            let imp = post_row.imp();
            if let Some(post) = imp.post.borrow().as_ref() {
                if let Some(cb) = imp.profile_clicked_callback.borrow().as_ref() {
                    cb(post.author.clone());
                }
            }
        });

        let name_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        name_box.set_hexpand(true);

        // Top row: display name + reply indicator
        let name_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let display_name = gtk4::Label::new(None);
        display_name.set_halign(gtk4::Align::Start);
        display_name.add_css_class("heading");
        display_name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        name_row.append(&display_name);

        // Reply indicator (shows "replying to @handle")
        let reply_indicator = gtk4::Label::new(None);
        reply_indicator.add_css_class("dim-label");
        reply_indicator.add_css_class("caption");
        reply_indicator.set_visible(false);
        name_row.append(&reply_indicator);

        name_box.append(&name_row);

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

        // Content area (clickable to open thread)
        let content_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_area.set_cursor_from_name(Some("pointer"));

        // Post content
        let content = gtk4::Label::new(None);
        content.set_halign(gtk4::Align::Start);
        content.set_wrap(true);
        content.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        content.set_selectable(false); // Disable selection to allow click-through
        content.set_xalign(0.0);
        content_area.append(&content);

        // Unified embed container (images, videos, external cards, quotes)
        let embed_container = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        embed_container.set_margin_top(8);
        embed_container.add_css_class("post-embed");
        embed_container.set_visible(false);
        content_area.append(&embed_container);

        // Add click gesture to content area
        let gesture = gtk4::GestureClick::new();
        let post_row = self.clone();
        gesture.connect_released(move |_, _, _, _| {
            let imp = post_row.imp();
            if let Some(post) = imp.post.borrow().as_ref() {
                if let Some(cb) = imp.post_clicked_callback.borrow().as_ref() {
                    cb(post.clone());
                }
            }
        });
        content_area.add_controller(gesture);

        self.append(&content_area);

        // Action bar
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
        actions.set_margin_top(8);

        let (reply_btn, reply_count, reply_btn_ref) =
            Self::create_action_button("mail-reply-sender-symbolic");
        actions.append(&reply_btn);

        // Repost menu button with popover
        let (repost_btn_box, repost_count, repost_menu_btn, repost_item, repost_item_label, quote_item) =
            Self::create_repost_menu_button();
        actions.append(&repost_btn_box);

        let (like_btn, like_count, like_btn_ref) =
            Self::create_action_button("emote-love-symbolic");
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
        imp.repost_btn.replace(Some(repost_menu_btn));
        imp.repost_item.replace(Some(repost_item));
        imp.repost_item_label.replace(Some(repost_item_label));
        imp.quote_item.replace(Some(quote_item));
        imp.reply_btn.replace(Some(reply_btn_ref));
        imp.repost_row.replace(Some(repost_row));
        imp.reply_indicator.replace(Some(reply_indicator));
        imp.embed_container.replace(Some(embed_container));
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

    /// Create a repost menu button with "Repost"/"Undo Repost" and "Quote" options
    fn create_repost_menu_button() -> (gtk4::Box, gtk4::Label, gtk4::MenuButton, gtk4::Button, gtk4::Label, gtk4::Button) {
        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

        // Create popover content
        let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        popover_box.set_margin_top(8);
        popover_box.set_margin_bottom(8);
        popover_box.set_margin_start(8);
        popover_box.set_margin_end(8);

        let repost_item = gtk4::Button::new();
        let repost_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let repost_icon = gtk4::Image::from_icon_name("media-playlist-repeat-symbolic");
        repost_content.append(&repost_icon);
        let repost_item_label = gtk4::Label::new(Some("Repost"));
        repost_content.append(&repost_item_label);
        repost_item.set_child(Some(&repost_content));
        repost_item.add_css_class("flat");
        popover_box.append(&repost_item);

        let quote_item = gtk4::Button::new();
        let quote_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let quote_icon = gtk4::Image::from_icon_name("mail-reply-sender-symbolic");
        quote_content.append(&quote_icon);
        quote_content.append(&gtk4::Label::new(Some("Quote")));
        quote_item.set_child(Some(&quote_content));
        quote_item.add_css_class("flat");
        popover_box.append(&quote_item);

        let popover = gtk4::Popover::new();
        popover.set_child(Some(&popover_box));
        popover.add_css_class("menu");
        popover.set_has_arrow(false);

        let menu_btn = gtk4::MenuButton::new();
        menu_btn.set_icon_name("media-playlist-repeat-symbolic");
        menu_btn.add_css_class("flat");
        menu_btn.add_css_class("circular");
        menu_btn.set_popover(Some(&popover));

        btn_box.append(&menu_btn);

        let count_label = gtk4::Label::new(Some("0"));
        count_label.add_css_class("dim-label");
        btn_box.append(&count_label);

        (btn_box, count_label, menu_btn, repost_item, repost_item_label, quote_item)
    }

    pub fn connect_like_clicked<F: Fn(&PostRow, bool, Option<String>) + 'static>(&self, f: F) {
        let imp = self.imp();
        if let Some(id) = imp.like_handler_id.take() {
            if let Some(btn) = imp.like_btn.borrow().as_ref() {
                btn.disconnect(id);
            }
        }
        if let Some(btn) = imp.like_btn.borrow().as_ref() {
            // Toggle visual state optimistically when clicked
            let post_row = self.clone();
            let id = btn.connect_clicked(move |_| {
                // Capture current state BEFORE toggling
                let was_liked = post_row.is_liked();
                let like_uri = post_row.viewer_like_uri();
                // Toggle visual state
                post_row.toggle_like_visual();
                // Call callback with self, state info: (was_liked, like_uri_if_unliking)
                f(&post_row, was_liked, like_uri);
            });
            imp.like_handler_id.replace(Some(id));
        }
    }

    /// Toggle the like button visual state (optimistic update)
    pub fn toggle_like_visual(&self) {
        let imp = self.imp();
        let was_liked = *imp.is_liked.borrow();

        if let Some(btn) = imp.like_btn.borrow().as_ref() {
            if was_liked {
                btn.remove_css_class("liked");
                imp.is_liked.replace(false);
                // Clear the like URI since we're unliking
                imp.viewer_like_uri.replace(None);
                // Decrement count
                if let Some(label) = imp.like_count_label.borrow().as_ref() {
                    if let Ok(count) = label.text().parse::<i32>() {
                        label.set_text(&Self::format_count(Some((count - 1).max(0) as u32)));
                    }
                }
            } else {
                btn.add_css_class("liked");
                imp.is_liked.replace(true);
                // Note: we don't have the new like URI yet, but that's OK for visual state
                // Increment count
                if let Some(label) = imp.like_count_label.borrow().as_ref() {
                    if let Ok(count) = label.text().parse::<i32>() {
                        label.set_text(&Self::format_count(Some((count + 1) as u32)));
                    }
                }
            }
        }
    }

    /// Connect callback for the "Repost" menu item
    pub fn connect_repost_clicked<F: Fn(&PostRow, bool, Option<String>) + 'static>(&self, f: F) {
        let imp = self.imp();
        if let Some(id) = imp.repost_handler_id.take() {
            if let Some(btn) = imp.repost_item.borrow().as_ref() {
                btn.disconnect(id);
            }
        }
        if let Some(btn) = imp.repost_item.borrow().as_ref() {
            // Close popover when clicked
            let menu_btn = imp.repost_btn.borrow();
            let popover = menu_btn.as_ref().and_then(|m| m.popover());
            let post_row = self.clone();
            let id = btn.connect_clicked(move |_| {
                if let Some(p) = &popover {
                    p.popdown();
                }
                // Capture current state BEFORE toggling
                let was_reposted = post_row.is_reposted();
                let repost_uri = post_row.viewer_repost_uri();
                // Toggle visual state
                post_row.toggle_repost_visual();
                // Call callback with self, state info
                f(&post_row, was_reposted, repost_uri);
            });
            imp.repost_handler_id.replace(Some(id));
        }
    }

    /// Toggle the repost button visual state (optimistic update)
    pub fn toggle_repost_visual(&self) {
        let imp = self.imp();
        let was_reposted = *imp.is_reposted.borrow();

        if let Some(btn) = imp.repost_btn.borrow().as_ref() {
            if was_reposted {
                btn.remove_css_class("reposted");
                imp.is_reposted.replace(false);
                imp.viewer_repost_uri.replace(None);
                // Decrement count
                if let Some(label) = imp.repost_count_label.borrow().as_ref() {
                    if let Ok(count) = label.text().parse::<i32>() {
                        label.set_text(&Self::format_count(Some((count - 1).max(0) as u32)));
                    }
                }
                // Update menu item label
                if let Some(label) = imp.repost_item_label.borrow().as_ref() {
                    label.set_text("Repost");
                }
            } else {
                btn.add_css_class("reposted");
                imp.is_reposted.replace(true);
                // Increment count
                if let Some(label) = imp.repost_count_label.borrow().as_ref() {
                    if let Ok(count) = label.text().parse::<i32>() {
                        label.set_text(&Self::format_count(Some((count + 1) as u32)));
                    }
                }
                // Update menu item label
                if let Some(label) = imp.repost_item_label.borrow().as_ref() {
                    label.set_text("Undo Repost");
                }
            }
        }
    }

    /// Connect callback for the "Quote" menu item
    pub fn connect_quote_clicked<F: Fn() + 'static>(&self, f: F) {
        let imp = self.imp();
        if let Some(id) = imp.quote_handler_id.take() {
            if let Some(btn) = imp.quote_item.borrow().as_ref() {
                btn.disconnect(id);
            }
        }
        if let Some(btn) = imp.quote_item.borrow().as_ref() {
            // Close popover when clicked
            let menu_btn = imp.repost_btn.borrow();
            let popover = menu_btn.as_ref().and_then(|m| m.popover());
            let id = btn.connect_clicked(move |_| {
                if let Some(p) = &popover {
                    p.popdown();
                }
                f();
            });
            imp.quote_handler_id.replace(Some(id));
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

    /// Set callback for when the post content area is clicked (to open thread)
    pub fn set_post_clicked_callback<F: Fn(Post) + 'static>(&self, f: F) {
        self.imp()
            .post_clicked_callback
            .replace(Some(Box::new(f)));
    }

    /// Set callback for when the avatar/profile is clicked (to open profile)
    pub fn set_profile_clicked_callback<F: Fn(Profile) + 'static>(&self, f: F) {
        self.imp()
            .profile_clicked_callback
            .replace(Some(Box::new(f)));
    }

    pub fn bind(&self, post: &Post) {
        let imp = self.imp();
        imp.post.replace(Some(post.clone()));

        // Show/hide repost attribution
        if let Some(repost_row) = imp.repost_row.borrow().as_ref() {
            if let Some(reason) = &post.repost_reason {
                let name = reason
                    .by
                    .display_name
                    .as_deref()
                    .unwrap_or(&reason.by.handle);
                // Find the label inside the repost row
                if let Some(label) = repost_row.last_child() {
                    if let Ok(label) = label.downcast::<gtk4::Label>() {
                        label.set_text(&format!("Reposted by {}", name));
                    }
                }
                repost_row.set_visible(true);
            } else {
                repost_row.set_visible(false);
            }
        }

        // Show/hide reply indicator
        if let Some(reply_indicator) = imp.reply_indicator.borrow().as_ref() {
            if let Some(context) = &post.reply_context {
                // Always show @handle for clarity
                reply_indicator.set_text(&format!("↩ @{}", context.parent_author.handle));
                reply_indicator.set_visible(true);
            } else {
                reply_indicator.set_visible(false);
            }
        }

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

        // Render embed content
        if let Some(container) = imp.embed_container.borrow().as_ref() {
            // Clear previous embeds
            while let Some(child) = container.first_child() {
                container.remove(&child);
            }

            if let Some(embed) = &post.embed {
                self.render_embed(container, embed);
                container.set_visible(true);
            } else {
                container.set_visible(false);
            }
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

        // Track like/repost state for toggle operations
        imp.is_liked.replace(post.viewer_like.is_some());
        imp.is_reposted.replace(post.viewer_repost.is_some());
        imp.viewer_like_uri.replace(post.viewer_like.clone());
        imp.viewer_repost_uri.replace(post.viewer_repost.clone());

        // Update like button state
        if let Some(btn) = imp.like_btn.borrow().as_ref() {
            if post.viewer_like.is_some() {
                btn.add_css_class("liked");
            } else {
                btn.remove_css_class("liked");
            }
        }

        // Update repost button state and menu label
        if let Some(btn) = imp.repost_btn.borrow().as_ref() {
            if post.viewer_repost.is_some() {
                btn.add_css_class("reposted");
            } else {
                btn.remove_css_class("reposted");
            }
        }
        // Update repost menu item label
        if let Some(label) = imp.repost_item_label.borrow().as_ref() {
            if post.viewer_repost.is_some() {
                label.set_text("Undo Repost");
            } else {
                label.set_text("Repost");
            }
        }
    }

    /// Check if the post is currently liked (tracks local state after user actions)
    pub fn is_liked(&self) -> bool {
        *self.imp().is_liked.borrow()
    }

    /// Check if the post is currently reposted (tracks local state after user actions)
    pub fn is_reposted(&self) -> bool {
        *self.imp().is_reposted.borrow()
    }

    /// Get the viewer's like URI (if liked)
    pub fn viewer_like_uri(&self) -> Option<String> {
        self.imp().viewer_like_uri.borrow().clone()
    }

    /// Get the viewer's repost URI (if reposted)
    pub fn viewer_repost_uri(&self) -> Option<String> {
        self.imp().viewer_repost_uri.borrow().clone()
    }

    /// Set the viewer's like URI (after a successful like operation)
    pub fn set_viewer_like_uri(&self, uri: Option<String>) {
        self.imp().viewer_like_uri.replace(uri);
    }

    /// Set the viewer's repost URI (after a successful repost operation)
    pub fn set_viewer_repost_uri(&self, uri: Option<String>) {
        self.imp().viewer_repost_uri.replace(uri);
    }

    /// Render embed content into the container
    fn render_embed(&self, container: &gtk4::Box, embed: &Embed) {
        match embed {
            Embed::Images(images) => {
                self.render_images(container, images);
            }
            Embed::External(ext) => {
                self.render_external_card(container, ext);
            }
            Embed::Video(video) => {
                self.render_video(container, video);
            }
            Embed::Quote(quote) => {
                self.render_quote(container, quote);
            }
            Embed::QuoteWithMedia { quote, media } => {
                // Render media first, then quote below
                self.render_embed(container, media);
                self.render_quote(container, quote);
            }
        }
    }

    /// Render images with smart layout based on count
    fn render_images(&self, container: &gtk4::Box, images: &[ImageEmbed]) {
        // Create a container for the image grid
        let grid_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        grid_container.add_css_class("image-grid");

        match images.len() {
            0 => return,
            1 => {
                // Single image - preserve aspect ratio with max height constraint
                let img = &images[0];

                // Use a frame with overflow hidden and max height to constrain tall images
                let frame = gtk4::Frame::new(None);
                frame.set_hexpand(true);
                frame.set_overflow(gtk4::Overflow::Hidden);
                frame.add_css_class("post-embed-image");
                // Max height of 400px for single images - prevents tall images from dominating
                frame.set_size_request(-1, -1);

                let picture = gtk4::Picture::new();
                picture.set_hexpand(true);
                picture.set_can_shrink(true);
                picture.set_keep_aspect_ratio(true);
                // Set a reasonable max height via widget height request
                picture.set_size_request(-1, 400);

                frame.set_child(Some(&picture));
                avatar_cache::load_image_into_picture(picture, img.thumb.clone());
                grid_container.append(&frame);
            }
            2 => {
                // Two images side by side, both square-ish
                let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                row.set_homogeneous(true);

                for img in images.iter() {
                    let cell = self.create_image_cell(&img.thumb, 200);
                    row.append(&cell);
                }
                grid_container.append(&row);
            }
            3 => {
                // One tall left, two stacked right
                let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                row.set_homogeneous(true);

                // Left image - tall (spans both rows visually)
                let left = self.create_image_cell(&images[0].thumb, 260);
                row.append(&left);

                // Right column with two stacked images
                let right_col = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
                let top_right = self.create_image_cell(&images[1].thumb, 128);
                let bot_right = self.create_image_cell(&images[2].thumb, 128);
                right_col.append(&top_right);
                right_col.append(&bot_right);
                row.append(&right_col);

                grid_container.append(&row);
            }
            _ => {
                // 4+ images: 2x2 grid
                let top_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                top_row.set_homogeneous(true);
                let bot_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                bot_row.set_homogeneous(true);

                for (i, img) in images.iter().take(4).enumerate() {
                    let cell = self.create_image_cell(&img.thumb, 150);
                    if i < 2 {
                        top_row.append(&cell);
                    } else {
                        bot_row.append(&cell);
                    }
                }

                grid_container.append(&top_row);
                grid_container.set_spacing(4);
                grid_container.append(&bot_row);
            }
        }

        container.append(&grid_container);
    }

    /// Create an image cell that fills a fixed height container
    fn create_image_cell(&self, url: &str, height: i32) -> gtk4::Frame {
        // Frame acts as the clipping container
        let frame = gtk4::Frame::new(None);
        frame.set_hexpand(true);
        frame.set_size_request(-1, height);
        frame.set_overflow(gtk4::Overflow::Hidden);
        frame.add_css_class("post-embed-image");

        // Create picture - stretch to fill (no aspect ratio preservation for "cover" effect)
        let picture = gtk4::Picture::new();
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_can_shrink(true);
        picture.set_keep_aspect_ratio(false); // Stretch to fill the cell

        frame.set_child(Some(&picture));
        avatar_cache::load_image_into_picture(picture, url.to_string());

        frame
    }

    /// Render an external link card
    fn render_external_card(&self, container: &gtk4::Box, ext: &crate::atproto::ExternalEmbed) {
        let card = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        card.add_css_class("external-card");

        // Thumbnail (if available)
        if let Some(thumb_url) = &ext.thumb {
            let thumb = gtk4::Picture::new();
            thumb.set_keep_aspect_ratio(true);
            thumb.set_can_shrink(true);
            thumb.set_size_request(100, 100);
            thumb.add_css_class("external-thumb");
            avatar_cache::load_image_into_picture(thumb.clone(), thumb_url.clone());
            card.append(&thumb);
        }

        // Text content
        let text_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        text_box.set_hexpand(true);
        text_box.set_valign(gtk4::Align::Center);

        let title = gtk4::Label::new(Some(&ext.title));
        title.set_halign(gtk4::Align::Start);
        title.add_css_class("external-title");
        title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        title.set_max_width_chars(50);
        text_box.append(&title);

        if !ext.description.is_empty() {
            let desc = gtk4::Label::new(Some(&ext.description));
            desc.set_halign(gtk4::Align::Start);
            desc.add_css_class("dim-label");
            desc.add_css_class("caption");
            desc.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            desc.set_lines(2);
            desc.set_max_width_chars(60);
            text_box.append(&desc);
        }

        // Extract domain from URI
        if let Ok(url) = url::Url::parse(&ext.uri) {
            if let Some(domain) = url.host_str() {
                let domain_label = gtk4::Label::new(Some(domain));
                domain_label.set_halign(gtk4::Align::Start);
                domain_label.add_css_class("dim-label");
                domain_label.add_css_class("caption");
                text_box.append(&domain_label);
            }
        }

        card.append(&text_box);
        container.append(&card);
    }

    /// Render a video embed
    fn render_video(&self, container: &gtk4::Box, video: &crate::atproto::VideoEmbed) {
        let overlay = gtk4::Overlay::new();
        overlay.add_css_class("video-embed");

        // Thumbnail
        let thumb = gtk4::Picture::new();
        thumb.set_keep_aspect_ratio(true);
        thumb.set_can_shrink(true);
        thumb.set_size_request(400, 225);
        thumb.add_css_class("post-embed-image");

        if let Some(thumb_url) = &video.thumbnail {
            avatar_cache::load_image_into_picture(thumb.clone(), thumb_url.clone());
        }
        overlay.set_child(Some(&thumb));

        // Play button overlay
        let play_btn = gtk4::Button::from_icon_name("media-playback-start-symbolic");
        play_btn.add_css_class("circular");
        play_btn.add_css_class("osd");
        play_btn.set_halign(gtk4::Align::Center);
        play_btn.set_valign(gtk4::Align::Center);

        // Open video in browser when clicked
        let playlist_url = video.playlist.clone();
        play_btn.connect_clicked(move |_| {
            let _ = open::that(&playlist_url);
        });

        overlay.add_overlay(&play_btn);

        container.append(&overlay);
    }

    /// Render a quote post card
    fn render_quote(&self, container: &gtk4::Box, quote: &crate::atproto::QuoteEmbed) {
        let card = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        card.add_css_class("quote-card");

        // Author row
        let author_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);

        let avatar = adw::Avatar::new(24, None, true);
        let author_name = quote
            .author
            .display_name
            .as_deref()
            .unwrap_or(&quote.author.handle);
        avatar.set_text(Some(author_name));
        if let Some(avatar_url) = &quote.author.avatar {
            avatar_cache::load_avatar(avatar.clone(), avatar_url.clone());
        }
        author_row.append(&avatar);

        let name_label = gtk4::Label::new(Some(author_name));
        name_label.add_css_class("heading");
        name_label.add_css_class("caption");
        author_row.append(&name_label);

        let handle_label = gtk4::Label::new(Some(&format!("@{}", quote.author.handle)));
        handle_label.add_css_class("dim-label");
        handle_label.add_css_class("caption");
        author_row.append(&handle_label);

        let time_label = gtk4::Label::new(Some(&Self::format_timestamp(&quote.indexed_at)));
        time_label.add_css_class("dim-label");
        time_label.add_css_class("caption");
        time_label.set_hexpand(true);
        time_label.set_halign(gtk4::Align::End);
        author_row.append(&time_label);

        card.append(&author_row);

        // Quote text
        if !quote.text.is_empty() {
            let text_label = gtk4::Label::new(Some(&quote.text));
            text_label.set_halign(gtk4::Align::Start);
            text_label.set_wrap(true);
            text_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            text_label.set_xalign(0.0);
            text_label.add_css_class("caption");
            card.append(&text_label);
        }

        // Nested embed (if any)
        if let Some(nested_embed) = &quote.embed {
            let nested_container = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            self.render_embed(&nested_container, nested_embed);
            card.append(&nested_container);
        }

        container.append(&card);
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
