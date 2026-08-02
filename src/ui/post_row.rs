// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]
#![allow(clippy::collapsible_if)]

use crate::atproto::{Embed, ImageEmbed, Post};
use crate::ui::avatar_cache;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;

use crate::atproto::Profile;

/// Width of the post content column at the timeline's 800px `AdwClamp`.
///
/// Media embeds derive their height from this and their own aspect ratio. The
/// timeline's rows are allocated their *minimum* height, so a height request on
/// a media widget is not a floor, it is the height - which means an embed sized
/// independently of its aspect ratio ends up with slack that GtkPicture always
/// centres, and centred media next to left-aligned text reads as misaligned.
///
/// An estimate, unavoidably: the real column measures 495px in a 700px window
/// with the sidebar, 633 at 820, and settles near 684 once the clamp binds, and
/// the height has to be requested before any of that is known. Tuned toward the
/// narrow end, because guessing high costs visible dead space above and below
/// an embed while guessing low only means the image stops short of the column
/// edge. The clamp in [`PostRow::embed_height`] bounds the error either way.
const CONTENT_WIDTH: f64 = 540.0;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct PostRow {
        pub post: RefCell<Option<Post>>,
        /// Set on the post a thread view is already focused on. Suppresses the
        /// row-body click without touching post_clicked_callback, which
        /// embedded quote cards still need in order to navigate.
        pub is_focused_post: Cell<bool>,
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
        // Main box for cursor control
        pub main_box: RefCell<Option<gtk4::Box>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PostRow {
        const NAME: &'static str = "HangarPostRow";
        type Type = super::PostRow;
        type ParentType = gtk4::Box;

        fn class_init(klass: &mut Self::Class) {
            // Use Group role (available since GTK 4.0) with RoleDescription "post"
            // to provide screen readers with meaningful context.
            // Article role requires v4_14 feature flag.
            klass.set_accessible_role(gtk4::AccessibleRole::Group);
        }
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
        // This entire area is clickable to open the thread
        let main_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        main_box.set_hexpand(true);
        main_box.set_cursor_from_name(Some("pointer"));

        // Left column: Avatar (fixed width, aligned to top)
        let avatar_column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        avatar_column.set_valign(gtk4::Align::Start);

        // Wrap avatar in a button-like container for clickability
        let avatar_btn = gtk4::Button::new();
        avatar_btn.add_css_class("flat");
        avatar_btn.add_css_class("circular");
        avatar_btn.add_css_class("avatar-button");
        avatar_btn.set_cursor_from_name(Some("pointer"));
        avatar_btn.set_tooltip_text(Some("View profile"));
        avatar_btn.update_property(&[gtk4::accessible::Property::Label("View profile")]);
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
        // A label with neither wrap nor ellipsize reports the full pixel width
        // of its text as its *minimum*, and a minimum travels out through
        // ListView, AdwClamp and Viewport untouched -- the clamp caps natural
        // width only. So one long display name, in any row the ListView has
        // bound rather than any row on screen, widens the entire feed past the
        // window. Every label in this file carrying text off the wire is
        // ellipsized for that reason.
        repost_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
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
        let verified_badge = gtk4::Image::from_icon_name("object-select-symbolic");
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
        reply_handle_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        reply_indicator.set_child(Some(&reply_handle_label));
        reply_indicator_box.append(&reply_indicator);

        reply_indicator_box.set_visible(false);
        content_column.append(&reply_indicator_box);

        // Content area
        let content_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Post content
        let content = gtk4::Label::new(None);
        content.set_halign(gtk4::Align::Fill);
        content.set_hexpand(true);
        content.set_wrap(true);
        content.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        content.set_natural_wrap_mode(gtk4::NaturalWrapMode::Word);
        content.set_selectable(false); // Disable selection to allow click-through
        content.set_xalign(0.0);
        content.set_use_markup(true); // Enable markup for clickable links

        // Handle link clicks (URLs, @mentions, #hashtags)
        let post_row_for_links = self.clone();
        content.connect_activate_link(move |label, uri| {
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
                crate::ui::external::open_url(label, uri, "link");
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

        content_column.append(&content_area);

        // Action bar
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        actions.set_margin_top(8);

        let (reply_btn, reply_count, reply_btn_ref) =
            Self::create_action_button("mail-reply-sender-symbolic");
        reply_btn_ref.set_tooltip_text(Some("Reply"));
        reply_btn_ref.update_property(&[gtk4::accessible::Property::Label("Reply")]);
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
        repost_menu_btn.set_tooltip_text(Some("Repost"));
        repost_menu_btn.update_property(&[gtk4::accessible::Property::Label("Repost options")]);
        actions.append(&repost_btn_box);

        let (like_btn, like_count, like_btn_ref) =
            Self::create_action_button("emote-love-symbolic");
        like_btn_ref.set_tooltip_text(Some("Like"));
        like_btn_ref.update_property(&[gtk4::accessible::Property::Label("Like")]);
        actions.append(&like_btn);

        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        actions.append(&spacer);

        // Post overflow menu button
        let (menu_btn, view_post_item, copy_link_item, open_link_item) =
            Self::create_post_menu_button();
        menu_btn.set_tooltip_text(Some("More options"));
        menu_btn.update_property(&[gtk4::accessible::Property::Label("More options")]);
        actions.append(&menu_btn);

        content_column.append(&actions);

        main_box.append(&content_column);

        // Add click gesture to main_box for opening thread
        let gesture = gtk4::GestureClick::new();
        let post_row = self.clone();
        gesture.connect_released(move |_, _, _, _| {
            let imp = post_row.imp();
            // The focused post of a thread is the subject of the page already;
            // letting the body navigate would push an identical thread page,
            // repeatable without limit.
            if imp.is_focused_post.get() {
                return;
            }
            if let Some(post) = imp.post.borrow().as_ref() {
                if let Some(cb) = imp.post_clicked_callback.borrow().as_ref() {
                    cb(post.clone());
                }
            }
        });
        main_box.add_controller(gesture);

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
        imp.main_box.replace(Some(main_box));
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
                // Decrement count and update accessible label
                if let Some(label) = imp.like_count_label.borrow().as_ref() {
                    if let Ok(count) = label.text().parse::<i32>() {
                        let new_count = (count - 1).max(0) as u32;
                        label.set_text(&Self::format_count(Some(new_count)));
                        btn.update_property(&[gtk4::accessible::Property::Label(&format!(
                            "Like. {} likes",
                            new_count
                        ))]);
                    }
                }
            } else {
                btn.add_css_class("liked");
                imp.is_liked.replace(true);
                // Note: we don't have the new like URI yet, but that's OK for visual state
                // Increment count and update accessible label
                if let Some(label) = imp.like_count_label.borrow().as_ref() {
                    if let Ok(count) = label.text().parse::<i32>() {
                        let new_count = (count + 1) as u32;
                        label.set_text(&Self::format_count(Some(new_count)));
                        btn.update_property(&[gtk4::accessible::Property::Label(&format!(
                            "Unlike. {} likes",
                            new_count
                        ))]);
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
                // Decrement count and update accessible label
                if let Some(label) = imp.repost_count_label.borrow().as_ref() {
                    if let Ok(count) = label.text().parse::<i32>() {
                        let new_count = (count - 1).max(0) as u32;
                        label.set_text(&Self::format_count(Some(new_count)));
                        btn.update_property(&[gtk4::accessible::Property::Label(&format!(
                            "Repost. {} reposts",
                            new_count
                        ))]);
                    }
                }
                // Update menu item label
                if let Some(label) = imp.repost_item_label.borrow().as_ref() {
                    label.set_text("Repost");
                }
            } else {
                btn.add_css_class("reposted");
                imp.is_reposted.replace(true);
                // Increment count and update accessible label
                if let Some(label) = imp.repost_count_label.borrow().as_ref() {
                    if let Ok(count) = label.text().parse::<i32>() {
                        let new_count = (count + 1) as u32;
                        label.set_text(&Self::format_count(Some(new_count)));
                        btn.update_property(&[gtk4::accessible::Property::Label(&format!(
                            "Undo repost. {} reposts",
                            new_count
                        ))]);
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

    /// Stop the row body from opening its own thread.
    ///
    /// Used for the main post in a thread view. This previously only cleared
    /// the pointer cursor, so the post still navigated on click and you could
    /// descend into the same thread indefinitely. The click callback stays in
    /// place because embedded quote cards depend on it.
    pub fn set_not_clickable(&self) {
        let imp = self.imp();
        imp.is_focused_post.set(true);
        if let Some(main_box) = imp.main_box.borrow().as_ref() {
            main_box.set_cursor_from_name(None::<&str>);
        }
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
                let like_label = format!("Unlike. {} likes", post.like_count.unwrap_or(0));
                btn.update_property(&[gtk4::accessible::Property::Label(&like_label)]);
            } else {
                btn.remove_css_class("liked");
                let like_label = format!("Like. {} likes", post.like_count.unwrap_or(0));
                btn.update_property(&[gtk4::accessible::Property::Label(&like_label)]);
            }
        }

        // Update reply button accessible label with count
        if let Some(btn) = imp.reply_btn.borrow().as_ref() {
            let reply_label = format!("Reply. {} replies", post.reply_count.unwrap_or(0));
            btn.update_property(&[gtk4::accessible::Property::Label(&reply_label)]);
        }

        // Update repost button state and menu label
        if let Some(btn) = imp.repost_btn.borrow().as_ref() {
            if post.viewer_repost.is_some() {
                btn.add_css_class("reposted");
                let repost_label =
                    format!("Undo repost. {} reposts", post.repost_count.unwrap_or(0));
                btn.update_property(&[gtk4::accessible::Property::Label(&repost_label)]);
            } else {
                btn.remove_css_class("reposted");
                let repost_label = format!("Repost. {} reposts", post.repost_count.unwrap_or(0));
                btn.update_property(&[gtk4::accessible::Property::Label(&repost_label)]);
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

        // Set composite accessible label on the PostRow for screen readers
        let text_preview = if post.text.len() > 200 {
            &post.text[..super::floor_char_boundary(&post.text, 200)]
        } else {
            &post.text
        };
        let article_label = format!(
            "Post by {} (@{}), {}: {}",
            display_name,
            post.author.handle,
            Self::format_timestamp(&post.indexed_at),
            text_preview
        );
        self.update_property(&[
            gtk4::accessible::Property::Label(&article_label),
            gtk4::accessible::Property::RoleDescription("post"),
        ]);

        // Update avatar button accessible label with display name
        // (avatar_btn is inside avatar_column but we need the parent button)
        // We set it via the avatar's parent which is the button
        if let Some(avatar_widget) = imp.avatar.borrow().as_ref() {
            if let Some(parent_btn) = avatar_widget.parent() {
                let profile_label = format!("View profile of {}", display_name);
                parent_btn.update_property(&[gtk4::accessible::Property::Label(&profile_label)]);
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
                crate::ui::external::copy_text(btn, &url, "Link to post");
            });
        }

        // Open Link action
        if let Some(open_item) = imp.open_link_item.borrow().as_ref() {
            let url = post_url;
            let popover_clone = popover;
            open_item.connect_clicked(move |btn| {
                if let Some(p) = &popover_clone {
                    p.popdown();
                }
                crate::ui::external::open_url(btn, &url, "post");
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
                // Single image - height follows the image's own aspect ratio so
                // it fills the content column edge to edge instead of sitting
                // letterboxed and centred inside a fixed 400px box.
                let img = &images[0];

                let frame = gtk4::Frame::new(None);
                frame.set_hexpand(true);
                frame.set_overflow(gtk4::Overflow::Hidden);
                frame.add_css_class("post-embed-image");
                // Clamped so a panorama is not a sliver and a tall portrait
                // does not take over the row.
                frame.set_size_request(-1, Self::embed_height(img.aspect_ratio, 180, 430));

                let picture = gtk4::Picture::new();
                picture.set_hexpand(true);
                picture.set_vexpand(true);
                // The only reason a Picture's minimum width is 0. Without it the
                // minimum is the image's intrinsic width and, with no horizontal
                // scroll valve on the timeline, that widens the whole window.
                picture.set_can_shrink(true);
                // Contain, so a lone image is shown whole. Cover crops to fill,
                // which is right for a grid cell where the neighbours have to
                // line up, but a single image has nothing to line up with and
                // cropping it is just losing the picture: measured at a 700px
                // window, Cover discarded 57% of a 2:3 portrait and 27% of a
                // 16:9 landscape. The height clamp below is an upper bound, so
                // whatever slack Contain leaves is at most a few pixels.
                picture.set_content_fit(gtk4::ContentFit::Contain);

                frame.set_child(Some(&picture));
                avatar_cache::load_image_into_picture(picture, img.thumb.clone());
                Self::attach_image_click(&frame, images, 0);
                grid_container.append(&frame);
            }
            2 => {
                // Two images side by side, both square-ish
                let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                row.set_homogeneous(true);

                for (i, img) in images.iter().enumerate() {
                    let cell = self.create_image_cell(&img.thumb, 240);
                    Self::attach_image_click(&cell, images, i);
                    row.append(&cell);
                }
                grid_container.append(&row);
            }
            3 => {
                // One tall left, two stacked right
                let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                row.set_homogeneous(true);

                // Left image - tall (spans both rows visually)
                let left = self.create_image_cell(&images[0].thumb, 300);
                Self::attach_image_click(&left, images, 0);
                row.append(&left);

                // Right column with two stacked images. 148 * 2 + 4 spacing
                // matches the left cell exactly.
                let right_col = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
                let top_right = self.create_image_cell(&images[1].thumb, 148);
                let bot_right = self.create_image_cell(&images[2].thumb, 148);
                Self::attach_image_click(&top_right, images, 1);
                Self::attach_image_click(&bot_right, images, 2);
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
                    let cell = self.create_image_cell(&img.thumb, 180);
                    Self::attach_image_click(&cell, images, i);
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
    /// Make an embedded image open the full-size viewer when clicked.
    ///
    /// The gesture claims the event so it does not also reach the row body,
    /// which would open the thread instead of the image.
    fn attach_image_click(widget: &impl IsA<gtk4::Widget>, images: &[ImageEmbed], index: usize) {
        let widget = widget.as_ref();
        widget.set_cursor_from_name(Some("pointer"));

        let gesture = gtk4::GestureClick::new();
        let images = images.to_vec();
        let widget_weak = widget.downgrade();
        gesture.connect_released(move |gesture, _, _, _| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            if let Some(widget) = widget_weak.upgrade() {
                crate::ui::media_viewer::show_images(&widget, images.clone(), index);
            }
        });
        widget.add_controller(gesture);
    }

    fn create_image_cell(&self, url: &str, height: i32) -> gtk4::Frame {
        // Frame acts as the clipping container
        let frame = gtk4::Frame::new(None);
        frame.set_hexpand(true);
        frame.set_size_request(-1, height);
        frame.set_overflow(gtk4::Overflow::Hidden);
        frame.add_css_class("post-embed-image");

        // Cover fills the cell and crops the overflow. Fill, which this used to
        // use, ignores the intrinsic aspect ratio and maps the image onto the
        // allocation - a 2:3 portrait in a 338x240 cell came out 2.4x too wide.
        let picture = gtk4::Picture::new();
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk4::ContentFit::Cover);

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

        // Vertical, matching the platform convention: thumbnail across the top,
        // then title, a two-line description and the domain.
        let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        card.add_css_class("external-card");
        // Lets the card's border-radius clip the thumbnail's top corners.
        card.set_overflow(gtk4::Overflow::Hidden);

        // Thumbnail (if available) - full card width, capped height
        if let Some(thumb_url) = &ext.thumb {
            let thumb_frame = gtk4::Frame::new(None);
            thumb_frame.set_hexpand(true);
            thumb_frame.set_overflow(gtk4::Overflow::Hidden);
            thumb_frame.add_css_class("external-thumb-top");
            // Height only. A width request here would become the card's - and
            // so the feed's - minimum width.
            thumb_frame.set_size_request(-1, 180);

            let thumb = gtk4::Picture::new();
            thumb.set_hexpand(true);
            thumb.set_vexpand(true);
            thumb.set_can_shrink(true);
            thumb.set_content_fit(gtk4::ContentFit::Cover);
            avatar_cache::load_image_into_picture(thumb.clone(), thumb_url.clone());
            thumb_frame.set_child(Some(&thumb));
            card.append(&thumb_frame);
        }

        // Text content
        let text_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        text_box.set_hexpand(true);
        text_box.add_css_class("external-card-text");

        let title = gtk4::Label::new(Some(&Self::collapse_whitespace(&ext.title)));
        title.set_halign(gtk4::Align::Start);
        title.set_xalign(0.0);
        title.add_css_class("external-title");
        title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        title.set_wrap(true);
        title.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        title.set_lines(2);
        text_box.append(&title);

        if !ext.description.is_empty() {
            // Strip any HTML tags that might be in the description from Open Graph metadata
            let clean_desc = Self::collapse_whitespace(&Self::strip_html_tags(&ext.description));
            if !clean_desc.is_empty() {
                let desc = gtk4::Label::new(Some(&clean_desc));
                desc.set_halign(gtk4::Align::Start);
                desc.set_xalign(0.0);
                desc.add_css_class("dim-label");
                desc.add_css_class("caption");
                desc.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                desc.set_wrap(true);
                desc.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
                desc.set_lines(2);
                text_box.append(&desc);
            }
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
                // The card's title and description already ellipsize, so an
                // unconstrained domain is the whole card's minimum width.
                domain_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                domain_row.append(&domain_label);
                text_box.append(&domain_row);
            }
        }

        card.append(&text_box);
        card_btn.set_child(Some(&card));

        // Open link in browser when clicked
        let url = ext.uri.clone();
        card_btn.connect_clicked(move |btn| {
            crate::ui::external::open_url(btn, &url, "link");
        });

        container.append(&card_btn);
    }

    /// Render a video embed
    fn render_video(&self, container: &gtk4::Box, video: &crate::atproto::VideoEmbed) {
        let overlay = gtk4::Overlay::new();
        overlay.add_css_class("video-embed");
        overlay.set_hexpand(true);

        // Thumbnail - fills the content column at the video's own aspect ratio.
        // The height goes on a clipping frame rather than the Picture: a
        // Picture given more width than its Contain-scaled image needs centres
        // the result, and there is no property to align it left.
        let frame = gtk4::Frame::new(None);
        frame.set_hexpand(true);
        frame.set_overflow(gtk4::Overflow::Hidden);
        frame.add_css_class("post-embed-image");
        frame.set_size_request(-1, Self::embed_height(video.aspect_ratio, 200, 400));

        let thumb = gtk4::Picture::new();
        thumb.set_hexpand(true);
        thumb.set_vexpand(true);
        thumb.set_can_shrink(true);
        thumb.set_content_fit(gtk4::ContentFit::Cover);

        if let Some(thumb_url) = &video.thumbnail {
            avatar_cache::load_image_into_picture(thumb.clone(), thumb_url.clone());
        }
        frame.set_child(Some(&thumb));
        overlay.set_child(Some(&frame));

        // Play button overlay
        let play_btn = gtk4::Button::from_icon_name("media-playback-start-symbolic");
        play_btn.add_css_class("circular");
        play_btn.add_css_class("osd");
        play_btn.set_halign(gtk4::Align::Center);
        play_btn.set_valign(gtk4::Align::Center);

        // Play in app. If the pipeline fails the viewer offers the post on the
        // web rather than opening it unasked.
        let video = video.clone();
        let post_row = self.clone();
        play_btn.connect_clicked(move |btn| {
            // The post on the web, or nothing. Falling back to the playlist
            // would hand a browser the page of raw text this player exists to
            // avoid, so the viewer drops the button instead.
            let fallback = post_row
                .imp()
                .post
                .borrow()
                .as_ref()
                .map(|p| Self::get_post_url(&p.author.handle, &p.uri));
            crate::ui::media_viewer::show_video(
                btn,
                crate::ui::video_player::VideoSource::from_embed(&video, fallback),
            );
        });

        overlay.add_overlay(&play_btn);

        container.append(&overlay);
    }

    /// Render a quote post card (clickable to open quoted post)
    fn render_quote(&self, container: &gtk4::Box, quote: &crate::atproto::QuoteEmbed) {
        // Deliberately a Box and not a Button. A GtkButton runs its click
        // gesture in the CAPTURE phase and claims the sequence on release, and
        // capture is walked ancestors-first - so a button wrapping the whole
        // card claims the release before any control *inside* the card can see
        // it, cancels their gestures, and stops propagation. That is why the
        // play button, images and link card in a quoted post navigated instead
        // of activating. A bubble-phase gesture on a plain Box is walked
        // descendants-first, so every interactive child gets first refusal and
        // this only fires for the parts of the card that nothing else claimed.
        //
        // The accessible role has to be set at construction: GTK documents that
        // it cannot be changed once set.
        let card_btn = glib::Object::builder::<gtk4::Box>()
            .property("orientation", gtk4::Orientation::Vertical)
            .property("accessible-role", gtk4::AccessibleRole::Button)
            .build();
        card_btn.add_css_class("quote-card-button");
        card_btn.set_cursor_from_name(Some("pointer"));
        // A Box is not focusable or key-activatable the way the Button was.
        card_btn.set_focusable(true);

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
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        author_row.append(&name_label);

        let handle_label = gtk4::Label::new(Some(&format!("@{}", quote.author.handle)));
        handle_label.add_css_class("dim-label");
        handle_label.add_css_class("caption");
        handle_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
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

        card_btn.append(&card);
        card_btn.update_property(&[gtk4::accessible::Property::Label(&format!(
            "Quoted post by {author_name}"
        ))]);

        // Navigate to quoted post in-app
        let quoted_post = Post {
            uri: quote.uri.clone(),
            cid: quote.cid.clone(),
            author: quote.author.clone(),
            text: quote.text.clone(),
            created_at: quote.indexed_at.clone(),
            indexed_at: quote.indexed_at.clone(),
            like_count: None,
            repost_count: None,
            reply_count: None,
            embed: quote.embed.as_deref().cloned(),
            viewer_like: None,
            viewer_repost: None,
            repost_reason: None,
            reply_context: None,
        };
        let imp = self.imp();

        let gesture = gtk4::GestureClick::new();
        // Claim on RELEASE, never on press. Claiming on press would deny every
        // descendant before they are given the release and reproduce the bug
        // this replaced. Claiming at all is still required: without it the row
        // body's own gesture also fires and one click opens both the quoted
        // post and the parent thread.
        gesture.connect_released(glib::clone!(
            #[weak]
            imp,
            #[strong]
            quoted_post,
            move |gesture, _, _, _| {
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                if let Some(cb) = imp.post_clicked_callback.borrow().as_ref() {
                    cb(quoted_post.clone());
                }
            }
        ));
        card_btn.add_controller(gesture);

        let key = gtk4::EventControllerKey::new();
        key.connect_key_pressed(glib::clone!(
            #[weak]
            imp,
            #[strong]
            quoted_post,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, keyval, _, _| {
                match keyval {
                    gdk::Key::Return
                    | gdk::Key::KP_Enter
                    | gdk::Key::space
                    | gdk::Key::KP_Space => {
                        if let Some(cb) = imp.post_clicked_callback.borrow().as_ref() {
                            cb(quoted_post.clone());
                        }
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            }
        ));
        card_btn.add_controller(key);

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

    /// Strip HTML tags from text (for cleaning up Open Graph descriptions)
    fn strip_html_tags(text: &str) -> String {
        let tag_pattern = regex::Regex::new(r"<[^>]*>").unwrap();
        tag_pattern.replace_all(text, "").to_string()
    }

    /// Fold every run of whitespace, newlines included, into a single space.
    ///
    /// `gtk_label_set_lines` is implemented as a negative Pango layout height,
    /// which means "N lines *per paragraph*". An App Store description is
    /// thirty newline-separated bullets, so a two-line clamp allows sixty
    /// lines - and because that is the label's *minimum* height, nothing
    /// downstream can take it back. Collapsing to one paragraph is what makes
    /// the clamp bind.
    fn collapse_whitespace(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Pixel height for a media embed of `aspect_ratio`, clamped to `[min, max]`.
    ///
    /// Sized so the embed fills [`CONTENT_WIDTH`] exactly at the timeline's
    /// clamp width. Narrower windows crop rather than reintroduce dead space,
    /// which is deliberate: the height cannot track the width without a tick
    /// callback or a custom layout manager, and a repeating callback outliving
    /// its row is a bug this codebase has already had three times.
    fn embed_height(aspect_ratio: Option<(u32, u32)>, min: i32, max: i32) -> i32 {
        let (w, h) = aspect_ratio
            .filter(|(w, h)| *w > 0 && *h > 0)
            .unwrap_or((16, 9));
        ((CONTENT_WIDTH * f64::from(h) / f64::from(w)).round() as i32).clamp(min, max)
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

        // Pattern for @mentions (e.g., @user.bsky.social)
        let mention_pattern = regex::Regex::new(
            r"@([a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z]([a-zA-Z0-9-]*[a-zA-Z0-9])?",
        )
        .unwrap();

        // Pattern for hashtags
        let hashtag_pattern = regex::Regex::new(r"#[a-zA-Z][a-zA-Z0-9_]*").unwrap();

        // Replace URLs with protocol - use placeholder to avoid re-matching
        let mut link_count = 0;
        let mut links: Vec<String> = Vec::new();
        result = url_with_protocol
            .replace_all(&result, |caps: &regex::Captures| {
                let url = &caps[0];
                let placeholder = format!("\x00LINK{}\x00", link_count);
                links.push(format!("<a href=\"{}\">{}</a>", url, url));
                link_count += 1;
                placeholder
            })
            .to_string();

        // Replace @mentions BEFORE bare URLs (since handles like @user.bsky.social
        // would otherwise be caught by the bare URL pattern matching .social TLD)
        result = mention_pattern
            .replace_all(&result, |caps: &regex::Captures| {
                let mention = &caps[0];
                let handle = &mention[1..]; // Strip the @ prefix for the URI
                let placeholder = format!("\x00LINK{}\x00", link_count);
                links.push(format!(
                    "<a href=\"bsky-mention://{}\">{}</a>",
                    handle, mention
                ));
                link_count += 1;
                placeholder
            })
            .to_string();

        // Pattern for bare domain URLs (domain.tld/path) - only match if not already linkified
        // Common TLDs that are likely to be URLs
        let bare_url_pattern = regex::Regex::new(
            r"\b([a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+(?:com|org|net|io|co|app|dev|edu|gov|me|info|biz|social)[/a-zA-Z0-9._~:/?#@!$&'()*+,;=-]*",
        )
        .unwrap();

        // Replace bare domain URLs
        result = bare_url_pattern
            .replace_all(&result, |caps: &regex::Captures| {
                let url = &caps[0];
                let placeholder = format!("\x00LINK{}\x00", link_count);
                links.push(format!("<a href=\"https://{}\">{}</a>", url, url));
                link_count += 1;
                placeholder
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

        // Restore all link placeholders
        for (i, link) in links.iter().enumerate() {
            let placeholder = format!("\x00LINK{}\x00", i);
            result = result.replace(&placeholder, link);
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atproto::{ExternalEmbed, QuoteEmbed, VideoEmbed};

    #[test]
    fn collapse_whitespace_folds_paragraphs_so_the_line_clamp_binds() {
        // `Label::set_lines` is a negative Pango layout height, which means
        // N lines *per paragraph*. A thirty-bullet App Store description was
        // therefore allowed sixty lines, as the label's *minimum* height, and
        // one post filled the window.
        let bullets = (1..=30)
            .map(|i| format!("• Feature bullet {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let collapsed = PostRow::collapse_whitespace(&bullets);
        assert!(!collapsed.contains('\n'), "no paragraph breaks survive");
        assert!(collapsed.starts_with("• Feature bullet 1 • Feature bullet 2"));

        // Every flavour of wire whitespace, not just \n.
        assert_eq!(
            PostRow::collapse_whitespace("a\r\nb\tc\u{2028}d  e\n\n\nf"),
            "a b c d e f"
        );
        assert_eq!(PostRow::collapse_whitespace("  \n\t "), "");
        assert_eq!(PostRow::collapse_whitespace(""), "");
    }

    #[test]
    fn embed_height_tracks_the_aspect_ratio_and_clamps() {
        // Derived from CONTENT_WIDTH rather than written out, so tuning that
        // estimate does not turn this into a failing test about a number
        // nobody chose deliberately.
        let expect = |w: f64, h: f64| (CONTENT_WIDTH * h / w).round() as i32;

        let sixteen_nine = expect(16.0, 9.0);
        assert_eq!(PostRow::embed_height(Some((16, 9)), 180, 430), sixteen_nine);
        assert_eq!(
            PostRow::embed_height(Some((3, 1)), 180, 430),
            expect(3.0, 1.0)
        );
        // A tall portrait hits the cap rather than taking over the row.
        assert_eq!(PostRow::embed_height(Some((2, 3)), 180, 430), 430);
        // A very wide panorama hits the floor rather than becoming a sliver.
        assert_eq!(PostRow::embed_height(Some((10, 1)), 180, 430), 180);
        // Missing or nonsense ratios fall back to 16:9 instead of dividing by
        // zero. The API makes aspectRatio optional and does not promise > 0.
        assert_eq!(PostRow::embed_height(None, 180, 430), sixteen_nine);
        assert_eq!(PostRow::embed_height(Some((0, 9)), 180, 430), sixteen_nine);
        assert_eq!(PostRow::embed_height(Some((16, 0)), 180, 430), sixteen_nine);
        // Whatever the estimate, the clamp is what actually bounds the row.
        assert!((180..=430).contains(&sixteen_nine));
    }

    fn profile() -> Profile {
        Profile {
            did: "did:plc:test".into(),
            handle: "someone.bsky.social".into(),
            display_name: Some("Someone".into()),
            avatar: None,
            banner: None,
            description: None,
            followers_count: None,
            following_count: None,
            posts_count: None,
            viewer_following: None,
            viewer_followed_by: None,
        }
    }

    /// A URL that fails to connect immediately, so nothing in the test waits on
    /// a network that may not be there.
    fn dead_url() -> String {
        "http://127.0.0.1:1/x.jpg".into()
    }

    fn quote_with(embed: Option<Embed>) -> QuoteEmbed {
        QuoteEmbed {
            uri: "at://did:plc:test/app.bsky.feed.post/quoted".into(),
            cid: "cid".into(),
            author: profile(),
            text: "the quoted text".into(),
            indexed_at: "2026-01-01T00:00:00Z".into(),
            embed: embed.map(Box::new),
        }
    }

    /// Every controller on `widget`, as (type name, propagation phase).
    fn controllers(widget: &gtk4::Widget) -> Vec<(String, gtk4::PropagationPhase)> {
        let list = widget.observe_controllers();
        (0..list.n_items())
            .filter_map(|i| list.item(i))
            .filter_map(|obj| obj.downcast::<gtk4::EventController>().ok())
            .map(|c| (c.type_().name().to_string(), c.propagation_phase()))
            .collect()
    }

    fn walk(widget: &gtk4::Widget, depth: usize, out: &mut Vec<(usize, gtk4::Widget)>) {
        out.push((depth, widget.clone()));
        let mut child = widget.first_child();
        while let Some(c) = child {
            walk(&c, depth + 1, out);
            child = c.next_sibling();
        }
    }

    /// A quote card must not out-rank the controls inside it.
    ///
    /// A `GtkButton` runs its click gesture in the CAPTURE phase and claims the
    /// sequence on release. Capture is walked ancestors-first, so a button
    /// wrapping the whole card answered the release before the play button, the
    /// image cells or the link card ever saw it - it claimed, GTK cancelled
    /// their gestures and stopped propagating, and the app navigated to the
    /// quoted thread instead of activating what was clicked.
    ///
    /// GTK's ordering rules make this structural, so the test is structural:
    /// nothing enclosing the card's interior may run a capture-phase gesture.
    ///
    /// Video, images, the link card and a nested quote share one test because
    /// GTK may only be used from the thread that initialised it, and the test
    /// harness gives each `#[test]` a thread of its own.
    #[test]
    fn quote_card_does_not_pre_empt_the_controls_inside_it() {
        // Headless CI has no display, and a GTK widget cannot be built without
        // one. Nothing to assert there rather than a failure to report.
        if gtk4::init().is_err() {
            eprintln!("skipping: no display available");
            return;
        }
        adw::init().expect("libadwaita");

        let row = PostRow::new();
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // A quote carrying a video, so the card contains a real GtkButton (the
        // play button) that the old wrapper used to steal the click from.
        row.render_quote(
            &container,
            &quote_with(Some(Embed::Video(VideoEmbed {
                playlist: dead_url(),
                thumbnail: Some(dead_url()),
                alt: None,
                aspect_ratio: Some((16, 9)),
            }))),
        );

        let card = container.first_child().expect("quote card was rendered");

        assert!(
            !card.is::<gtk4::Button>(),
            "the quote card wrapper must not be a GtkButton: its internal \
             gesture is capture-phase and claims on release, which pre-empts \
             every control inside the card"
        );

        let card_controllers = controllers(&card);
        let clicks: Vec<_> = card_controllers
            .iter()
            .filter(|(name, _)| name == "GtkGestureClick")
            .collect();
        assert_eq!(clicks.len(), 1, "exactly one click gesture on the card");
        assert_eq!(
            clicks[0].1,
            gtk4::PropagationPhase::Bubble,
            "the card's gesture must bubble: bubble is walked descendants-first, \
             so interactive children get first refusal"
        );

        // The Button we replaced was focusable, key-activatable and announced as
        // a button. A plain Box is none of those unless we say so.
        assert!(card.is_focusable(), "keyboard can still reach the card");
        assert_eq!(card.accessible_role(), gtk4::AccessibleRole::Button);
        assert!(
            card_controllers
                .iter()
                .any(|(name, _)| name == "GtkEventControllerKey"),
            "Enter/Space still activate the card"
        );

        // Nothing between the card and its interior may capture, either.
        let mut widgets = Vec::new();
        walk(&card, 0, &mut widgets);
        let play_depth = widgets
            .iter()
            .find(|(_, w)| w.is::<gtk4::Button>())
            .map(|(d, _)| *d)
            .expect("the nested video's play button");
        for (depth, widget) in &widgets {
            if *depth >= play_depth {
                continue;
            }
            for (name, phase) in controllers(widget) {
                assert_ne!(
                    phase,
                    gtk4::PropagationPhase::Capture,
                    "{name} on an ancestor of the play button captures, which \
                     is exactly the bug: capture runs ancestors-first"
                );
            }
        }

        // The play button keeps its own capture-phase gesture, which is what
        // lets it out-rank the card's bubble gesture and activate on its own.
        let play = widgets
            .iter()
            .find(|(_, w)| w.is::<gtk4::Button>())
            .map(|(_, w)| w.clone())
            .expect("the nested video's play button");
        assert!(
            controllers(&play)
                .iter()
                .any(|(name, phase)| name == "GtkGestureClick"
                    && *phase == gtk4::PropagationPhase::Capture),
            "GtkButton's own gesture still captures and claims on release"
        );

        // ---- Images and the external link card ----
        //
        // Images: the frame carries a bubble GestureClick that claims, and it
        // sits deeper than the card, so the bubble walk reaches it first.
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        row.render_quote(
            &container,
            &quote_with(Some(Embed::Images(vec![
                ImageEmbed {
                    thumb: dead_url(),
                    fullsize: dead_url(),
                    alt: String::new(),
                    aspect_ratio: Some((4, 3)),
                };
                4
            ]))),
        );
        let card = container.first_child().expect("quote card");
        let mut widgets = Vec::new();
        walk(&card, 0, &mut widgets);
        let cells: Vec<_> = widgets
            .iter()
            .filter(|(_, w)| {
                w.is::<gtk4::Frame>()
                    && controllers(w).iter().any(|(name, phase)| {
                        name == "GtkGestureClick" && *phase == gtk4::PropagationPhase::Bubble
                    })
            })
            .collect();
        assert_eq!(
            cells.len(),
            4,
            "every image cell is independently clickable"
        );
        for (depth, _) in &cells {
            assert!(
                *depth > 0,
                "cells are descendants of the card, so bubble first"
            );
        }

        // External link card: still a GtkButton, so it captures and claims on
        // release, out-ranking the card's bubble gesture.
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        row.render_quote(
            &container,
            &quote_with(Some(Embed::External(ExternalEmbed {
                uri: "https://apps.apple.com/app/id1".into(),
                title: "A title\nwith a newline".into(),
                description: "line one\nline two\nline three".into(),
                thumb: Some(dead_url()),
            }))),
        );
        let card = container.first_child().expect("quote card");
        let mut widgets = Vec::new();
        walk(&card, 0, &mut widgets);
        let link_btn = widgets
            .iter()
            .find(|(_, w)| w.is::<gtk4::Button>())
            .map(|(_, w)| w.clone())
            .expect("the external link card button");
        assert!(
            controllers(&link_btn)
                .iter()
                .any(|(name, phase)| name == "GtkGestureClick"
                    && *phase == gtk4::PropagationPhase::Capture),
            "the link card captures and claims, so it opens the link rather \
             than letting the quote card navigate"
        );

        // And its wire text reached the labels as a single paragraph, so the
        // two-line clamp actually clamps.
        for (_, widget) in &widgets {
            if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
                assert!(
                    !label.text().contains('\n'),
                    "label text {:?} still has a paragraph break, so set_lines \
                     will not bind",
                    label.text()
                );
            }
        }

        // ---- A nested quote (a quote of a quote) navigates to the INNER post.
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        row.render_quote(
            &container,
            &quote_with(Some(Embed::Quote(quote_with(None)))),
        );

        let outer = container.first_child().expect("outer quote card");
        let mut widgets = Vec::new();
        walk(&outer, 0, &mut widgets);
        let cards: Vec<_> = widgets
            .iter()
            .filter(|(_, w)| w.has_css_class("quote-card-button"))
            .collect();
        assert_eq!(cards.len(), 2, "outer card plus the nested one");
        // Both bubble, so the deeper one is reached first and claims.
        for (_, widget) in &cards {
            assert!(
                controllers(widget)
                    .iter()
                    .any(|(name, phase)| name == "GtkGestureClick"
                        && *phase == gtk4::PropagationPhase::Bubble)
            );
        }
        assert!(cards[1].0 > cards[0].0, "the nested card is the deeper one");
    }
}
