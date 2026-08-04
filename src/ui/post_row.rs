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

/// Post content column width at the timeline's 800px `AdwClamp`.
///
/// An estimate: the real column is 495px in a 700px window, 633 at 820, ~684
/// once the clamp binds, and the height has to be requested before any of that
/// is known. Tuned narrow; [`PostRow::embed_height`] clamps the error.
const CONTENT_WIDTH: f64 = 540.0;

/// Take off whatever the last `bind` left on a button's `clicked`.
///
/// The buttons are built once in `setup_ui` and `connect_clicked` adds a
/// handler rather than replacing one, so anything left on the button is a
/// closure still holding the post it was built for. Unconditional, so a row
/// that has stopped being a repost stops navigating to the last reposter.
fn disarm_click(btn: &gtk4::Button, slot: &std::cell::RefCell<Option<glib::SignalHandlerId>>) {
    if let Some(id) = slot.take() {
        btn.disconnect(id);
    }
}

/// Replace a button's `clicked` handler, keeping the id so the next bind can
/// take this one off in turn. See [`disarm_click`].
fn rearm_click<F: Fn(&gtk4::Button) + 'static>(
    btn: &gtk4::Button,
    slot: &std::cell::RefCell<Option<glib::SignalHandlerId>>,
    f: F,
) {
    disarm_click(btn, slot);
    slot.replace(Some(btn.connect_clicked(f)));
}

thread_local! {
    /// The signed-in user's DID. `bind` compares each post's author against
    /// it to decide whether the menu offers Delete.
    static CURRENT_USER_DID: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
    /// What a Delete click does. The app installs one handler and every row
    /// dispatches to it, so the list factories need no per-bind wiring.
    static DELETE_POST_HANDLER: std::cell::RefCell<Option<Box<dyn Fn(Post)>>> =
        const { std::cell::RefCell::new(None) };
    /// What a Save/Remove click does. Same shape as the delete handler; the
    /// row comes along weakly so the app can flip its label on success.
    static BOOKMARK_POST_HANDLER: std::cell::RefCell<
        Option<Box<dyn Fn(Post, glib::WeakRef<PostRow>)>>,
    > = const { std::cell::RefCell::new(None) };
}

/// Record whose posts are deletable. `None` on sign-out.
pub fn set_current_user_did(did: Option<&str>) {
    CURRENT_USER_DID.with(|cell| {
        cell.replace(did.map(str::to_string));
    });
}

/// Install the app-level delete flow. Thread-local for the same reason the
/// video director is: rows are built in half a dozen factories and this
/// spares each one the wiring.
pub fn set_delete_post_handler<F: Fn(Post) + 'static>(handler: F) {
    DELETE_POST_HANDLER.with(|cell| {
        cell.replace(Some(Box::new(handler)));
    });
}

