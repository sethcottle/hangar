// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]
#![allow(clippy::collapsible_if)]

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
        // Rich embeds and context
        pub repost_row: RefCell<Option<gtk4::Box>>,
        pub repost_avatar: RefCell<Option<adw::Avatar>>,
        pub repost_label: RefCell<Option<gtk4::Label>>,
        pub repost_attribution_btn: RefCell<Option<gtk4::Button>>,
        pub reply_indicator: RefCell<Option<gtk4::Button>>,
        pub reply_handle_label: RefCell<Option<gtk4::Label>>,
        pub reply_indicator_box: RefCell<Option<gtk4::Box>>,
        pub embed_container: RefCell<Option<gtk4::Box>>,
        pub verified_badge: RefCell<Option<gtk4::Image>>,
        pub post_menu_btn: RefCell<Option<gtk4::MenuButton>>,
        pub view_post_item: RefCell<Option<gtk4::Button>>,
        pub copy_link_item: RefCell<Option<gtk4::Button>>,
        pub open_link_item: RefCell<Option<gtk4::Button>>,
        // Track current like/repost state (may differ from original post after user actions)
        pub is_liked: RefCell<bool>,
        pub is_reposted: RefCell<bool>,
        pub viewer_like_uri: RefCell<Option<String>>,
        pub viewer_repost_uri: RefCell<Option<String>>,
        // Store indexed_at for timestamp refresh
        pub indexed_at: RefCell<String>,
        // Navigation callbacks
        pub post_clicked_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
        pub profile_clicked_callback: RefCell<Option<Box<dyn Fn(Profile) + 'static>>>,
        pub mention_clicked_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
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
        self.set_margin_start(12);
        self.set_margin_end(16);
        self.set_margin_top(8);
        self.set_margin_bottom(8);
        self.add_css_class("post-row");

        // Main horizontal layout: avatar on left, content on right
        let main_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        main_box.set_hexpand(true);

        // Left column: Avatar (fixed width, aligned to top)
        let avatar_column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        avatar_column.set_valign(gtk4::Align::Start);

        // Wrap avatar in a button-like container for clickability
        let avatar_btn = gtk4::Button::new();
        avatar_btn.add_css_class("flat");
        avatar_btn.add_css_class("circular");
        avatar_btn.add_css_class("avatar-button");
        avatar_btn.set_cursor_from_name(Some("pointer"));
        let avatar = adw::Avatar::new(42, None, true);
        avatar_btn.set_child(Some(&avatar));
        avatar_column.append(&avatar_btn);

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

        main_box.append(&avatar_column);

        // Right column: all content (repost attribution, header, text, embeds, actions)
        let content_column = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        content_column.set_hexpand(true);

        // Repost attribution row (above header, shows "Reposted by X") - clickable to go to reposter's profile
        let repost_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        repost_row.set_margin_bottom(2);
        repost_row.add_css_class("repost-attribution");
        let repost_icon = gtk4::Image::from_icon_name("media-playlist-repeat-symbolic");
        repost_icon.add_css_class("dim-label");
        repost_icon.set_pixel_size(12);
        repost_row.append(&repost_icon);

        // Clickable button containing avatar + name
        let repost_btn = gtk4::Button::new();
        repost_btn.add_css_class("flat");
        repost_btn.add_css_class("repost-btn");
        repost_btn.set_cursor_from_name(Some("pointer"));

        let repost_btn_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let repost_avatar = adw::Avatar::new(16, None, true);
        repost_avatar.add_css_class("repost-avatar");
        repost_btn_content.append(&repost_avatar);
        let repost_label = gtk4::Label::new(None);
        repost_label.add_css_class("dim-label");
        repost_label.add_css_class("caption");
        repost_btn_content.append(&repost_label);
        repost_btn.set_child(Some(&repost_btn_content));
        repost_row.append(&repost_btn);

        repost_row.set_visible(false);
        content_column.append(&repost_row);

        // Header row: display name + verified badge + handle + · + timestamp (all on one line)
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        header.set_hexpand(true);

        let display_name = gtk4::Label::new(None);
        display_name.set_halign(gtk4::Align::Start);
        display_name.add_css_class("heading");
        display_name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        header.append(&display_name);

        // Verified badge (checkmark)
        let verified_badge = gtk4::Image::from_icon_name("emblem-ok-symbolic");
        verified_badge.add_css_class("verified-badge");
        verified_badge.set_pixel_size(14);
        verified_badge.set_margin_start(4);
        verified_badge.set_visible(false);
        header.append(&verified_badge);

        // Handle (with spacing)
        let handle = gtk4::Label::new(None);
        handle.set_halign(gtk4::Align::Start);
        handle.add_css_class("dim-label");
        handle.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        handle.set_margin_start(6);
        header.append(&handle);

        // Separator dot
        let dot = gtk4::Label::new(Some("·"));
        dot.add_css_class("dim-label");
        dot.set_margin_start(6);
        dot.set_margin_end(6);
        header.append(&dot);

        // Timestamp
        let timestamp = gtk4::Label::new(None);
        timestamp.add_css_class("dim-label");
        header.append(&timestamp);

        // Spacer to push timestamp/menu to the right if needed
        let header_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        header_spacer.set_hexpand(true);
        header.append(&header_spacer);

        content_column.append(&header);

        // Reply indicator (shows "Replying to @handle") - below header, above content - clickable
        let reply_indicator_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        reply_indicator_box.set_margin_top(2);
        reply_indicator_box.set_margin_bottom(4);
        reply_indicator_box.set_valign(gtk4::Align::Center);

        let reply_icon = gtk4::Image::from_icon_name("mail-reply-sender-symbolic");
        reply_icon.add_css_class("dim-label");
        reply_icon.set_pixel_size(12);
        reply_icon.set_margin_end(6);
        reply_indicator_box.append(&reply_icon);

        let reply_text = gtk4::Label::new(Some("Replying to"));
        reply_text.add_css_class("dim-label");
        reply_text.add_css_class("caption");
        reply_text.set_valign(gtk4::Align::Center);
        reply_indicator_box.append(&reply_text);

        // Clickable handle button (minimal padding, tight to text)
        let reply_indicator = gtk4::Button::new();
        reply_indicator.add_css_class("flat");
        reply_indicator.add_css_class("reply-handle-btn");
        reply_indicator.set_cursor_from_name(Some("pointer"));
        reply_indicator.set_valign(gtk4::Align::Center);
        let reply_handle_label = gtk4::Label::new(None);
        reply_handle_label.add_css_class("caption");
        reply_handle_label.add_css_class("link-label");
        reply_indicator.set_child(Some(&reply_handle_label));
        reply_indicator_box.append(&reply_indicator);

        reply_indicator_box.set_visible(false);
        content_column.append(&reply_indicator_box);

        // Content area (clickable to open thread)
        let content_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_area.set_cursor_from_name(Some("pointer"));

        // Post content
        let content = gtk4::Label::new(None);
        content.set_halign(gtk4::Align::Start);
        content.set_hexpand(true);
        content.set_wrap(true);
        content.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        content.set_selectable(false); // Disable selection to allow click-through
        content.set_xalign(0.0);
        content.set_use_markup(true); // Enable markup for clickable links

        // Handle link clicks (URLs, @mentions, #hashtags)
        let post_row_for_links = self.clone();
        content.connect_activate_link(move |_, uri| {
            if uri.starts_with("bsky-mention://") {
                // Handle @mention click - navigate to profile
                let handle = uri.strip_prefix("bsky-mention://").unwrap_or("");
                let imp = post_row_for_links.imp();
                if let Some(cb) = imp.mention_clicked_callback.borrow().as_ref() {
                    cb(handle.to_string());
                }
                glib::Propagation::Stop // We handled it
            } else if uri.starts_with("bsky-tag://") {
                // Handle hashtag click - could open search in future
                let _tag = uri.strip_prefix("bsky-tag://").unwrap_or("");
                // TODO: Implement hashtag search navigation
                glib::Propagation::Stop
            } else {
                // Regular URL - open in browser
                let _ = open::that(uri);
                glib::Propagation::Stop
            }
        });
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

        content_column.append(&content_area);

        // Action bar
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        actions.set_margin_top(8);

        let (reply_btn, reply_count, reply_btn_ref) =
            Self::create_action_button("mail-reply-sender-symbolic");
        actions.append(&reply_btn);

        // Repost menu button with popover
        let (
            repost_btn_box,
            repost_count,
            repost_menu_btn,
            repost_item,
            repost_item_label,
            quote_item,
        ) = Self::create_repost_menu_button();
        actions.append(&repost_btn_box);

        let (like_btn, like_count, like_btn_ref) =
            Self::create_action_button("emote-love-symbolic");
        actions.append(&like_btn);

        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        actions.append(&spacer);

        // Post overflow menu button
        let (menu_btn, view_post_item, copy_link_item, open_link_item) =
            Self::create_post_menu_button();
        actions.append(&menu_btn);

        content_column.append(&actions);

        main_box.append(&content_column);
        self.append(&main_box);

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
        imp.repost_avatar.replace(Some(repost_avatar));
        imp.repost_label.replace(Some(repost_label));
        imp.repost_attribution_btn.replace(Some(repost_btn));
        imp.reply_indicator.replace(Some(reply_indicator));
        imp.reply_handle_label.replace(Some(reply_handle_label));
        imp.reply_indicator_box.replace(Some(reply_indicator_box));
        imp.embed_container.replace(Some(embed_container));
        imp.verified_badge.replace(Some(verified_badge));
        imp.post_menu_btn.replace(Some(menu_btn));
        imp.view_post_item.replace(Some(view_post_item));
        imp.copy_link_item.replace(Some(copy_link_item));
        imp.open_link_item.replace(Some(open_link_item));
    }

    fn create_action_button(icon_name: &str) -> (gtk4::Box, gtk4::Label, gtk4::Button) {
        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        btn_box.set_valign(gtk4::Align::Center);

        let btn = gtk4::Button::from_icon_name(icon_name);
        btn.add_css_class("flat");
        btn.add_css_class("circular");
        btn_box.append(&btn);

        let count_label = gtk4::Label::new(Some(""));
        count_label.add_css_class("dim-label");
        count_label.add_css_class("caption");
        btn_box.append(&count_label);

        (btn_box, count_label, btn)
    }

    /// Create a repost menu button with "Repost"/"Undo Repost" and "Quote" options
    fn create_repost_menu_button() -> (
        gtk4::Box,
        gtk4::Label,
        gtk4::MenuButton,
        gtk4::Button,
        gtk4::Label,
        gtk4::Button,
    ) {
        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        btn_box.set_valign(gtk4::Align::Center);

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

        let count_label = gtk4::Label::new(Some(""));
        count_label.add_css_class("dim-label");
        count_label.add_css_class("caption");
        btn_box.append(&count_label);

        (
            btn_box,
            count_label,
            menu_btn,
            repost_item,
            repost_item_label,
            quote_item,
        )
    }

    /// Create a post overflow menu button with View Post, Bookmark, Report, etc.
    /// Returns: (menu_btn, view_item, copy_link_item, open_link_item)
    fn create_post_menu_button() -> (gtk4::MenuButton, gtk4::Button, gtk4::Button, gtk4::Button) {
        let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        popover_box.set_margin_top(6);
        popover_box.set_margin_bottom(6);
        popover_box.set_margin_start(6);
        popover_box.set_margin_end(6);

        // View Post and Replies
        let view_item = gtk4::Button::new();
        let view_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        view_content.append(&gtk4::Image::from_icon_name("view-reveal-symbolic"));
        view_content.append(&gtk4::Label::new(Some("View Post and Replies")));
        view_item.set_child(Some(&view_content));
        view_item.add_css_class("flat");
        popover_box.append(&view_item);

        // Separator
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        sep.set_margin_top(4);
        sep.set_margin_bottom(4);
        popover_box.append(&sep);

        // Open Link to Post
        let open_link_item = gtk4::Button::new();
        let open_link_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        open_link_content.append(&gtk4::Image::from_icon_name("web-browser-symbolic"));
        open_link_content.append(&gtk4::Label::new(Some("Open Link to Post")));
        open_link_item.set_child(Some(&open_link_content));
        open_link_item.add_css_class("flat");
        popover_box.append(&open_link_item);

        // Copy Link to Post
        let copy_link_item = gtk4::Button::new();
        let copy_link_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        copy_link_content.append(&gtk4::Image::from_icon_name("edit-copy-symbolic"));
        copy_link_content.append(&gtk4::Label::new(Some("Copy Link to Post")));
        copy_link_item.set_child(Some(&copy_link_content));
        copy_link_item.add_css_class("flat");
        popover_box.append(&copy_link_item);

        // Separator
        let sep2 = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        sep2.set_margin_top(4);
        sep2.set_margin_bottom(4);
        popover_box.append(&sep2);

        // Report Post
        let report_item = gtk4::Button::new();
        let report_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        report_content.append(&gtk4::Image::from_icon_name("dialog-warning-symbolic"));
        report_content.append(&gtk4::Label::new(Some("Report Post...")));
        report_item.set_child(Some(&report_content));
        report_item.add_css_class("flat");
        popover_box.append(&report_item);

        let popover = gtk4::Popover::new();
        popover.set_child(Some(&popover_box));
        popover.add_css_class("menu");
        popover.set_has_arrow(false);

        let menu_btn = gtk4::MenuButton::new();
        menu_btn.set_icon_name("view-more-symbolic");
        menu_btn.add_css_class("flat");
        menu_btn.add_css_class("circular");
        menu_btn.set_popover(Some(&popover));

        (menu_btn, view_item, copy_link_item, open_link_item)
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
        self.imp().post_clicked_callback.replace(Some(Box::new(f)));
    }

    /// Set callback for when the avatar/profile is clicked (to open profile)
    pub fn set_profile_clicked_callback<F: Fn(Profile) + 'static>(&self, f: F) {
        self.imp()
            .profile_clicked_callback
            .replace(Some(Box::new(f)));
    }

    /// Set callback for when an @mention in post text is clicked (handle without @)
    pub fn set_mention_clicked_callback<F: Fn(String) + 'static>(&self, f: F) {
        self.imp()
            .mention_clicked_callback
            .replace(Some(Box::new(f)));
    }

    pub fn bind(&self, post: &Post) {
        let imp = self.imp();
        imp.post.replace(Some(post.clone()));

        // Show/hide repost attribution with avatar - clickable to go to reposter's profile
        if let Some(repost_row) = imp.repost_row.borrow().as_ref() {
            if let Some(reason) = &post.repost_reason {
                let name = reason
                    .by
                    .display_name
                    .as_deref()
                    .unwrap_or(&reason.by.handle);
                // Update the repost label
                if let Some(label) = imp.repost_label.borrow().as_ref() {
                    label.set_text(name);
                }
                // Load the reposter's avatar
                if let Some(repost_avatar) = imp.repost_avatar.borrow().as_ref() {
                    repost_avatar.set_text(Some(name));
                    if let Some(avatar_url) = &reason.by.avatar {
                        avatar_cache::load_avatar(repost_avatar.clone(), avatar_url.clone());
                    }
                }
                // Wire up click to navigate to reposter's profile
                if let Some(btn) = imp.repost_attribution_btn.borrow().as_ref() {
                    let post_row = self.clone();
                    let reposter_profile = reason.by.clone();
                    btn.connect_clicked(move |_| {
                        let inner_imp = post_row.imp();
                        if let Some(cb) = inner_imp.profile_clicked_callback.borrow().as_ref() {
                            cb(reposter_profile.clone());
                        }
                    });
                }
                repost_row.set_visible(true);
            } else {
                repost_row.set_visible(false);
            }
        }

        // Show/hide reply indicator - clickable to go to parent author's profile
        if let Some(reply_indicator_box) = imp.reply_indicator_box.borrow().as_ref() {
            if let Some(context) = &post.reply_context {
                // Update the handle label
                if let Some(label) = imp.reply_handle_label.borrow().as_ref() {
                    label.set_text(&format!("@{}", context.parent_author.handle));
                }
                // Wire up click to navigate to parent author's profile
                if let Some(btn) = imp.reply_indicator.borrow().as_ref() {
                    let post_row = self.clone();
                    let parent_profile = context.parent_author.clone();
                    btn.connect_clicked(move |_| {
                        let inner_imp = post_row.imp();
                        if let Some(cb) = inner_imp.profile_clicked_callback.borrow().as_ref() {
                            cb(parent_profile.clone());
                        }
                    });
                }
                reply_indicator_box.set_visible(true);
            } else {
                reply_indicator_box.set_visible(false);
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
        // Store indexed_at for timestamp refresh
        imp.indexed_at.replace(post.indexed_at.clone());

        // Set content with rich text formatting (links, mentions, hashtags)
        if let Some(label) = imp.content_label.borrow().as_ref() {
            let markup = Self::format_post_text(&post.text);
            label.set_markup(&markup);
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

        // Wire up post menu actions
        let post_url = Self::get_post_url(&post.author.handle, &post.uri);
        let popover = imp
            .post_menu_btn
            .borrow()
            .as_ref()
            .and_then(|m| m.popover());

        // View Post action - uses existing post_clicked_callback
        if let Some(view_item) = imp.view_post_item.borrow().as_ref() {
            let post_row = self.clone();
            let popover_clone = popover.clone();
            view_item.connect_clicked(move |_| {
                if let Some(p) = &popover_clone {
                    p.popdown();
                }
                let inner_imp = post_row.imp();
                if let Some(post) = inner_imp.post.borrow().as_ref() {
                    if let Some(cb) = inner_imp.post_clicked_callback.borrow().as_ref() {
                        cb(post.clone());
                    }
                }
            });
        }

        // Copy Link action
        if let Some(copy_item) = imp.copy_link_item.borrow().as_ref() {
            let url = post_url.clone();
            let popover_clone = popover.clone();
            copy_item.connect_clicked(move |btn| {
                if let Some(p) = &popover_clone {
                    p.popdown();
                }
                // Copy to clipboard
                let display = btn.display();
                display.clipboard().set_text(&url);
            });
        }

        // Open Link action
        if let Some(open_item) = imp.open_link_item.borrow().as_ref() {
            let url = post_url;
            let popover_clone = popover;
            open_item.connect_clicked(move |_| {
                if let Some(p) = &popover_clone {
                    p.popdown();
                }
                let _ = open::that(&url);
            });
        }
    }

    /// Generate a Bluesky web URL for a post
    fn get_post_url(handle: &str, uri: &str) -> String {
        // Extract the rkey from the AT URI (e.g., at://did:plc:xxx/app.bsky.feed.post/rkey)
        let rkey = uri.rsplit('/').next().unwrap_or("");
        format!("https://bsky.app/profile/{}/post/{}", handle, rkey)
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

    /// Render an external link card (clickable to open URL)
    fn render_external_card(&self, container: &gtk4::Box, ext: &crate::atproto::ExternalEmbed) {
        // Wrap card in a button for clickability
        let card_btn = gtk4::Button::new();
        card_btn.add_css_class("flat");
        card_btn.add_css_class("external-card-button");
        card_btn.set_cursor_from_name(Some("pointer"));

        let card = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        card.add_css_class("external-card");

        // Thumbnail (if available) - smaller for compact layout
        if let Some(thumb_url) = &ext.thumb {
            let thumb_frame = gtk4::Frame::new(None);
            thumb_frame.set_overflow(gtk4::Overflow::Hidden);
            thumb_frame.add_css_class("external-thumb");

            let thumb = gtk4::Picture::new();
            thumb.set_keep_aspect_ratio(true);
            thumb.set_can_shrink(true);
            thumb.set_size_request(72, 72); // Smaller thumbnail for compact window
            avatar_cache::load_image_into_picture(thumb.clone(), thumb_url.clone());
            thumb_frame.set_child(Some(&thumb));
            card.append(&thumb_frame);
        }

        // Text content
        let text_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        text_box.set_hexpand(true);
        text_box.set_valign(gtk4::Align::Center);

        let title = gtk4::Label::new(Some(&ext.title));
        title.set_halign(gtk4::Align::Start);
        title.add_css_class("external-title");
        title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        title.set_wrap(true);
        title.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        title.set_lines(2);
        text_box.append(&title);

        if !ext.description.is_empty() {
            let desc = gtk4::Label::new(Some(&ext.description));
            desc.set_halign(gtk4::Align::Start);
            desc.add_css_class("dim-label");
            desc.add_css_class("caption");
            desc.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            desc.set_wrap(true);
            desc.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            desc.set_lines(2);
            text_box.append(&desc);
        }

        // Extract domain from URI and show with web browser icon
        if let Ok(url) = url::Url::parse(&ext.uri) {
            if let Some(domain) = url.host_str() {
                let domain_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                let link_icon = gtk4::Image::from_icon_name("web-browser-symbolic");
                link_icon.add_css_class("dim-label");
                link_icon.set_pixel_size(12);
                domain_row.append(&link_icon);
                let domain_label = gtk4::Label::new(Some(domain));
                domain_label.set_halign(gtk4::Align::Start);
                domain_label.add_css_class("dim-label");
                domain_label.add_css_class("caption");
                domain_row.append(&domain_label);
                text_box.append(&domain_row);
            }
        }

        card.append(&text_box);
        card_btn.set_child(Some(&card));

        // Open link in browser when clicked
        let url = ext.uri.clone();
        card_btn.connect_clicked(move |_| {
            let _ = open::that(&url);
        });

        container.append(&card_btn);
    }

    /// Render a video embed
    fn render_video(&self, container: &gtk4::Box, video: &crate::atproto::VideoEmbed) {
        let overlay = gtk4::Overlay::new();
        overlay.add_css_class("video-embed");
        overlay.set_hexpand(true);

        // Thumbnail - responsive width, constrained height
        let thumb = gtk4::Picture::new();
        thumb.set_hexpand(true);
        thumb.set_keep_aspect_ratio(true);
        thumb.set_can_shrink(true);
        thumb.set_size_request(-1, 200); // Only constrain height, let width be flexible
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

    /// Render a quote post card (clickable to open quoted post)
    fn render_quote(&self, container: &gtk4::Box, quote: &crate::atproto::QuoteEmbed) {
        // Wrap card in a button for clickability
        let card_btn = gtk4::Button::new();
        card_btn.add_css_class("flat");
        card_btn.add_css_class("quote-card-button");
        card_btn.set_cursor_from_name(Some("pointer"));

        let card = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        card.add_css_class("quote-card");

        // Author row with display name + handle + timestamp on same line
        let author_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

        let avatar = adw::Avatar::new(20, None, true);
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

        let dot = gtk4::Label::new(Some("·"));
        dot.add_css_class("dim-label");
        dot.add_css_class("caption");
        author_row.append(&dot);

        let time_label = gtk4::Label::new(Some(&Self::format_timestamp(&quote.indexed_at)));
        time_label.add_css_class("dim-label");
        time_label.add_css_class("caption");
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

        card_btn.set_child(Some(&card));

        // Open quoted post in browser when clicked
        let url = Self::get_post_url(&quote.author.handle, &quote.uri);
        card_btn.connect_clicked(move |_| {
            let _ = open::that(&url);
        });

        container.append(&card_btn);
    }

    fn format_count(count: Option<u32>) -> String {
        match count {
            Some(c) if c >= 1_000_000 => format!("{:.1}M", c as f64 / 1_000_000.0),
            Some(c) if c >= 1_000 => format!("{:.1}K", c as f64 / 1_000.0),
            Some(c) if c > 0 => c.to_string(),
            _ => String::new(), // Don't show "0", just leave empty
        }
    }

    /// Format post text with clickable links, mentions, and hashtags using Pango markup
    /// URLs become actual <a> tags, mentions use bsky-mention:// scheme, hashtags use bsky-tag:// scheme
    fn format_post_text(text: &str) -> String {
        // First escape the entire text for Pango markup
        let escaped = glib::markup_escape_text(text);
        let mut result = escaped.to_string();

        // Pattern for URLs with protocol (http/https)
        let url_with_protocol =
            regex::Regex::new(r"https?://[^\s<>\[\]{}|\\^`\x00-\x1f\x7f]+").unwrap();

        // Pattern for bare domain URLs (domain.tld/path) - starts with word boundary
        // Common TLDs that are likely to be URLs
        let bare_url_pattern = regex::Regex::new(
            r"\b([a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+(?:com|org|net|io|co|app|dev|edu|gov|me|info|biz|social)[/a-zA-Z0-9._~:/?#@!$&'()*+,;=-]*",
        )
        .unwrap();

        // Pattern for @mentions (e.g., @user.bsky.social)
        let mention_pattern = regex::Regex::new(
            r"@([a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z]([a-zA-Z0-9-]*[a-zA-Z0-9])?",
        )
        .unwrap();

        // Pattern for hashtags
        let hashtag_pattern = regex::Regex::new(r"#[a-zA-Z][a-zA-Z0-9_]*").unwrap();

        // Replace URLs with protocol first
        result = url_with_protocol
            .replace_all(&result, |caps: &regex::Captures| {
                let url = &caps[0];
                format!("<a href=\"{}\">{}</a>", url, url)
            })
            .to_string();

        // Replace bare domain URLs (only if not already inside an <a> tag)
        // We check by looking for urls not preceded by href="
        result = bare_url_pattern
            .replace_all(&result, |caps: &regex::Captures| {
                let url = &caps[0];
                // Skip if this looks like it's already been linkified (contains href=)
                if url.contains("href=") {
                    url.to_string()
                } else {
                    format!("<a href=\"https://{}\">{}</a>", url, url)
                }
            })
            .to_string();

        // Replace @mentions with clickable links using custom scheme
        result = mention_pattern
            .replace_all(&result, |caps: &regex::Captures| {
                let mention = &caps[0];
                let handle = &mention[1..]; // Strip the @ prefix for the URI
                format!("<a href=\"bsky-mention://{}\">{}</a>", handle, mention)
            })
            .to_string();

        // Replace hashtags with clickable links using custom scheme
        result = hashtag_pattern
            .replace_all(&result, |caps: &regex::Captures| {
                let hashtag = &caps[0];
                let tag = &hashtag[1..]; // Strip the # prefix for the URI
                format!("<a href=\"bsky-tag://{}\">{}</a>", tag, hashtag)
            })
            .to_string();

        result
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

    /// Refresh the timestamp display (for periodic updates)
    pub fn refresh_timestamp(&self) {
        let imp = self.imp();
        let indexed_at = imp.indexed_at.borrow();
        if !indexed_at.is_empty() {
            if let Some(label) = imp.timestamp_label.borrow().as_ref() {
                label.set_text(&Self::format_timestamp(&indexed_at));
            }
        }
    }
}

impl Default for PostRow {
    fn default() -> Self {
        Self::new()
    }
}