/// Install the app-level save/unsave flow. See [`set_delete_post_handler`].
pub fn set_bookmark_post_handler<F: Fn(Post, glib::WeakRef<PostRow>) + 'static>(handler: F) {
    BOOKMARK_POST_HANDLER.with(|cell| {
        cell.replace(Some(Box::new(handler)));
    });
}

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
        /// The row's position at bind time, so J and K can land on the
        /// neighbouring item of the list this row sits in.
        pub list_position: Cell<u32>,
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
        /// The five handlers [`super::PostRow::bind`] installs itself. Same
        /// reason as the four above: the buttons outlive every post the row
        /// shows, so the next `bind` needs the id to disconnect.
        pub repost_attribution_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub reply_indicator_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub view_post_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub copy_link_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        pub open_link_handler_id: RefCell<Option<glib::SignalHandlerId>>,
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
        pub bookmark_item: RefCell<Option<gtk4::Button>>,
        /// Reads "Save Post" or "Remove from Saved"; bind keeps it honest.
        pub bookmark_item_label: RefCell<Option<gtk4::Label>>,
        pub delete_item: RefCell<Option<gtk4::Button>>,
        /// Separator plus Delete button, shown and hidden as one block so no
        /// stray separator trails the menu on other people's posts.
        pub delete_section: RefCell<Option<gtk4::Box>>,
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
        /// The row's video embed. Only strong ref in the process; the director
        /// holds `Weak`s.
        pub video_slot: RefCell<Option<std::rc::Rc<crate::ui::inline_video::VideoSlot>>>,
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

            // Post shortcuts, live while the row itself has focus. Focus on
            // a button inside the row keeps that button's own keys.
            klass.install_action("post.like", None, |row, _, _| row.key_like());
            klass.install_action("post.reply", None, |row, _, _| row.key_reply());
            klass.install_action("post.repost", None, |row, _, _| row.key_repost_menu());
            klass.install_action("post.open", None, |row, _, _| row.key_open());
            klass.install_action("post.menu", None, |row, _, _| row.key_menu());
            let empty = gtk4::gdk::ModifierType::empty();
            klass.add_binding_action(gtk4::gdk::Key::l, empty, "post.like");
            klass.add_binding_action(gtk4::gdk::Key::r, empty, "post.reply");
            klass.add_binding_action(gtk4::gdk::Key::t, empty, "post.repost");
            klass.add_binding_action(gtk4::gdk::Key::o, empty, "post.open");
            klass.add_binding_action(gtk4::gdk::Key::m, empty, "post.menu");
            klass.add_binding_action(gtk4::gdk::Key::Return, empty, "post.open");
            klass.add_binding_action(gtk4::gdk::Key::KP_Enter, empty, "post.open");
            klass.add_binding(gtk4::gdk::Key::j, empty, |row| {
                row.focus_neighbor(1);
                glib::Propagation::Stop
            });
            klass.add_binding(gtk4::gdk::Key::k, empty, |row| {
                row.focus_neighbor(-1);
                glib::Propagation::Stop
            });
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
        // A keyboard stop of its own; the post.* shortcuts act on it.
        self.set_focusable(true);

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

        // Repost attribution row above the header, shows "Reposted by X".
        // Clicking goes to the reposter's profile.
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
        // A label with no wrap or ellipsize reports its full text width as its
        // minimum, and AdwClamp caps natural width only, so one long display
        // name widens the feed. Every wire-text label here is ellipsized.
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

        // Reply indicator between header and content, shows "Replying to
        // @handle". Clickable.
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
                // An @mention click navigates to the profile
                let handle = uri.strip_prefix("bsky-mention://").unwrap_or("");
                let imp = post_row_for_links.imp();
                if let Some(cb) = imp.mention_clicked_callback.borrow().as_ref() {
                    cb(handle.to_string());
                }
                glib::Propagation::Stop // We handled it
            } else if uri.starts_with("bsky-tag://") {
                // Hashtag click. No navigation yet.
                let _tag = uri.strip_prefix("bsky-tag://").unwrap_or("");
                // TODO: Implement hashtag search navigation
                glib::Propagation::Stop
            } else {
                // A regular URL opens in the browser
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
        let (
            menu_btn,
            view_post_item,
            copy_link_item,
            open_link_item,
            bookmark_item,
            bookmark_item_label,
            delete_item,
            delete_section,
        ) = Self::create_post_menu_button();
        menu_btn.set_tooltip_text(Some("More options"));
        menu_btn.update_property(&[gtk4::accessible::Property::Label("More options")]);
        actions.append(&menu_btn);

        // Wired once, here. The handler reads the row's current post when
        // clicked, so rebinding has nothing to disarm. Weak for the usual
        // reason: the button is a descendant of the row.
        let row_weak = self.downgrade();
        let delete_popover = menu_btn.popover();
        delete_item.connect_clicked(move |_| {
            if let Some(p) = &delete_popover {
                p.popdown();
            }
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            let post = row.imp().post.borrow().clone();
            if let Some(post) = post {
                DELETE_POST_HANDLER.with(|cell| {
                    if let Some(handler) = cell.borrow().as_ref() {
                        handler(post);
                    }
                });
            }
        });

        // Save/Remove, wired once for the same reason as Delete.
        let row_weak = self.downgrade();
        let bookmark_popover = menu_btn.popover();
        bookmark_item.connect_clicked(move |_| {
            if let Some(p) = &bookmark_popover {
                p.popdown();
            }
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            let post = row.imp().post.borrow().clone();
            if let Some(post) = post {
                BOOKMARK_POST_HANDLER.with(|cell| {
                    if let Some(handler) = cell.borrow().as_ref() {
                        handler(post, row.downgrade());
                    }
                });
            }
        });

        content_column.append(&actions);

        main_box.append(&content_column);

        // Add click gesture to main_box for opening thread
        let gesture = gtk4::GestureClick::new();
        let post_row = self.clone();
        gesture.connect_released(move |_, _, _, _| {
            let imp = post_row.imp();
            // Already the subject of this page; navigating would push an
            // identical one.
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
        imp.bookmark_item.replace(Some(bookmark_item));
        imp.bookmark_item_label.replace(Some(bookmark_item_label));
        imp.delete_item.replace(Some(delete_item));
        imp.delete_section.replace(Some(delete_section));
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

    /// L: the like button, optimistic path and all.
    fn key_like(&self) {
        if let Some(btn) = self.imp().like_btn.borrow().as_ref() {
            btn.emit_clicked();
        }
    }

    /// R: the reply button.
    fn key_reply(&self) {
        if let Some(btn) = self.imp().reply_btn.borrow().as_ref() {
            btn.emit_clicked();
        }
    }

    /// T: the repost menu, so quote stays one arrow key away.
    fn key_repost_menu(&self) {
        if let Some(btn) = self.imp().repost_btn.borrow().as_ref() {
            btn.popup();
        }
    }

    /// O: open the thread, same rules as clicking the row body.
    fn key_open(&self) {
        let imp = self.imp();
        if imp.is_focused_post.get() {
            return;
        }
        if let Some(post) = imp.post.borrow().as_ref() {
            if let Some(cb) = imp.post_clicked_callback.borrow().as_ref() {
                cb(post.clone());
            }
        }
    }

    /// M: the overflow menu.
    fn key_menu(&self) {
        if let Some(btn) = self.imp().post_menu_btn.borrow().as_ref() {
            btn.popup();
        }
    }

    /// J and K. In a ListView, land focus on the neighbouring item; a
    /// thread page's plain box falls back to ordinary focus movement.
    fn focus_neighbor(&self, delta: i64) {
        let mut ancestor = self.parent();
        while let Some(widget) = ancestor {
            if let Some(list) = widget.downcast_ref::<gtk4::ListView>() {
                let target = self.imp().list_position.get() as i64 + delta;
                let count = list.model().map(|m| m.n_items() as i64).unwrap_or(0);
                if (0..count).contains(&target) {
                    list.scroll_to(target as u32, gtk4::ListScrollFlags::FOCUS, None);
                }
                return;
            }
            ancestor = widget.parent();
        }
        self.emit_move_focus(if delta > 0 {
            gtk4::DirectionType::Down
        } else {
            gtk4::DirectionType::Up
        });
    }

    /// Remember where this row sits in its list; see `focus_neighbor`.
    pub fn set_list_position(&self, position: u32) {
        self.imp().list_position.set(position);
    }

    /// Create a post overflow menu button with View Post, Save, Report, etc.
    /// Returns: (menu_btn, view_item, copy_link_item, open_link_item,
    /// bookmark_item, bookmark_item_label, delete_item, delete_section)
    fn create_post_menu_button() -> (
        gtk4::MenuButton,
        gtk4::Button,
        gtk4::Button,
        gtk4::Button,
        gtk4::Button,
        gtk4::Label,
        gtk4::Button,
        gtk4::Box,
    ) {
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

        // Save Post / Remove from Saved. One item; bind swaps the label to
        // match the post it is showing.
        let bookmark_item = gtk4::Button::new();
        let bookmark_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        bookmark_content.append(&gtk4::Image::from_icon_name("bookmark-new-symbolic"));
        let bookmark_item_label = gtk4::Label::new(Some("Save Post"));
        bookmark_content.append(&bookmark_item_label);
        bookmark_item.set_child(Some(&bookmark_content));
        bookmark_item.add_css_class("flat");
        popover_box.append(&bookmark_item);

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

        // Delete Post, in a block of its own. Hidden until bind sees the post
        // belongs to the signed-in user.
        let delete_section = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        let sep3 = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        sep3.set_margin_top(4);
        sep3.set_margin_bottom(4);
        delete_section.append(&sep3);

        let delete_item = gtk4::Button::new();
        let delete_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        delete_content.append(&gtk4::Image::from_icon_name("user-trash-symbolic"));
        delete_content.append(&gtk4::Label::new(Some("Delete Post...")));
        delete_item.set_child(Some(&delete_content));
        delete_item.add_css_class("flat");
        delete_item.add_css_class("destructive-action");
        delete_section.append(&delete_item);
        delete_section.set_visible(false);
        popover_box.append(&delete_section);

        let popover = gtk4::Popover::new();
        popover.set_child(Some(&popover_box));
        popover.add_css_class("menu");
        popover.set_has_arrow(false);

        let menu_btn = gtk4::MenuButton::new();
        menu_btn.set_icon_name("view-more-symbolic");
        menu_btn.add_css_class("flat");
        menu_btn.add_css_class("circular");
        menu_btn.set_popover(Some(&popover));

        (
            menu_btn,
            view_item,
            copy_link_item,
            open_link_item,
            bookmark_item,
            bookmark_item_label,
            delete_item,
            delete_section,
        )
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
    /// descend into the same thread indefinitely. The click callback stays
    /// because embedded quote cards depend on it.
    pub fn set_not_clickable(&self) {
        let imp = self.imp();
        imp.is_focused_post.set(true);
        if let Some(main_box) = imp.main_box.borrow().as_ref() {
            main_box.set_cursor_from_name(None::<&str>);
        }
    }

    /// Show `post` on this row, whatever it was showing before.
    ///
    /// The row is not necessarily new: a recycling `GtkListView` binds one
    /// pooled row to thousands of posts, so anything this only ever adds sticks
    /// for the life of the row. Two shapes of that here: a `connect_clicked`
    /// per bind, see [`disarm_click`], and state set from outside that nothing
    /// put back, like [`Self::set_not_clickable`].
    pub fn bind(&self, post: &Post) {
        let imp = self.imp();
        imp.post.replace(Some(post.clone()));

        // Clickable again until something says otherwise. `set_not_clickable`
        // runs after `bind` on a thread's focused post, and without this reset
        // the row that carried it would refuse to navigate ever after.
        imp.is_focused_post.set(false);
        if let Some(main_box) = imp.main_box.borrow().as_ref() {
            main_box.set_cursor_from_name(Some("pointer"));
        }

        // Show or hide the repost attribution. Clicking goes to the reposter's profile.
        if let Some(repost_row) = imp.repost_row.borrow().as_ref() {
            // Outside the branch. A row recycled from a repost onto a post
            // nobody reposted must not keep the old reposter wired to a button
            // it is about to hide.
            if let Some(btn) = imp.repost_attribution_btn.borrow().as_ref() {
                disarm_click(btn, &imp.repost_attribution_handler_id);
            }
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
                    // Weak. The button is a descendant of the row, so a strong
                    // clone is the row holding itself alive through its own
                    // widget tree and a pooled row is never dropped.
                    let post_row = self.downgrade();
                    let reposter_profile = reason.by.clone();
                    rearm_click(btn, &imp.repost_attribution_handler_id, move |_| {
                        let Some(post_row) = post_row.upgrade() else {
                            return;
                        };
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

        // Show or hide the reply indicator. Clicking goes to the parent author's profile.
        if let Some(reply_indicator_box) = imp.reply_indicator_box.borrow().as_ref() {
            // As above. Off first, whether or not this post puts one back.
            if let Some(btn) = imp.reply_indicator.borrow().as_ref() {
                disarm_click(btn, &imp.reply_indicator_handler_id);
            }
            if let Some(context) = &post.reply_context {
                // Update the handle label
                if let Some(label) = imp.reply_handle_label.borrow().as_ref() {
                    label.set_text(&format!("@{}", context.parent_author.handle));
                }
                // Wire up click to navigate to parent author's profile
                if let Some(btn) = imp.reply_indicator.borrow().as_ref() {
                    let post_row = self.downgrade();
                    let parent_profile = context.parent_author.clone();
                    rearm_click(btn, &imp.reply_indicator_handler_id, move |_| {
                        let Some(post_row) = post_row.upgrade() else {
                            return;
                        };
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
            Self::set_post_markup(label, &post.text);
        }

        // Render embed content
        if let Some(container) = imp.embed_container.borrow().as_ref() {
            // Before the widgets go: a recycled row's pipeline belongs to the
            // old post.
            self.release_video();

            // Clear previous embeds
            while let Some(child) = container.first_child() {
                container.remove(&child);
            }

            if let Some(embed) = &post.embed {
                self.render_embed(container, embed, true);
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

        // Delete is offered only on the signed-in user's own posts, decided
        // fresh on every bind so a recycled row cannot keep the offer.
        if let Some(section) = imp.delete_section.borrow().as_ref() {
            let own = CURRENT_USER_DID
                .with(|cell| cell.borrow().as_deref() == Some(post.author.did.as_str()));
            section.set_visible(own);
        }

        // The Save/Remove label follows the post, fresh on every bind so a
        // recycled row cannot offer to save a post the user already saved.
        if let Some(label) = imp.bookmark_item_label.borrow().as_ref() {
            label.set_text(if post.viewer_bookmarked == Some(true) {
                "Remove from Saved"
            } else {
                "Save Post"
            });
        }

        // The View Post action reuses post_clicked_callback
        if let Some(view_item) = imp.view_post_item.borrow().as_ref() {
            let post_row = self.downgrade();
            let popover_clone = popover.clone();
            rearm_click(view_item, &imp.view_post_handler_id, move |_| {
                if let Some(p) = &popover_clone {
                    p.popdown();
                }
                let Some(post_row) = post_row.upgrade() else {
                    return;
                };
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
            // The URL is captured by value, so a stale handler copies a
            // different post's link and raises a toast for it.
            let url = post_url.clone();
            let popover_clone = popover.clone();
            rearm_click(copy_item, &imp.copy_link_handler_id, move |btn| {
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
            rearm_click(open_item, &imp.open_link_handler_id, move |btn| {
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

    /// The URI of the post this row is currently showing
    pub fn post_uri(&self) -> Option<String> {
        self.imp().post.borrow().as_ref().map(|p| p.uri.clone())
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

    /// Record a save or unsave that just succeeded. Updates the stored post,
    /// so the next click toggles the other way, and the menu label with it.
    pub fn set_viewer_bookmarked(&self, saved: bool) {
        let imp = self.imp();
        if let Some(post) = imp.post.borrow_mut().as_mut() {
            post.viewer_bookmarked = Some(saved);
        }
        if let Some(label) = imp.bookmark_item_label.borrow().as_ref() {
            label.set_text(if saved {
                "Remove from Saved"
            } else {
                "Save Post"
            });
        }
    }

    /// Give up any video this row is playing. Idempotent.
    ///
    /// Called from every list factory's `unbind`, and from `bind` before the
    /// old embed's widgets go. Public because a `SignalListItemFactory` has
    /// nothing but the row to call.
    pub fn release_video(&self) {
        // take() ends the borrow before the drop. Dropping the last `Rc` runs
        // `VideoSlot::drop`, which can re-enter this `RefCell`.
        let slot = self.imp().video_slot.borrow_mut().take();
        drop(slot);
    }

    /// Render embed content into the container
    ///
    /// `inline_allowed` is false inside a quote card: a quoted video keeps its
    /// thumbnail and opens the viewer.
    fn render_embed(&self, container: &gtk4::Box, embed: &Embed, inline_allowed: bool) {
        match embed {
            Embed::Images(images) => {
                self.render_images(container, images);
            }
            Embed::External(ext) => {
                self.render_external_card(container, ext);
            }
            Embed::Video(video) => {
                self.render_video(container, video, inline_allowed);
            }
            Embed::Quote(quote) => {
                self.render_quote(container, quote);
            }
            Embed::QuoteWithMedia { quote, media } => {
                // Render media first, then quote below
                self.render_embed(container, media, inline_allowed);
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
                // The height comes from the image's own aspect ratio instead
                // of a fixed 400px box.
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
                // Without can_shrink the minimum width is the image's full
                // width. The timeline has no horizontal scrollbar, so that
                // widens the whole window.
                picture.set_can_shrink(true);
                // Contain: a lone image has nothing to line up with. Cover
                // discarded 57% of a 2:3 portrait and 27% of a 16:9 landscape
                // at a 700px window.
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

                // Left image, tall, visually spanning both rows
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

        // Cover fills the cell. Fill ignored the aspect ratio and stretched a
        // 2:3 portrait in a 338x240 cell 2.4x too wide.
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
        // Bluesky has no GIF embed type; GIFs arrive as external cards. See
        // `atproto::gif`. Anything unrecognised falls through as a link card.
        if let Some(gif) = crate::atproto::gif::detect(&ext.uri) {
            self.render_gif(container, ext, &gif);
            return;
        }

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

        // Optional thumbnail spanning the card width, height capped
        if let Some(thumb_url) = &ext.thumb {
            let thumb_frame = gtk4::Frame::new(None);
            thumb_frame.set_hexpand(true);
            thumb_frame.set_overflow(gtk4::Overflow::Hidden);
            thumb_frame.add_css_class("external-thumb-top");
            // Height only. A width request here would become the minimum
            // width of the card and then the feed.
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

    /// The alt text a GIF carries, with Bluesky's own prefix taken back off.
    ///
    /// The composer writes the picker's caption into `description` as
    /// `"ALT: <caption>"`, which is why an unrecognised GIF used to render as a
    /// link card whose subtitle read "ALT: shrug".
    fn gif_alt(ext: &crate::atproto::ExternalEmbed) -> Option<String> {
        let text = ext
            .description
            .strip_prefix("ALT: ")
            .unwrap_or(&ext.description)
            .trim();
        let text = if text.is_empty() {
            ext.title.trim()
        } else {
            text
        };
        (!text.is_empty()).then(|| format!("GIF: {text}"))
    }

    /// Render an animated GIF that arrived as an external embed.
    ///
    /// Still frame + badge + play button. An inline pipeline here would mean
    /// one `playbin3` per visible GIF in a recycled `ListView`, a pipeline per
    /// row. Clicking opens the media viewer.
    fn render_gif(
        &self,
        container: &gtk4::Box,
        ext: &crate::atproto::ExternalEmbed,
        gif: &crate::atproto::GifEmbed,
    ) {
        let overlay = gtk4::Overlay::new();
        overlay.add_css_class("video-embed");
        overlay.set_hexpand(true);

        // The GIF hosts state the exact pixel size in the link's query string,
        // so this never falls back to 16:9. Same clamp as a single image.
        let frame = gtk4::Frame::new(None);
        frame.set_hexpand(true);
        frame.set_overflow(gtk4::Overflow::Hidden);
        frame.add_css_class("post-embed-image");
        frame.set_size_request(-1, Self::embed_height(Some(gif.aspect_ratio), 180, 430));

        let thumb = gtk4::Picture::new();
        thumb.set_hexpand(true);
        thumb.set_vexpand(true);
        // Without can_shrink the thumbnail's full width becomes the row's
        // minimum.
        thumb.set_can_shrink(true);
        thumb.set_content_fit(gtk4::ContentFit::Contain);
        if let Some(alt) = Self::gif_alt(ext) {
            thumb.set_alternative_text(Some(&alt));
        }
        if let Some(thumb_url) = &ext.thumb {
            avatar_cache::load_image_into_picture(thumb.clone(), thumb_url.clone());
        }
        frame.set_child(Some(&thumb));
        overlay.set_child(Some(&frame));

        // The GIF badge.
        let badge = gtk4::Label::new(Some("GIF"));
        badge.add_css_class("gif-badge");
        badge.set_halign(gtk4::Align::Start);
        badge.set_valign(gtk4::Align::End);
        badge.set_margin_start(8);
        badge.set_margin_bottom(8);
        badge.set_can_target(false);
        overlay.add_overlay(&badge);

        let play_btn = gtk4::Button::from_icon_name("media-playback-start-symbolic");
        play_btn.add_css_class("circular");
        play_btn.add_css_class("osd");
        play_btn.set_halign(gtk4::Align::Center);
        play_btn.set_valign(gtk4::Align::Center);
        play_btn.set_tooltip_text(Some("Play GIF"));
        play_btn.update_property(&[gtk4::accessible::Property::Label("Play GIF")]);

        let gif = gif.clone();
        let alt = Self::gif_alt(ext);
        let link = ext.uri.clone();
        let post_row = self.clone();
        play_btn.connect_clicked(move |btn| {
            // The post on the web, else the GIF's own page. Never the
            // re-encoded MP4.
            let fallback = post_row
                .imp()
                .post
                .borrow()
                .as_ref()
                .map(|p| Self::get_post_url(&p.author.handle, &p.uri))
                .or_else(|| Some(link.clone()));
            crate::ui::media_viewer::show_video(
                btn,
                crate::ui::video_player::VideoSource::from_gif(&gif, alt.clone(), fallback),
            );
        });
        overlay.add_overlay(&play_btn);

        container.append(&overlay);
    }

    /// Render a video embed: clipping frame at the video's aspect ratio,
    /// thumbnail, centre play button.
    ///
    /// `inline_allowed` decides whether the button arms a
    /// [`crate::ui::inline_video::VideoSlot`] or opens the media viewer.
    fn render_video(
        &self,
        container: &gtk4::Box,
        video: &crate::atproto::VideoEmbed,
        inline_allowed: bool,
    ) {
        let overlay = gtk4::Overlay::new();
        overlay.add_css_class("video-embed");
        overlay.set_hexpand(true);

        // The height goes on a clipping frame. A Picture given more width
        // than its Contain-scaled image needs centres the result, and there is
        // no property to align it left.
        let frame = gtk4::Frame::new(None);
        frame.set_hexpand(true);
        frame.set_overflow(gtk4::Overflow::Hidden);
        frame.add_css_class("post-embed-image");
        frame.set_size_request(-1, Self::embed_height(video.aspect_ratio, 200, 400));

        let thumb = gtk4::Picture::new();
        thumb.set_hexpand(true);
        thumb.set_vexpand(true);
        thumb.set_can_shrink(true);
        // Contain, same as a lone image. The frame's height already comes
        // from this video's aspect ratio, so it fills edge to edge. Grid cells
        // keep Cover; see `create_image_cell`.
        thumb.set_content_fit(gtk4::ContentFit::Contain);
        if let Some(alt) = video
            .alt
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            thumb.set_alternative_text(Some(alt));
        }

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
        play_btn.set_tooltip_text(Some("Play video"));
        play_btn.update_property(&[gtk4::accessible::Property::Label("Play video")]);

        // The post on the web, or no button. Never the playlist.
        let fallback = self
            .imp()
            .post
            .borrow()
            .as_ref()
            .map(|p| Self::get_post_url(&p.author.handle, &p.uri));
        let source = crate::ui::video_player::VideoSource::from_embed(video, fallback);

        overlay.add_overlay(&play_btn);

        if inline_allowed {
            let slot = crate::ui::inline_video::VideoSlot::new(
                &overlay, &frame, &thumb, &play_btn, source,
            );
            // The row owns the slot; the slot owns the pipeline.
            self.imp().video_slot.replace(Some(slot));
        } else {
            play_btn.connect_clicked(move |btn| {
                crate::ui::media_viewer::show_video(btn, source.clone());
            });
        }

        container.append(&overlay);
    }

    /// Render a quote post card (clickable to open quoted post)
    fn render_quote(&self, container: &gtk4::Box, quote: &crate::atproto::QuoteEmbed) {
        // Deliberately a Box. GtkButton's click gesture runs in CAPTURE and
        // claims on release, and capture is walked ancestors-first, so a
        // button wrapping the card claimed the release before the play button,
        // images or link card inside it, and they navigated instead of
        // activating. A bubble-phase gesture on a Box is walked
        // descendants-first.
        //
        // The accessible role must be set at construction; GTK cannot change it
        // later.
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
            self.render_embed(&nested_container, nested_embed, false);
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
            viewer_bookmarked: None,
            repost_reason: None,
            reply_context: None,
        };
        let imp = self.imp();

        let gesture = gtk4::GestureClick::new();
        // Claim on RELEASE, never on press: claiming on press would deny every
        // descendant. Without claiming at all, the row body's gesture fires too
        // and one click opens both the quoted post and the parent thread.
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
    /// `gtk_label_set_lines` is a negative Pango layout height, so it clamps N
    /// lines per paragraph. A thirty-bullet App Store description got sixty
    /// lines out of set_lines(2), and that is the label's minimum height.
    fn collapse_whitespace(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Pixel height for `aspect_ratio`, clamped to `[min, max]`.
    ///
    /// Sized to fill [`CONTENT_WIDTH`] at the clamp width; the height cannot
    /// track the width without a tick callback or a custom layout manager.
    fn embed_height(aspect_ratio: Option<(u32, u32)>, min: i32, max: i32) -> i32 {
        let (w, h) = aspect_ratio
            .filter(|(w, h)| *w > 0 && *h > 0)
            .unwrap_or((16, 9));
        ((CONTENT_WIDTH * f64::from(h) / f64::from(w)).round() as i32).clamp(min, max)
    }

    /// Set post text on `label`, linkified, with a plain-text fallback.
    ///
    /// `gtk_label_set_markup` on markup it cannot parse emits a `Gtk-WARNING`
    /// and changes nothing, so a recycled row keeps the previous post's text.
    /// Clear, set markup, and if nothing arrived set it unformatted.
    ///
    /// Checked through the label itself. GtkLabel runs its own parser first
    /// and strips the `<a>` tags `pango::parse_markup` would reject.
    fn set_post_markup(label: &gtk4::Label, text: &str) {
        let markup = Self::format_post_text(text);
        label.set_text("");
        label.set_markup(&markup);
        if label.text().is_empty() && !text.is_empty() {
            eprintln!(
                "hangar: post text could not be marked up; showing it unformatted.\n  \
                 text: {text:?}\n  markup: {markup:?}"
            );
            label.set_text(text);
        }
    }

    /// Format post text with clickable links, mentions and hashtags as Pango
    /// markup. The rules live in [`crate::ui::rich_text`], shared with
    /// profile bios; the `bsky-mention://` and `bsky-tag://` schemes are
    /// handled in `connect_activate_link`.
    fn format_post_text(text: &str) -> String {
        crate::ui::rich_text::linkify(text)
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
        // set_lines clamps per paragraph, so a thirty-bullet App Store
        // description was allowed sixty lines.
        let bullets = (1..=30)
            .map(|i| format!("• Feature bullet {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let collapsed = PostRow::collapse_whitespace(&bullets);
        assert!(!collapsed.contains('\n'), "no paragraph breaks survive");
        assert!(collapsed.starts_with("• Feature bullet 1 • Feature bullet 2"));

        // Every flavour of wire whitespace: \r\n, tabs, U+2028, runs of spaces.
        assert_eq!(
            PostRow::collapse_whitespace("a\r\nb\tc\u{2028}d  e\n\n\nf"),
            "a b c d e f"
        );
        assert_eq!(PostRow::collapse_whitespace("  \n\t "), "");
        assert_eq!(PostRow::collapse_whitespace(""), "");
    }

    #[test]
    fn embed_height_tracks_the_aspect_ratio_and_clamps() {
        // Derived from CONTENT_WIDTH so tuning it doesn't break this test.
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

    /// A Klipy GIF exactly as one arrives on the wire.
    const KLIPY_GIF: &str = "https://static.klipy.com/ii/4493325008d34b7bf8cd6813cd5c1619/76/03/cgMM2dB2oEoY.gif?hh=278&ww=498&mp4=UxPaHvFAwuHpj3&webm=MCEXFCTK5uN0J47r";

    /// Whatever the post says, it has to end up on the label.
    ///
    /// Asserted through a real `GtkLabel`: it runs two parsers, its own for the
    /// `<a>` tags and then Pango's, and only the label answers for both. Every
    /// case below was a post that rendered blank.
    #[test]
    fn every_kind_of_post_text_ends_up_on_the_label() {
        crate::ui::with_gtk(every_kind_of_post_text_ends_up_on_the_label_body);
    }

    fn every_kind_of_post_text_ends_up_on_the_label_body() {
        let label = gtk4::Label::new(None);

        let cases = [
            // The three characters that are markup.
            "Tom & Jerry",
            "5 < 6 > 4 && 7 > 3",
            "she said \"hi\" and it's fine",
            // The reported crash, near enough: a control character between an
            // emoji sequence and a hyphen. GLib escapes U+001F to `&#x1f;`,
            // and the hashtag pattern used to take `#x1f` straight out of the
            // middle of it, leaving an entity with no semicolon.
            "Finally picked up Caper: Europe by @keymastergames.bsky.social, \
             excited to try it\u{1f}-just look at that art direction \
             keymaster.fun/products/caper",
            // Every other character GLib escapes numerically.
            "bell\u{7} backspace\u{8} vtab\u{b} escape\u{1b} delete\u{7f}",
            // A mention pressed up against an ampersand, both directions.
            "AT&T @someone.bsky.social & #rust",
            "@a.bsky.social&#rust",
            "&@a.bsky.social",
            // Emoji, including a skin-tone modifier and a ZWJ sequence, which
            // are the multi-codepoint cases a byte-offset bug would split.
            "\u{1F44D}\u{1F3FD} and \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F466} \u{FE0F}",
            // A query string is mostly ampersands, and it is inside an href.
            "https://example.com/s?a=1&b=2&amp=3 <tag> #done",
            "R&D at r&d.example.com",
            // Degenerate input.
            "",
            "#",
            "&",
            "<",
            "&#x1f;",
            "&amp;",
            "&&&###@@@",
        ];

        for text in cases {
            PostRow::set_post_markup(&label, text);
            assert_eq!(
                label.text().as_str(),
                text,
                "the label must show the post, not an empty string\n  markup: {}",
                PostRow::format_post_text(text)
            );
            // `use-markup` still on means the markup was accepted rather than
            // having gone down the unformatted fallback.
            assert!(
                label.uses_markup(),
                "{text:?} fell back to plain text; the markup was rejected\n  markup: {}",
                PostRow::format_post_text(text)
            );
        }

        // If unparseable markup ever reaches the label, it should still
        // show the text instead of going blank.
        label.set_text("a previous post, from before this row was recycled");
        label.set_markup("broken &markup <a href=\"x\">");
        assert!(
            label.text().contains("previous post"),
            "GtkLabel leaves the old text in place on a parse failure, which is \
             what set_post_markup exists to catch"
        );
    }

    #[test]
    fn links_mentions_and_hashtags_are_still_linkified() {
        // The escaping fix must not have quietly turned everything into plain
        // text: these are what makes the label worth marking up at all.
        let markup = PostRow::format_post_text(
            "hi @user.bsky.social see https://example.com and example.org too #rust",
        );
        assert!(
            markup.contains(r#"<a href="bsky-mention://user.bsky.social">@user.bsky.social</a>"#),
            "{markup}"
        );
        assert!(
            markup.contains(r#"<a href="https://example.com">https://example.com</a>"#),
            "{markup}"
        );
        assert!(
            markup.contains(r#"<a href="https://example.org">example.org</a>"#),
            "{markup}"
        );
        assert!(
            markup.contains(r#"<a href="bsky-tag://rust">#rust</a>"#),
            "{markup}"
        );

        // A handle must come out as a mention even though `.social` is in
        // the bare-domain TLD list.
        let markup = PostRow::format_post_text("@someone.bsky.social");
        assert!(
            markup.starts_with(r#"<a href="bsky-mention://"#),
            "{markup}"
        );

        // An `&` inside a URL is escaped in the href as well as in the body,
        // and the two agree.
        let markup = PostRow::format_post_text("https://example.com/?a=1&b=2");
        assert_eq!(
            markup,
            r#"<a href="https://example.com/?a=1&amp;b=2">https://example.com/?a=1&amp;b=2</a>"#
        );
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

    /// Standalone media is shown whole at its own aspect ratio.
    #[test]
    fn standalone_media_is_shown_whole_and_a_gif_is_playable() {
        crate::ui::with_gtk(standalone_media_is_shown_whole_and_a_gif_is_playable_body);
    }

    fn standalone_media_is_shown_whole_and_a_gif_is_playable_body() {
        let row = PostRow::new();

        let pictures = |widgets: &[(usize, gtk4::Widget)]| -> Vec<gtk4::Picture> {
            widgets
                .iter()
                .filter_map(|(_, w)| w.downcast_ref::<gtk4::Picture>().cloned())
                .collect()
        };

        // ---- A GIF, which arrives as an external embed ----
        //
        // Square, so its height is unmistakably not the 16:9 an embed with no
        // stated aspect ratio would fall back to.
        let square_gif = KLIPY_GIF.replace("hh=278&ww=498", "hh=500&ww=500");
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        row.render_embed(
            &container,
            &Embed::External(ExternalEmbed {
                uri: square_gif,
                title: "Team".into(),
                description: "ALT: a shrug".into(),
                thumb: Some(dead_url()),
            }),
            true,
        );
        let embed = container.first_child().expect("the GIF rendered something");
        assert!(
            !embed.is::<gtk4::Button>() && embed.has_css_class("video-embed"),
            "a GIF is playable media, not the link card it arrives disguised as"
        );

        let mut widgets = Vec::new();
        walk(&embed, 0, &mut widgets);
        assert!(
            widgets.iter().any(|(_, w)| w
                .downcast_ref::<gtk4::Label>()
                .is_some_and(|l| l.text() == "GIF")),
            "a still frame needs something on it saying that it moves"
        );
        assert!(
            widgets.iter().any(|(_, w)| w.is::<gtk4::Button>()),
            "and something to press to make it move"
        );
        let frame = widgets
            .iter()
            .find_map(|(_, w)| w.downcast_ref::<gtk4::Frame>().cloned())
            .expect("the clipping frame carries the height");
        assert_eq!(
            frame.height_request(),
            PostRow::embed_height(Some((500, 500)), 180, 430),
            "the GIF hosts state the exact size, so nothing here guesses 16:9"
        );
        for picture in pictures(&widgets) {
            assert_eq!(picture.content_fit(), gtk4::ContentFit::Contain);
            assert!(
                picture.can_shrink(),
                "a Picture that cannot shrink reports its image's width as the \
                 row minimum, and the timeline has no horizontal scrollbar to \
                 absorb that"
            );
        }

        // ---- An ordinary link is still an ordinary link card ----
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        row.render_embed(
            &container,
            &Embed::External(ExternalEmbed {
                uri: "https://www.theguardian.com/world/2026/aug/01/a-story".into(),
                title: "A story".into(),
                description: "Something happened".into(),
                thumb: Some(dead_url()),
            }),
            true,
        );
        assert!(
            container
                .first_child()
                .expect("the link card rendered")
                .is::<gtk4::Button>(),
            "only GIF hosts are diverted; everything else keeps its link card"
        );

        // ---- A single video, which has nothing to line up with ----
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        row.render_embed(
            &container,
            &Embed::Video(VideoEmbed {
                playlist: dead_url(),
                thumbnail: Some(dead_url()),
                alt: Some("a cat".into()),
                aspect_ratio: Some((9, 16)),
            }),
            true,
        );
        let embed = container.first_child().expect("the video rendered");
        let mut widgets = Vec::new();
        walk(&embed, 0, &mut widgets);
        let frame = widgets
            .iter()
            .find_map(|(_, w)| w.downcast_ref::<gtk4::Frame>().cloned())
            .expect("the video's clipping frame");
        assert_eq!(
            frame.height_request(),
            PostRow::embed_height(Some((9, 16)), 200, 400)
        );
        for picture in pictures(&widgets) {
            assert_eq!(
                picture.content_fit(),
                gtk4::ContentFit::Contain,
                "Cover crops to fill, and a lone video has no neighbour whose \
                 edges it has to match — cropping it is just losing the frame \
                 the author chose"
            );
            assert_eq!(picture.alternative_text().as_deref(), Some("a cat"));
        }

        // ---- Grid cells keep Cover, which is the point of the distinction ----
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        row.render_embed(
            &container,
            &Embed::Images(vec![
                ImageEmbed {
                    thumb: dead_url(),
                    fullsize: dead_url(),
                    alt: String::new(),
                    aspect_ratio: Some((2, 3)),
                };
                4
            ]),
            true,
        );
        let embed = container.first_child().expect("the grid rendered");
        let mut widgets = Vec::new();
        walk(&embed, 0, &mut widgets);
        let cells = pictures(&widgets);
        assert_eq!(cells.len(), 4);
        for picture in cells {
            assert_eq!(
                picture.content_fit(),
                gtk4::ContentFit::Cover,
                "in a grid the cells have to line up, so they crop"
            );
        }
    }

    fn post_with(embed: Option<Embed>, uri: &str) -> Post {
        Post {
            uri: uri.into(),
            cid: "cid".into(),
            author: profile(),
            text: "a post".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            indexed_at: "2026-01-01T00:00:00Z".into(),
            like_count: None,
            repost_count: None,
            reply_count: None,
            embed,
            viewer_like: None,
            viewer_repost: None,
            viewer_bookmarked: None,
            repost_reason: None,
            reply_context: None,
        }
    }

    fn a_video(playlist: &str) -> Embed {
        Embed::Video(VideoEmbed {
            playlist: playlist.into(),
            thumbnail: Some(dead_url()),
            alt: Some("a cat".into()),
            aspect_ratio: Some((16, 9)),
        })
    }

    /// A recycled row must not carry the old post's pipeline.
    ///
    /// Two paths: the factory's `unbind`, and `bind` reusing a row. Checked
    /// against `live_pipelines`.
    #[test]
    fn a_recycled_row_gives_up_its_video() {
        crate::ui::with_gtk(a_recycled_row_gives_up_its_video_body);
    }

    fn a_recycled_row_gives_up_its_video_body() {
        use crate::ui::inline_video::{Intent, VideoDirector, set_director};

        let director = VideoDirector::new(&crate::state::AppSettings::default());
        set_director(&director);
        assert_eq!(crate::media::live_pipelines(), 0);

        let row = PostRow::new();
        row.bind(&post_with(
            Some(a_video("file:///nonexistent/hangar-row-1.mp4")),
            "at://did:plc:test/app.bsky.feed.post/one",
        ));

        let slot = row
            .imp()
            .video_slot
            .borrow()
            .clone()
            .expect("a timeline video row owns a slot");
        director.arm(&slot, Intent::Manual);
        assert_eq!(
            crate::media::live_pipelines(),
            1,
            "pressing play should have built exactly one pipeline"
        );
        drop(slot);

        // Door one: the row is reused for somebody else's post.
        row.bind(&post_with(None, "at://did:plc:test/app.bsky.feed.post/two"));
        assert_eq!(
            crate::media::live_pipelines(),
            0,
            "rebinding left the previous post's pipeline running"
        );
        assert!(
            row.imp().video_slot.borrow().is_none(),
            "a post with no video left the previous post's slot in place"
        );

        // Door two: the factory unbinds the row while it is still playing.
        row.bind(&post_with(
            Some(a_video("file:///nonexistent/hangar-row-2.mp4")),
            "at://did:plc:test/app.bsky.feed.post/three",
        ));
        let slot = row
            .imp()
            .video_slot
            .borrow()
            .clone()
            .expect("the third post is a video too");
        director.arm(&slot, Intent::Manual);
        assert_eq!(crate::media::live_pipelines(), 1);
        drop(slot);

        row.release_video();
        assert_eq!(
            crate::media::live_pipelines(),
            0,
            "unbind left a pipeline bound to a row the list has taken back"
        );
        // Idempotent, because unbind and the next bind both call it.
        row.release_video();
        assert_eq!(crate::media::live_pipelines(), 0);
    }

    /// A quoted video keeps its thumbnail and opens the viewer.
    #[test]
    fn a_quoted_video_does_not_play_inline() {
        crate::ui::with_gtk(a_quoted_video_does_not_play_inline_body);
    }

    fn a_quoted_video_does_not_play_inline_body() {
        let row = PostRow::new();
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        row.render_quote(
            &container,
            &quote_with(Some(a_video("file:///nonexistent/quoted.mp4"))),
        );
        assert!(
            row.imp().video_slot.borrow().is_none(),
            "a quoted video claimed the row's one inline slot"
        );

        // ...while the same video in the post's own embed does.
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        row.render_embed(
            &container,
            &a_video("file:///nonexistent/standalone.mp4"),
            true,
        );
        assert!(
            row.imp().video_slot.borrow().is_some(),
            "a standalone video should be playable in place"
        );
        row.release_video();
    }

    /// A quote card must not out-rank the controls inside it. See
    /// `render_quote`: nothing enclosing the card's interior may run a
    /// capture-phase gesture.
    ///
    /// Video, images, the link card and a nested quote share one test because
    /// GTK may only be used from the thread that initialised it.
    #[test]
    fn quote_card_does_not_pre_empt_the_controls_inside_it() {
        // On the GTK thread, or skipped where there is no display: see
        // `crate::ui::with_gtk`.
        crate::ui::with_gtk(quote_card_does_not_pre_empt_the_controls_inside_it_body);
    }

    fn quote_card_does_not_pre_empt_the_controls_inside_it_body() {
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

    // ---------------------------------------------------------------------
    // Rebinding a row that is on screen
    // ---------------------------------------------------------------------

    /// Pump the main context for `ms`, so GTK has frames in which to lay
    /// anything out.
    fn settle(ms: u64) {
        let ctx = glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while std::time::Instant::now() < deadline {
            while ctx.iteration(false) {}
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// The timeline's shape around a column of rows: no horizontal valve, an
    /// `AdwClamp`, and a box that leaves its children their natural height.
    ///
    /// Presented, and checked for having mapped. An unallocated widget answers
    /// every geometry question with a zero or a stale number.
    fn presented_column() -> (adw::Window, gtk4::ScrolledWindow, gtk4::Box) {
        use libadwaita::prelude::AdwWindowExt;

        let column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        column.set_valign(gtk4::Align::Start);

        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&column));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));

        let window = adw::Window::new();
        window.set_default_size(700, 900);
        window.set_content(Some(&scrolled));
        window.present();
        settle(250);
        (window, scrolled, column)
    }

    /// Give up if the session will not put a window on screen.
    fn window_is_usable(window: &adw::Window, scrolled: &gtk4::ScrolledWindow) -> bool {
        if window.is_mapped() && scrolled.width() > 400 {
            return true;
        }
        eprintln!(
            "skipping: the window never mapped (mapped {}, viewport {}px wide), so \
             there is no geometry to measure",
            window.is_mapped(),
            scrolled.width()
        );
        false
    }

    fn embed_container_of(row: &PostRow) -> gtk4::Box {
        row.imp()
            .embed_container
            .borrow()
            .clone()
            .expect("every row builds its embed container in setup_ui")
    }

    /// The embed subtree as (depth, type, width, height), container included.
    ///
    /// Compared against a freshly built row instead of numbers written down
    /// here, so retuning `embed_height` or restyling a card does not break it.
    fn subtree_geometry(container: &gtk4::Box) -> Vec<(usize, String, i32, i32)> {
        let mut widgets = Vec::new();
        walk(container.upcast_ref(), 0, &mut widgets);
        widgets
            .into_iter()
            .map(|(depth, w)| (depth, w.type_().name().to_string(), w.width(), w.height()))
            .collect()
    }

    /// What GTK paints at nine points across the embed band, as
    /// `Type@depth-below-the-container`.
    ///
    /// Hit-tested rather than read off allocations. A container can have the
    /// right size and still never allocate its child; the blank band this
    /// guards against picks only the container itself.
    ///
    /// `pick` resolves downwards from the container, so where the row is
    /// scrolled to does not matter.
    fn band_hits(container: &gtk4::Box) -> Vec<String> {
        let width = f64::from(container.width());
        let height = f64::from(container.height());
        let root: gtk4::Widget = container.clone().upcast();
        let mut hits = Vec::new();
        for fy in [0.25_f64, 0.5, 0.75] {
            for fx in [0.25_f64, 0.5, 0.75] {
                let hit = container.pick(width * fx, height * fy, gtk4::PickFlags::DEFAULT);
                hits.push(match hit {
                    None => "NOTHING".to_string(),
                    Some(hit) if hit == root => "THE EMPTY CONTAINER".to_string(),
                    Some(hit) => {
                        // How far beneath the container GTK had to go. One
                        // level is a filler box; real embeds are deeper.
                        let mut depth = 0;
                        let mut cursor = Some(hit.clone());
                        while let Some(widget) = cursor {
                            if widget == root {
                                break;
                            }
                            depth += 1;
                            cursor = widget.parent();
                        }
                        format!("{}@{depth}", hit.type_().name())
                    }
                });
            }
        }
        hits
    }

    fn image(aspect_ratio: (u32, u32)) -> ImageEmbed {
        ImageEmbed {
            thumb: dead_url(),
            fullsize: dead_url(),
            alt: "alt text".into(),
            aspect_ratio: Some(aspect_ratio),
        }
    }

    /// Every kind of embed a row can be recycled onto, plus the post with no
    /// embed, which is the transition that empties the container.
    fn every_embed_kind() -> Vec<(&'static str, Option<Embed>)> {
        vec![
            ("one image", Some(Embed::Images(vec![image((16, 9))]))),
            ("two images", Some(Embed::Images(vec![image((1, 1)); 2]))),
            ("three images", Some(Embed::Images(vec![image((4, 3)); 3]))),
            ("four images", Some(Embed::Images(vec![image((2, 3)); 4]))),
            (
                "video",
                Some(Embed::Video(VideoEmbed {
                    playlist: "file:///nonexistent/rebind.mp4".into(),
                    thumbnail: Some(dead_url()),
                    alt: Some("a cat".into()),
                    aspect_ratio: Some((16, 9)),
                })),
            ),
            (
                "gif",
                Some(Embed::External(ExternalEmbed {
                    uri: KLIPY_GIF.into(),
                    title: "Team".into(),
                    description: "ALT: a shrug".into(),
                    thumb: Some(dead_url()),
                })),
            ),
            (
                "link card",
                Some(Embed::External(ExternalEmbed {
                    uri: "https://www.example.com/world/2026/aug/01/a-story".into(),
                    title: "A story about something".into(),
                    description: "Something happened, and here is a sentence about it".into(),
                    thumb: Some(dead_url()),
                })),
            ),
            ("quote", Some(Embed::Quote(quote_with(None)))),
            (
                "quote with media",
                Some(Embed::QuoteWithMedia {
                    quote: quote_with(None),
                    media: Box::new(Embed::Images(vec![image((16, 9))])),
                }),
            ),
            ("no embed", None),
        ]
    }

    /// A row that is bound again shows the new post's embed, every time.
    ///
    /// The property the virtualization migration rests on. A `GtkListView`
    /// binds one pooled row to post after post, so anything `bind` gets wrong
    /// the second time is a blank band in the feed.
    ///
    /// Each case is measured against a row built seconds earlier and bound to
    /// the same post, in the same window, at the same width. Subtree geometry
    /// and nine hit tests must match, and none of the nine may come back as the
    /// container itself, which is what a subtree allocated 0x0 looks like.
    #[test]
    fn a_rebound_row_renders_every_embed_kind_where_you_can_hit_it() {
        crate::ui::with_gtk(a_rebound_row_renders_every_embed_kind_where_you_can_hit_it_body);
    }

    fn a_rebound_row_renders_every_embed_kind_where_you_can_hit_it_body() {
        /// Three, so the cycle is walked twice and two of the binds are
        /// rebinds of a row already showing something.
        const PASSES: usize = 3;

        let (window, scrolled, column) = presented_column();
        if !window_is_usable(&window, &scrolled) {
            return;
        }

        let recycled = PostRow::new();
        column.append(&recycled);
        let recycled_container = embed_container_of(&recycled);

        for pass in 0..PASSES {
            for (kind, embed) in every_embed_kind() {
                let uri = format!("at://did:plc:test/app.bsky.feed.post/{kind}-{pass}");

                // The control. Never held anything else, same post, same
                // column, same frame.
                let fresh = PostRow::new();
                column.append(&fresh);
                fresh.bind(&post_with(embed.clone(), &uri));
                let fresh_container = embed_container_of(&fresh);

                recycled.bind(&post_with(embed.clone(), &uri));
                settle(120);

                let at = format!("pass {pass}, {kind}");

                assert_eq!(
                    recycled_container.is_visible(),
                    fresh_container.is_visible(),
                    "{at}: the recycled row's embed container is not even shown the \
                     way a new row's is"
                );

                if embed.is_none() {
                    assert!(
                        !recycled_container.is_visible()
                            && recycled_container.first_child().is_none(),
                        "{at}: a post with no embed must leave the container empty \
                         and hidden, not showing the last post's media"
                    );
                    column.remove(&fresh);
                    continue;
                }

                assert!(
                    recycled_container.width() > 0 && recycled_container.height() > 0,
                    "{at}: the embed band itself was never allocated ({}x{})",
                    recycled_container.width(),
                    recycled_container.height()
                );

                // A zero-size widget is what a child its parent never
                // allocated looks like.
                let mut widgets = Vec::new();
                walk(recycled_container.upcast_ref(), 0, &mut widgets);
                let unallocated: Vec<String> = widgets
                    .iter()
                    .skip(1)
                    .filter(|(_, w)| w.is_visible() && (w.width() == 0 || w.height() == 0))
                    .map(|(depth, w)| format!("{}{}", "  ".repeat(*depth), w.type_().name()))
                    .collect();
                assert!(
                    unallocated.is_empty(),
                    "{at}: {} widget(s) in the new embed subtree are visible but \
                     allocated 0x0, so the band is blank:\n{}",
                    unallocated.len(),
                    unallocated.join("\n")
                );

                // A blank band picks the container and nothing else. Anything
                // real is at least two levels down.
                let hits = band_hits(&recycled_container);
                assert!(
                    !hits
                        .iter()
                        .any(|h| h == "THE EMPTY CONTAINER" || h == "NOTHING"),
                    "{at}: hit-testing the band found no embed beneath the \
                     container at some of the nine sampled points: {hits:?}"
                );
                let deepest = hits
                    .iter()
                    .filter_map(|h| h.rsplit_once('@'))
                    .filter_map(|(_, d)| d.parse::<usize>().ok())
                    .max()
                    .unwrap_or(0);
                assert!(
                    deepest >= 2,
                    "{at}: the band picks only filler one level down, not a \
                     frame, picture, card or play button: {hits:?}"
                );

                // And it matches what a new row shows.
                assert_eq!(
                    hits,
                    band_hits(&fresh_container),
                    "{at}: the recycled row paints something different from a \
                     row bound to this post for the first time"
                );
                assert_eq!(
                    subtree_geometry(&recycled_container),
                    subtree_geometry(&fresh_container),
                    "{at}: the recycled row's embed subtree is not laid out the \
                     way a new row's is"
                );

                column.remove(&fresh);
            }
        }
    }

    /// One click on a recycled row does one thing.
    ///
    /// The buttons `bind` wires are built once in `setup_ui` and outlive every
    /// post the row shows. Without a disconnect a pooled row accumulates one
    /// live closure per post it has been bound to, each holding that post, so
    /// one click opens every reposter the row has shown, pushes a thread page
    /// per bind, and copies a link per bind.
    ///
    /// Counted through the row's navigation callbacks, where a user would see
    /// it, instead of by counting handlers.
    #[test]
    fn rebinding_a_row_does_not_stack_up_the_last_posts_click_handlers() {
        crate::ui::with_gtk(rebinding_a_row_does_not_stack_up_the_last_posts_click_handlers_body);
    }

    fn rebinding_a_row_does_not_stack_up_the_last_posts_click_handlers_body() {
        use std::cell::RefCell as StdRefCell;
        use std::rc::Rc;

        /// Enough that an off-by-one in the disconnect is not a pass.
        const BINDS: usize = 4;

        let row = PostRow::new();

        let profiles: Rc<StdRefCell<Vec<String>>> = Rc::default();
        let sink = Rc::clone(&profiles);
        row.set_profile_clicked_callback(move |p| sink.borrow_mut().push(p.handle.clone()));

        let posts: Rc<StdRefCell<Vec<String>>> = Rc::default();
        let sink = Rc::clone(&posts);
        row.set_post_clicked_callback(move |p| sink.borrow_mut().push(p.uri.clone()));

        let someone = |n: usize| Profile {
            handle: format!("someone{n}.bsky.social"),
            ..profile()
        };

        for n in 0..BINDS {
            let mut post = post_with(None, &format!("at://did:plc:test/app.bsky.feed.post/{n}"));
            post.repost_reason = Some(crate::atproto::RepostReason {
                by: someone(n),
                indexed_at: "2026-01-01T00:00:00Z".into(),
            });
            post.reply_context = Some(crate::atproto::ReplyContext {
                parent_author: someone(100 + n),
                root_author: someone(100 + n),
            });
            row.bind(&post);
        }
        let last = BINDS - 1;

        let imp = row.imp();
        let click = |btn: &StdRefCell<Option<gtk4::Button>>| {
            btn.borrow()
                .as_ref()
                .expect("built in setup_ui")
                .emit_clicked();
        };

        click(&imp.repost_attribution_btn);
        assert_eq!(
            *profiles.borrow(),
            vec![someone(last).handle],
            "one click on the repost attribution of a row bound {BINDS} times \
             must open one profile, and it must be this post's reposter"
        );
        profiles.borrow_mut().clear();

        click(&imp.reply_indicator);
        assert_eq!(
            *profiles.borrow(),
            vec![someone(100 + last).handle],
            "one click on the reply indicator must open one profile, and it must \
             be this post's parent author"
        );
        profiles.borrow_mut().clear();

        click(&imp.view_post_item);
        assert_eq!(
            *posts.borrow(),
            vec![format!("at://did:plc:test/app.bsky.feed.post/{last}")],
            "\"View Post and Replies\" must push one thread page, not one per \
             post the row has ever been recycled onto"
        );
        posts.borrow_mut().clear();

        // "Copy Link to Post" captures the URL by value, so a stale handler
        // writes a different post's link and raises a toast for it. Counted off
        // the clipboard's change notification.
        //
        // Compared against a control write: how many `changed` one `set_text`
        // produces is the display's business, and on X11 it is two.
        let clipboard = gtk4::gdk::Display::default()
            .expect("a display, since with_gtk started GTK")
            .clipboard();
        let notifications = Rc::new(std::cell::Cell::new(0usize));
        let counter = Rc::clone(&notifications);
        clipboard.connect_changed(move |_| counter.set(counter.get() + 1));

        let control = gtk4::Button::new();
        control.connect_clicked(|btn| {
            crate::ui::external::copy_text(btn, "one write", "Control");
        });
        control.emit_clicked();
        settle(80);
        let per_write = notifications.get();
        assert!(
            per_write >= 1,
            "the clipboard reports nothing at all, so this measures nothing"
        );

        notifications.set(0);
        click(&imp.copy_link_item);
        settle(80);
        assert_eq!(
            notifications.get(),
            per_write,
            "one click on \"Copy Link to Post\" must write the clipboard once, \
             not once per post the row has ever been recycled onto"
        );

        // "Open Link to Post" is wired the same way but must not be clicked
        // here: it hands the URL to the user's browser, one per bind under the
        // bug. Checked at the bookkeeping instead. The handler id is the whole
        // mechanism by which the next bind takes the last one off.
        assert!(
            imp.open_link_handler_id.borrow().is_some(),
            "\"Open Link to Post\" is connected but its handler is not being \
             kept, so nothing will ever disconnect it and a recycled row will \
             open one browser tab per post it has shown"
        );

        // Recycled off a repost onto an ordinary post: the old reposter must
        // not stay wired to the button being hidden.
        row.bind(&post_with(
            None,
            "at://did:plc:test/app.bsky.feed.post/plain",
        ));
        click(&imp.repost_attribution_btn);
        click(&imp.reply_indicator);
        assert!(
            profiles.borrow().is_empty(),
            "a post that is neither a repost nor a reply has no profile to \
             navigate to, but the row still went to {:?}",
            profiles.borrow()
        );
    }

    /// `set_not_clickable` applies to one post, and `bind` has to undo it.
    ///
    /// It runs after `bind` on a thread's focused post. Nothing used to put it
    /// back, so once a thread page recycles rows the row that carried the
    /// focused post would refuse to navigate ever after.
    #[test]
    fn rebinding_a_row_makes_it_an_ordinary_clickable_post_again() {
        crate::ui::with_gtk(rebinding_a_row_makes_it_an_ordinary_clickable_post_again_body);
    }

    fn rebinding_a_row_makes_it_an_ordinary_clickable_post_again_body() {
        let row = PostRow::new();
        row.bind(&post_with(
            None,
            "at://did:plc:test/app.bsky.feed.post/focused",
        ));
        row.set_not_clickable();

        let main_box = row
            .imp()
            .main_box
            .borrow()
            .clone()
            .expect("built in setup_ui");
        assert!(row.imp().is_focused_post.get());
        assert!(
            main_box.cursor().is_none(),
            "the focused post does not advertise itself as clickable"
        );

        row.bind(&post_with(
            None,
            "at://did:plc:test/app.bsky.feed.post/next",
        ));
        assert!(
            !row.imp().is_focused_post.get(),
            "a recycled row is an ordinary post again, and an ordinary post \
             opens its thread"
        );
        assert_eq!(
            main_box.cursor().and_then(|c| c.name()).as_deref(),
            Some("pointer"),
            "and it says so under the pointer"
        );
    }

    /// Delete shows itself only on the signed-in user's own posts, and one
    /// click asks for one delete however often the row has been recycled.
    #[test]
    fn delete_is_offered_only_on_own_posts_and_fires_once() {
        crate::ui::with_gtk(delete_is_offered_only_on_own_posts_and_fires_once_body);
    }

    fn delete_is_offered_only_on_own_posts_and_fires_once_body() {
        use std::cell::RefCell as StdRefCell;
        use std::rc::Rc;

        let deleted: Rc<StdRefCell<Vec<String>>> = Rc::default();
        let sink = Rc::clone(&deleted);
        super::set_delete_post_handler(move |post| sink.borrow_mut().push(post.uri));

        let row = PostRow::new();
        let section = row
            .imp()
            .delete_section
            .borrow()
            .clone()
            .expect("built in setup_ui");

        let mine = || {
            let mut post = post_with(None, "at://did:plc:me/app.bsky.feed.post/mine");
            post.author = Profile {
                did: "did:plc:me".into(),
                ..profile()
            };
            post
        };
        let theirs = post_with(None, "at://did:plc:test/app.bsky.feed.post/theirs");

        // Signed out, nothing is deletable.
        super::set_current_user_did(None);
        row.bind(&mine());
        assert!(!section.get_visible(), "no signed-in user, no Delete");

        super::set_current_user_did(Some("did:plc:me"));
        row.bind(&mine());
        assert!(section.get_visible(), "an own post offers Delete");

        row.bind(&theirs);
        assert!(
            !section.get_visible(),
            "recycled onto someone else's post, the offer has to go away"
        );

        // Bound many times over, one click still asks for one delete, and of
        // the post the row shows now.
        for _ in 0..4 {
            row.bind(&mine());
            row.bind(&theirs);
        }
        row.bind(&mine());
        assert!(section.get_visible());
        row.imp()
            .delete_item
            .borrow()
            .as_ref()
            .expect("built in setup_ui")
            .emit_clicked();
        assert_eq!(
            *deleted.borrow(),
            vec!["at://did:plc:me/app.bsky.feed.post/mine".to_string()],
            "one click on Delete must request exactly one deletion"
        );

        // Other tests bind posts authored by did:plc:test; leave nothing set.
        super::set_current_user_did(None);
    }

    /// The menu's Save entry follows the post: bind decides the label fresh
    /// each time, one click asks for one toggle of the post the row shows
    /// now, and a confirmed save flips the label without a rebind.
    #[test]
    fn save_follows_the_bound_post_and_fires_once() {
        crate::ui::with_gtk(save_follows_the_bound_post_and_fires_once_body);
    }

    fn save_follows_the_bound_post_and_fires_once_body() {
        use std::cell::RefCell as StdRefCell;
        use std::rc::Rc;

        let toggled: Rc<StdRefCell<Vec<(String, Option<bool>)>>> = Rc::default();
        let sink = Rc::clone(&toggled);
        super::set_bookmark_post_handler(move |post, _row| {
            sink.borrow_mut().push((post.uri, post.viewer_bookmarked));
        });

        let row = PostRow::new();
        let label = row
            .imp()
            .bookmark_item_label
            .borrow()
            .clone()
            .expect("built in setup_ui");

        let saved = || {
            let mut post = post_with(None, "at://did:plc:test/app.bsky.feed.post/saved");
            post.viewer_bookmarked = Some(true);
            post
        };
        let unsaved = post_with(None, "at://did:plc:test/app.bsky.feed.post/unsaved");

        row.bind(&saved());
        assert_eq!(label.text(), "Remove from Saved");

        // Recycled onto an unsaved post, the offer has to flip back.
        row.bind(&unsaved);
        assert_eq!(label.text(), "Save Post");

        // Bound many times over, one click still asks for one toggle, of the
        // post the row shows now, carrying the saved state the server call
        // has to invert.
        for _ in 0..4 {
            row.bind(&saved());
            row.bind(&unsaved);
        }
        row.imp()
            .bookmark_item
            .borrow()
            .as_ref()
            .expect("built in setup_ui")
            .emit_clicked();
        assert_eq!(
            *toggled.borrow(),
            vec![(
                "at://did:plc:test/app.bsky.feed.post/unsaved".to_string(),
                None
            )],
            "one click on Save must request exactly one toggle"
        );

        // The app confirms the save: the label flips in place and the stored
        // post flips with it, so the next click asks to remove.
        row.set_viewer_bookmarked(true);
        assert_eq!(label.text(), "Remove from Saved");
        assert_eq!(
            row.imp()
                .post
                .borrow()
                .as_ref()
                .and_then(|p| p.viewer_bookmarked),
            Some(true),
            "the stored post must flip too, or the next click saves again"
        );
    }

    /// The post shortcuts drive the row's own controls: L rides the like
    /// button's full path and O opens the thread. The row is a keyboard
    /// stop so the bindings have somewhere to live.
    #[test]
    fn the_post_shortcuts_drive_the_row() {
        crate::ui::with_gtk(the_post_shortcuts_drive_the_row_body);
    }

    fn the_post_shortcuts_drive_the_row_body() {
        use std::cell::RefCell as StdRefCell;
        use std::rc::Rc;

        let row = PostRow::new();
        assert!(row.is_focusable(), "the shortcuts need a focused row");

        row.bind(&post_with(
            None,
            "at://did:plc:test/app.bsky.feed.post/keys",
        ));

        let liked: Rc<StdRefCell<u32>> = Rc::default();
        let sink = Rc::clone(&liked);
        row.connect_like_clicked(move |_, _, _| {
            *sink.borrow_mut() += 1;
        });
        assert!(row.activate_action("post.like", None).is_ok());
        assert_eq!(*liked.borrow(), 1, "L is the like button");

        let opened: Rc<StdRefCell<Vec<String>>> = Rc::default();
        let sink = Rc::clone(&opened);
        row.set_post_clicked_callback(move |post| {
            sink.borrow_mut().push(post.uri);
        });
        assert!(row.activate_action("post.open", None).is_ok());
        assert_eq!(
            opened.borrow().as_slice(),
            ["at://did:plc:test/app.bsky.feed.post/keys"],
            "O opens the thread"
        );

        // The thread page's own subject stays put, same as a body click.
        row.imp().is_focused_post.set(true);
        assert!(row.activate_action("post.open", None).is_ok());
        assert_eq!(opened.borrow().len(), 1, "the focused post does not reopen");
    }
}
