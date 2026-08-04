// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]

//! One open conversation: history above, composer below.
//!
//! Pushed onto the chat section's stack. The page owns its cursor, its
//! in-flight flags, and the set of message ids it has shown, so a popped
//! page takes its state with it, same as the follow list pages. Fetch
//! results reach the page through weak refs; a page that has been popped
//! simply never sees them.
//!
//! History arrives newest first from the server and reads oldest first on
//! screen, so pages are reversed on the way in. Older pages splice in at
//! the top and the scroll position is nudged to keep the same messages in
//! view.

use crate::atproto::{ChatMessage, Conversation, Embed, Post, QuoteEmbed};
use crate::ui::avatar_cache;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;

/// Message limits from the chat lexicon.
const MAX_MESSAGE_GRAPHEMES: usize = 1000;
const MAX_MESSAGE_BYTES: usize = 10000;

/// How many fetches one user action may chain when pages come back short.
///
/// `get_messages` drops deleted messages, so a page can be short or empty
/// with a cursor still stored. The cap keeps a pathological server from
/// looping us forever.
const BACKFILL_CAP: u8 = 5;

/// Within this many pixels of the top counts as asking for older history.
const TOP_THRESHOLD: f64 = 200.0;

/// Within this many pixels of the bottom still counts as reading along,
/// so new messages keep the view pinned there.
const BOTTOM_SLACK: f64 = 40.0;

/// The actor in a bsky.app profile link, if that is what the link is.
/// Deeper paths are posts and lists, not the profile itself.
fn bsky_profile_target(uri: &str) -> Option<String> {
    let rest = uri
        .strip_prefix("https://bsky.app/profile/")
        .or_else(|| uri.strip_prefix("http://bsky.app/profile/"))
        .or_else(|| uri.strip_prefix("https://www.bsky.app/profile/"))?;
    let actor = rest.split(['?', '#']).next().unwrap_or("");
    if actor.is_empty() || actor.contains('/') {
        return None;
    }
    Some(actor.to_string())
}

/// A message that is nothing but a GIF link plays as a GIF, not a URL.
/// The GIF pickers send them exactly like this.
fn lone_gif_link(text: &str) -> Option<(String, crate::atproto::GifEmbed)> {
    let trimmed = text.trim();
    if trimmed.split_whitespace().count() != 1 {
        return None;
    }
    let gif = crate::atproto::gif::detect(trimmed)?;
    Some((trimmed.to_string(), gif))
}

/// Whether the composer's draft may go on the wire.
fn message_sendable(text: &str) -> bool {
    !text.trim().is_empty()
        && text.len() <= MAX_MESSAGE_BYTES
        && text.graphemes(true).count() <= MAX_MESSAGE_GRAPHEMES
}

/// Whether the desktop asks for 12 hour clocks.
///
/// A deliberately set GNOME toggle wins. Reading the setting's value
/// would answer the schema default (24h) even on a machine where nothing
/// was ever toggled, so unset falls through to the locale instead.
fn clock_uses_12h() -> bool {
    thread_local! {
        static TWELVE_HOUR: std::cell::OnceCell<bool> = const { std::cell::OnceCell::new() };
    }
    TWELVE_HOUR.with(|cell| {
        *cell.get_or_init(|| {
            let toggled = gio::SettingsSchemaSource::default()
                .and_then(|source| source.lookup("org.gnome.desktop.interface", true))
                .and_then(|_| {
                    gio::Settings::new("org.gnome.desktop.interface").user_value("clock-format")
                })
                .and_then(|value| value.str().map(str::to_string));
            match toggled.as_deref() {
                Some("12h") => true,
                Some(_) => false,
                None => locale_uses_12h(),
            }
        })
    })
}

/// Whether the locale writes one in the afternoon without a 13.
fn locale_uses_12h() -> bool {
    glib::DateTime::from_local(2000, 1, 1, 13, 0, 0.0)
        .ok()
        .and_then(|dt| dt.format("%X").ok())
        .is_some_and(|formatted| !formatted.contains("13"))
}

/// Short label text and full tooltip text for a message timestamp.
fn format_message_time(sent_at: &str) -> (String, String) {
    use chrono::DateTime;

    let Ok(time) = DateTime::parse_from_rfc3339(sent_at) else {
        return (String::new(), String::new());
    };
    format_local_time(time.with_timezone(&chrono::Local), clock_uses_12h())
}

fn format_local_time(local: chrono::DateTime<chrono::Local>, use_12h: bool) -> (String, String) {
    let (time, date_time, full) = if use_12h {
        ("%-I:%M %p", "%b %-d, %-I:%M %p", "%B %-d, %Y at %-I:%M %p")
    } else {
        ("%H:%M", "%b %-d, %H:%M", "%B %-d, %Y at %H:%M")
    };
    let short = if local.date_naive() == chrono::Local::now().date_naive() {
        local.format(time).to_string()
    } else {
        local.format(date_time).to_string()
    };
    (short, local.format(full).to_string())
}

/// What `HangarWindow::push_message_page` did with the request.
pub enum MessagePush {
    /// A new page went onto the stack and needs wiring and a first fetch.
    Pushed(MessagePage),
    /// An already-open copy was popped back to. Its callbacks and poll are
    /// live; it only needs a fetch if its first load never completed.
    PoppedBack(MessagePage),
}

mod message_object {
    use super::*;

    mod imp {
        use super::*;
        use std::cell::RefCell;

        #[derive(Default)]
        pub struct MessageObject {
            pub message: RefCell<Option<ChatMessage>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for MessageObject {
            const NAME: &'static str = "HangarMessageObject";
            type Type = super::MessageObject;
            type ParentType = glib::Object;
        }

        impl ObjectImpl for MessageObject {}
    }

    glib::wrapper! {
        pub struct MessageObject(ObjectSubclass<imp::MessageObject>);
    }

    impl MessageObject {
        pub fn new(message: ChatMessage) -> Self {
            let obj: Self = glib::Object::builder().build();
            obj.imp().message.replace(Some(message));
            obj
        }

        pub fn message(&self) -> Option<ChatMessage> {
            self.imp().message.borrow().clone()
        }
    }
}

use message_object::MessageObject;

mod message_row {
    use super::*;

    mod imp {
        use super::*;
        use std::cell::RefCell;

        #[derive(Default)]
        pub struct MessageRow {
            /// Bubble plus the reaction strip under it; halign takes the
            /// message's side here.
            pub column: RefCell<Option<gtk4::Box>>,
            pub bubble: RefCell<Option<gtk4::Box>>,
            pub text_label: RefCell<Option<gtk4::Label>>,
            pub embed_slot: RefCell<Option<gtk4::Box>>,
            pub reactions_box: RefCell<Option<gtk4::Box>>,
            pub time_label: RefCell<Option<gtk4::Label>>,
            /// The bound message, read back by menu actions and chips.
            pub message: RefCell<Option<ChatMessage>>,
            pub my_did: RefCell<Option<String>>,
            pub context_menu: RefCell<Option<gtk4::Popover>>,
            pub emoji_chooser: RefCell<Option<gtk4::EmojiChooser>>,
            pub mention_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
            pub post_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
            /// Args: message id, emoji, true to add or false to remove.
            pub react_callback: RefCell<Option<Box<dyn Fn(String, String, bool) + 'static>>>,
            pub delete_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for MessageRow {
            const NAME: &'static str = "HangarMessageRow";
            type Type = super::MessageRow;
            type ParentType = gtk4::Box;
        }

        impl ObjectImpl for MessageRow {
            fn constructed(&self) {
                self.parent_constructed();
                self.obj().setup_ui();
            }

            fn dispose(&self) {
                // Popovers are parented by hand and must be unparented the
                // same way.
                if let Some(menu) = self.context_menu.take() {
                    menu.unparent();
                }
                if let Some(chooser) = self.emoji_chooser.take() {
                    chooser.unparent();
                }
            }
        }

        impl WidgetImpl for MessageRow {}
        impl BoxImpl for MessageRow {}
    }

    glib::wrapper! {
        pub struct MessageRow(ObjectSubclass<imp::MessageRow>)
            @extends gtk4::Box, gtk4::Widget,
            @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
    }

    impl MessageRow {
        pub fn new() -> Self {
            glib::Object::builder()
                .property("orientation", gtk4::Orientation::Horizontal)
                .property("spacing", 0)
                .build()
        }

        fn setup_ui(&self) {
            self.set_hexpand(true);
            self.set_margin_start(12);
            self.set_margin_end(12);
            self.set_margin_top(3);
            self.set_margin_bottom(3);

            // Without hexpand the row gives the column its natural width
            // and the halign flip in bind never moves anything; every
            // message sat left. With it, halign places the column and the
            // bubble background still hugs the content.
            let column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            column.set_hexpand(true);

            let bubble = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            bubble.add_css_class("msg-bubble");

            // Wire text. Escaped and linkified by `set_linkified`; wrapped
            // WordChar so an unbroken run cannot widen the page.
            let text_label = gtk4::Label::new(None);
            text_label.set_wrap(true);
            text_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            text_label.set_natural_wrap_mode(gtk4::NaturalWrapMode::Word);
            text_label.set_max_width_chars(55);
            text_label.set_xalign(0.0);
            text_label.set_halign(gtk4::Align::Start);
            text_label.set_use_markup(true);

            // Links route like post text: mentions in-app, hashtags styled
            // only, anything else to the browser.
            let row_weak = self.downgrade();
            text_label.connect_activate_link(move |label, uri| {
                if let Some(handle) = uri.strip_prefix("bsky-mention://") {
                    if let Some(row) = row_weak.upgrade()
                        && let Some(cb) = row.imp().mention_callback.borrow().as_ref()
                    {
                        cb(handle.to_string());
                    }
                    glib::Propagation::Stop
                } else if uri.starts_with("bsky-tag://") {
                    glib::Propagation::Stop
                } else if let Some(actor) = bsky_profile_target(uri) {
                    // A shared profile is a mention wearing a URL.
                    if let Some(row) = row_weak.upgrade()
                        && let Some(cb) = row.imp().mention_callback.borrow().as_ref()
                    {
                        cb(actor);
                    }
                    glib::Propagation::Stop
                } else {
                    crate::ui::external::open_url(label, uri, "link");
                    glib::Propagation::Stop
                }
            });
            bubble.append(&text_label);

            // A shared post's card goes here; bind rebuilds it per message.
            let embed_slot = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            bubble.append(&embed_slot);

            let time_label = gtk4::Label::new(None);
            time_label.add_css_class("dim-label");
            time_label.add_css_class("caption");
            time_label.set_halign(gtk4::Align::End);
            bubble.append(&time_label);

            column.append(&bubble);

            // Reaction chips hang on the bubble's bottom edge, outside it;
            // bind rebuilds them per message.
            let reactions_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            reactions_box.add_css_class("msg-reactions");
            reactions_box.set_visible(false);
            column.append(&reactions_box);

            self.append(&column);

            // React, copy, delete live behind a right click or long press
            // on the bubble.
            let menu = gtk4::Popover::new();
            menu.set_parent(&bubble);
            menu.set_has_arrow(false);
            menu.add_css_class("menu");
            let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

            let react_item = gtk4::Button::with_label("React");
            react_item.add_css_class("flat");
            let row_weak = self.downgrade();
            let menu_weak = menu.downgrade();
            react_item.connect_clicked(move |_| {
                if let Some(menu) = menu_weak.upgrade() {
                    menu.popdown();
                }
                if let Some(row) = row_weak.upgrade()
                    && let Some(chooser) = row.imp().emoji_chooser.borrow().as_ref()
                {
                    chooser.popup();
                }
            });
            menu_box.append(&react_item);

            let copy_item = gtk4::Button::with_label("Copy Text");
            copy_item.add_css_class("flat");
            let row_weak = self.downgrade();
            let menu_weak = menu.downgrade();
            copy_item.connect_clicked(move |_| {
                if let Some(menu) = menu_weak.upgrade() {
                    menu.popdown();
                }
                if let Some(row) = row_weak.upgrade()
                    && let Some(message) = row.imp().message.borrow().as_ref()
                {
                    row.clipboard().set_text(&message.text);
                }
            });
            menu_box.append(&copy_item);

            let delete_item = gtk4::Button::with_label("Delete for Me");
            delete_item.add_css_class("flat");
            let row_weak = self.downgrade();
            let menu_weak = menu.downgrade();
            delete_item.connect_clicked(move |_| {
                if let Some(menu) = menu_weak.upgrade() {
                    menu.popdown();
                }
                if let Some(row) = row_weak.upgrade() {
                    let id = row.imp().message.borrow().as_ref().map(|m| m.id.clone());
                    if let Some(id) = id
                        && let Some(cb) = row.imp().delete_callback.borrow().as_ref()
                    {
                        cb(id);
                    }
                }
            });
            menu_box.append(&delete_item);
            menu.set_child(Some(&menu_box));

            let chooser = gtk4::EmojiChooser::new();
            chooser.set_parent(&bubble);
            let row_weak = self.downgrade();
            chooser.connect_emoji_picked(move |_, emoji| {
                if let Some(row) = row_weak.upgrade() {
                    let id = row.imp().message.borrow().as_ref().map(|m| m.id.clone());
                    if let Some(id) = id
                        && let Some(cb) = row.imp().react_callback.borrow().as_ref()
                    {
                        cb(id, emoji.to_string(), true);
                    }
                }
            });

            let row_weak = self.downgrade();
            let open_menu = move |x: f64, y: f64| {
                let Some(row) = row_weak.upgrade() else {
                    return;
                };
                if let Some(menu) = row.imp().context_menu.borrow().as_ref() {
                    let rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                    menu.set_pointing_to(Some(&rect));
                    menu.popup();
                }
            };

            let right_click = gtk4::GestureClick::new();
            right_click.set_button(3);
            let open = open_menu.clone();
            right_click.connect_released(move |gesture, _, x, y| {
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                open(x, y);
            });
            bubble.add_controller(right_click);

            // Double click hearts the message. Single clicks pass through
            // untouched so links in the text keep working.
            let double_click = gtk4::GestureClick::new();
            double_click.set_button(1);
            let row_weak = self.downgrade();
            double_click.connect_released(move |_, n_press, _, _| {
                if n_press == 2
                    && let Some(row) = row_weak.upgrade()
                {
                    row.react_heart();
                }
            });
            bubble.add_controller(double_click);

            let long_press = gtk4::GestureLongPress::new();
            long_press.set_touch_only(true);
            long_press.connect_pressed(move |gesture, x, y| {
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                open_menu(x, y);
            });
            bubble.add_controller(long_press);

            let imp = self.imp();
            imp.column.replace(Some(column));
            imp.bubble.replace(Some(bubble));
            imp.text_label.replace(Some(text_label));
            imp.embed_slot.replace(Some(embed_slot));
            imp.reactions_box.replace(Some(reactions_box));
            imp.time_label.replace(Some(time_label));
            imp.context_menu.replace(Some(menu));
            imp.emoji_chooser.replace(Some(chooser));
        }

        /// Fill the row for one message. Recycled rows pass through here
        /// again, so every setter overwrites what the last message left.
        pub fn bind(&self, message: &ChatMessage, mine: bool, my_did: Option<&str>) {
            let imp = self.imp();
            imp.message.replace(Some(message.clone()));
            imp.my_did.replace(my_did.map(String::from));

            if let Some(column) = imp.column.borrow().as_ref() {
                column.set_halign(if mine {
                    gtk4::Align::End
                } else {
                    gtk4::Align::Start
                });
            }
            if let Some(bubble) = imp.bubble.borrow().as_ref() {
                if mine {
                    bubble.add_css_class("msg-out");
                    bubble.remove_css_class("msg-in");
                } else {
                    bubble.add_css_class("msg-in");
                    bubble.remove_css_class("msg-out");
                }
                let (_, full) = format_message_time(&message.sent_at);
                bubble.set_tooltip_text(if full.is_empty() { None } else { Some(&full) });
            }

            let gif = lone_gif_link(&message.text);

            if let Some(label) = imp.text_label.borrow().as_ref() {
                crate::ui::rich_text::set_linkified(label, &message.text);
                // A post shared without a comment has no text of its own,
                // and a lone GIF link reads as its player below.
                label.set_visible(!message.text.is_empty() && gif.is_none());
            }

            if let Some(slot) = imp.embed_slot.borrow().as_ref() {
                while let Some(child) = slot.first_child() {
                    slot.remove(&child);
                }
                if let Some(Embed::Quote(quote)) = &message.embed {
                    slot.append(&self.shared_post_card(quote));
                } else if let Some((link, gif)) = gif {
                    slot.append(&Self::gif_player(link, gif));
                }
            }

            self.rebuild_reaction_chips(message, my_did, mine);

            if let Some(label) = imp.time_label.borrow().as_ref() {
                let (short, _) = format_message_time(&message.sent_at);
                label.set_text(&short);
            }
        }

        /// One chip per distinct emoji, counted, in first-seen order. Your
        /// own chip is marked and clicking it takes the reaction back;
        /// clicking someone else's joins in.
        fn rebuild_reaction_chips(&self, message: &ChatMessage, my_did: Option<&str>, mine: bool) {
            let imp = self.imp();
            let Some(strip) = imp.reactions_box.borrow().clone() else {
                return;
            };
            while let Some(child) = strip.first_child() {
                strip.remove(&child);
            }
            strip.set_visible(!message.reactions.is_empty());
            // Hug the bubble's edge on the message's own side.
            strip.set_halign(if mine {
                gtk4::Align::End
            } else {
                gtk4::Align::Start
            });

            let mut grouped: Vec<(String, u32, bool)> = Vec::new();
            for reaction in &message.reactions {
                let mine = my_did == Some(reaction.sender_did.as_str());
                match grouped.iter_mut().find(|(v, _, _)| v == &reaction.value) {
                    Some((_, count, own)) => {
                        *count += 1;
                        *own |= mine;
                    }
                    None => grouped.push((reaction.value.clone(), 1, mine)),
                }
            }

            for (value, count, own) in grouped {
                let chip = gtk4::Button::with_label(&format!("{value} {count}"));
                chip.add_css_class("reaction-chip");
                if own {
                    chip.add_css_class("reaction-chip-mine");
                }
                chip.update_property(&[gtk4::accessible::Property::Label(&format!(
                    "{count} reacted with {value}, {}",
                    if own { "including you" } else { "react too" }
                ))]);
                let row_weak = self.downgrade();
                let message_id = message.id.clone();
                chip.connect_clicked(move |_| {
                    if let Some(row) = row_weak.upgrade()
                        && let Some(cb) = row.imp().react_callback.borrow().as_ref()
                    {
                        cb(message_id.clone(), value.clone(), !own);
                    }
                });
                strip.append(&chip);
            }
        }

        /// Compact card for a post shared into the conversation: author,
        /// text, and a hint of its media. Clicking opens the thread, where
        /// the media plays for real. A Button, unlike the feed's quote
        /// card: nothing inside this one takes clicks of its own.
        fn shared_post_card(&self, quote: &QuoteEmbed) -> gtk4::Button {
            let card = gtk4::Button::new();
            card.add_css_class("card");
            card.add_css_class("msg-post-card");

            let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            content.set_margin_start(10);
            content.set_margin_end(10);
            content.set_margin_top(8);
            content.set_margin_bottom(8);

            let author_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            let avatar = adw::Avatar::new(20, None, true);
            let author_name = quote
                .author
                .display_name
                .as_deref()
                .unwrap_or(&quote.author.handle);
            avatar.set_text(Some(author_name));
            if let Some(url) = &quote.author.avatar {
                avatar_cache::load_avatar(avatar.clone(), url.clone());
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
            content.append(&author_row);

            if !quote.text.is_empty() {
                let text_label = gtk4::Label::new(Some(&quote.text));
                text_label.set_halign(gtk4::Align::Start);
                text_label.set_xalign(0.0);
                text_label.set_wrap(true);
                text_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
                text_label.set_max_width_chars(45);
                text_label.add_css_class("caption");
                content.append(&text_label);
            }

            if let Some(nested) = &quote.embed {
                content.append(&Self::media_hint(nested));
            }

            card.set_child(Some(&content));
            card.update_property(&[gtk4::accessible::Property::Label(&format!(
                "Shared post by {author_name}"
            ))]);

            // The card is the whole post as far as navigation cares.
            let post = Post {
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
            let row_weak = self.downgrade();
            card.connect_clicked(move |_| {
                if let Some(row) = row_weak.upgrade()
                    && let Some(cb) = row.imp().post_callback.borrow().as_ref()
                {
                    cb(post.clone());
                }
            });
            card
        }

        /// Thumbnails for images, a labeled chip for everything else. The
        /// thread view the card opens does the real rendering.
        fn media_hint(embed: &Embed) -> gtk4::Widget {
            match embed {
                Embed::Images(images) => {
                    let strip = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                    for img in images.iter().take(4) {
                        let frame = gtk4::Frame::new(None);
                        frame.set_overflow(gtk4::Overflow::Hidden);
                        frame.add_css_class("msg-card-thumb");
                        frame.set_size_request(72, 72);
                        let picture = gtk4::Picture::new();
                        picture.set_can_shrink(true);
                        picture.set_content_fit(gtk4::ContentFit::Cover);
                        if !img.alt.is_empty() {
                            picture.set_tooltip_text(Some(&img.alt));
                        }
                        frame.set_child(Some(&picture));
                        avatar_cache::load_image_into_picture(picture, img.thumb.clone());
                        strip.append(&frame);
                    }
                    strip.upcast()
                }
                Embed::Video(video) => match &video.thumbnail {
                    Some(thumb) => {
                        let frame = gtk4::Frame::new(None);
                        frame.set_overflow(gtk4::Overflow::Hidden);
                        frame.add_css_class("msg-card-thumb");
                        frame.set_size_request(128, 72);
                        frame.set_halign(gtk4::Align::Start);
                        let picture = gtk4::Picture::new();
                        picture.set_can_shrink(true);
                        picture.set_content_fit(gtk4::ContentFit::Cover);
                        if let Some(alt) = video.alt.as_deref().filter(|a| !a.is_empty()) {
                            picture.set_tooltip_text(Some(alt));
                        }
                        frame.set_child(Some(&picture));
                        avatar_cache::load_image_into_picture(picture, thumb.clone());
                        frame.upcast()
                    }
                    None => Self::media_chip("video-x-generic-symbolic", "Video"),
                },
                Embed::External(external) => {
                    if crate::atproto::gif::detect(&external.uri).is_some() {
                        Self::media_chip("image-x-generic-symbolic", "GIF")
                    } else {
                        let label = if external.title.is_empty() {
                            &external.uri
                        } else {
                            &external.title
                        };
                        Self::media_chip("emblem-symbolic-link", label)
                    }
                }
                Embed::Quote(_) | Embed::QuoteWithMedia { .. } => {
                    Self::media_chip("mail-forward-symbolic", "Quoted post")
                }
            }
        }

        /// A play affordance for a GIF sent as a bare link. The pickers
        /// give no poster frame, so the badge and button carry it; play
        /// opens the same dialog player GIFs use in the feed.
        fn gif_player(link: String, gif: crate::atproto::GifEmbed) -> gtk4::Widget {
            let overlay = gtk4::Overlay::new();

            let frame = gtk4::Frame::new(None);
            frame.set_overflow(gtk4::Overflow::Hidden);
            frame.add_css_class("msg-card-thumb");
            // Sized from the link's own stated pixels, capped for a bubble.
            let (w, h) = gif.aspect_ratio;
            let width = 240.0_f32;
            let height = (width * h as f32 / w.max(1) as f32).clamp(100.0, 320.0);
            frame.set_size_request(width as i32, height as i32);
            overlay.set_child(Some(&frame));

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
            play_btn.connect_clicked(move |btn| {
                crate::ui::media_viewer::show_video(
                    btn,
                    crate::ui::video_player::VideoSource::from_gif(&gif, None, Some(link.clone())),
                );
            });
            overlay.add_overlay(&play_btn);

            overlay.upcast()
        }

        fn media_chip(icon: &str, text: &str) -> gtk4::Widget {
            let chip = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            chip.set_halign(gtk4::Align::Start);
            chip.append(&gtk4::Image::from_icon_name(icon));
            let label = gtk4::Label::new(Some(text));
            label.add_css_class("dim-label");
            label.add_css_class("caption");
            label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            label.set_max_width_chars(40);
            chip.append(&label);
            chip.upcast()
        }

        /// Replace the handler run when a mention in the text is activated.
        pub fn set_mention_callback<F: Fn(String) + 'static>(&self, callback: F) {
            self.imp()
                .mention_callback
                .replace(Some(Box::new(callback)));
        }

        /// Replace the handler run when a shared post's card is activated.
        pub fn set_post_callback<F: Fn(Post) + 'static>(&self, callback: F) {
            self.imp().post_callback.replace(Some(Box::new(callback)));
        }

        /// Heart the bound message, the double-click shortcut. Adding is
        /// idempotent server-side, so a second double-click is harmless.
        pub(super) fn react_heart(&self) {
            let id = self.imp().message.borrow().as_ref().map(|m| m.id.clone());
            if let Some(id) = id
                && let Some(cb) = self.imp().react_callback.borrow().as_ref()
            {
                cb(id, "\u{2764}\u{FE0F}".to_string(), true);
            }
        }

        /// Replace the reaction handler: message id, emoji, add or remove.
        pub fn set_react_callback<F: Fn(String, String, bool) + 'static>(&self, callback: F) {
            self.imp().react_callback.replace(Some(Box::new(callback)));
        }

        /// Replace the handler run when Delete for Me is chosen.
        pub fn set_delete_callback<F: Fn(String) + 'static>(&self, callback: F) {
            self.imp().delete_callback.replace(Some(Box::new(callback)));
        }
    }

    impl Default for MessageRow {
        fn default() -> Self {
            Self::new()
        }
    }
}

use message_row::MessageRow;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct MessagePage {
        pub model: RefCell<Option<gio::ListStore>>,
        /// Ids already shown, so poll pages and echoes of our own sends
        /// merge instead of duplicating rows.
        pub known_ids: RefCell<HashSet<String>>,
        pub convo_id: RefCell<String>,
        pub my_did: RefCell<Option<String>>,
        pub title: RefCell<String>,
        pub overlay: RefCell<Option<gtk4::Overlay>>,
        pub spinner: RefCell<Option<gtk4::Spinner>>,
        pub empty_state: RefCell<Option<adw::StatusPage>>,
        pub error_state: RefCell<Option<adw::StatusPage>>,
        pub vadjustment: RefCell<Option<gtk4::Adjustment>>,
        pub entry: RefCell<Option<gtk4::Entry>>,
        pub send_button: RefCell<Option<gtk4::Button>>,
        /// Cursor into older history; None once the top is reached.
        pub cursor: RefCell<Option<String>>,
        /// A history fetch is out, first page or older.
        pub fetching: Cell<bool>,
        /// A background poll fetch is out.
        pub polling: Cell<bool>,
        pub sending: Cell<bool>,
        pub loaded_once: Cell<bool>,
        /// The view is at the bottom and should stay there as rows land.
        pub pinned: Cell<bool>,
        /// Upper bound recorded just before a prepend, so the scroll can
        /// keep the same messages in view once the rows land.
        pub anchor_upper: Cell<Option<f64>>,
        /// Chained fetches left before the next user action re-arms it.
        pub backfill_budget: Cell<u8>,
        pub load_older_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub retry_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub send_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
        pub mention_clicked_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
        pub post_clicked_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
        /// Args: message id, emoji, add or remove.
        pub react_callback: RefCell<Option<Box<dyn Fn(String, String, bool) + 'static>>>,
        pub delete_message_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessagePage {
        const NAME: &'static str = "HangarMessagePage";
        type Type = super::MessagePage;
        type ParentType = gtk4::Box;
    }

    impl ObjectImpl for MessagePage {}
    impl WidgetImpl for MessagePage {}
    impl BoxImpl for MessagePage {}
}

glib::wrapper! {
    pub struct MessagePage(ObjectSubclass<imp::MessagePage>)
        @extends gtk4::Box, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
}

impl MessagePage {
    pub fn new(conversation: &Conversation, my_did: Option<&str>) -> Self {
        let page: Self = glib::Object::builder()
            .property("orientation", gtk4::Orientation::Vertical)
            .property("spacing", 0)
            .build();

        let imp = page.imp();
        imp.convo_id.replace(conversation.id.clone());
        imp.my_did.replace(my_did.map(String::from));

        // The person across the table, ourselves filtered out. Group chats
        // fall back to whoever is listed first.
        let other = conversation
            .members
            .iter()
            .find(|m| my_did.is_none_or(|did| m.did != did))
            .or_else(|| conversation.members.first());
        let title = other
            .map(|m| m.display_name.clone().unwrap_or_else(|| m.handle.clone()))
            .unwrap_or_else(|| "Conversation".to_string());
        imp.title.replace(title);

        // The handle is wire text and the status page takes markup.
        let empty_description = match other {
            Some(m) => format!("Say hello to @{}.", glib::markup_escape_text(&m.handle)),
            None => "Say hello.".to_string(),
        };

        page.setup_ui(&empty_description);
        // Opening the page is the first user action.
        imp.backfill_budget.set(BACKFILL_CAP);
        page
    }

    fn setup_ui(&self, empty_description: &str) {
        self.set_vexpand(true);

        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);

        let model = gio::ListStore::new::<MessageObject>();
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let row = MessageRow::new();
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                // The list's activation machinery claims first clicks, so a
                // double click never counted to two for the heart shortcut.
                list_item.set_activatable(false);
                list_item.set_selectable(false);
                list_item.set_child(Some(&row));
            }
        });

        let page_weak = self.downgrade();
        factory.connect_bind(move |_, item| {
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(object) = list_item.item().and_downcast::<MessageObject>()
                && let Some(message) = object.message()
                && let Some(row) = list_item.child().and_downcast::<MessageRow>()
            {
                let my_did = page.imp().my_did.borrow().clone();
                let mine = my_did.as_deref() == Some(message.sender_did.as_str());
                row.bind(&message, mine, my_did.as_deref());
                let page_weak = page.downgrade();
                row.set_mention_callback(move |handle| {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    if let Some(cb) = page.imp().mention_clicked_callback.borrow().as_ref() {
                        cb(handle);
                    }
                });
                let page_weak = page.downgrade();
                row.set_post_callback(move |post| {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    if let Some(cb) = page.imp().post_clicked_callback.borrow().as_ref() {
                        cb(post);
                    }
                });
                let page_weak = page.downgrade();
                row.set_react_callback(move |message_id, value, add| {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    if let Some(cb) = page.imp().react_callback.borrow().as_ref() {
                        cb(message_id, value, add);
                    }
                });
                let page_weak = page.downgrade();
                row.set_delete_callback(move |message_id| {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    if let Some(cb) = page.imp().delete_message_callback.borrow().as_ref() {
                        cb(message_id);
                    }
                });
            }
        });

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // The virtualized chain every feed uses: scroller over clamp
        // scrollable over list view, nothing between. See `build_timeline`.
        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Older history loads at the top, so the spinner sits there.
        let spinner = gtk4::Spinner::new();
        spinner.update_property(&[gtk4::accessible::Property::Label("Loading")]);
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::Start);
        spinner.set_margin_top(16);
        overlay.add_overlay(&spinner);

        // Shown once a first load comes back with nothing to show.
        let empty_state = adw::StatusPage::new();
        empty_state.set_icon_name(Some("chat-message-new-symbolic"));
        empty_state.set_title("No messages yet");
        empty_state.set_description(Some(empty_description));
        empty_state.set_vexpand(true);
        empty_state.set_visible(false);

        // Shown when a fetch fails with nothing listed yet.
        let error_state = adw::StatusPage::new();
        error_state.set_icon_name(Some("dialog-warning-symbolic"));
        error_state.set_title("Couldn't Load This Conversation");
        error_state.set_description(Some("Check your connection and try again."));
        error_state.set_vexpand(true);
        error_state.set_visible(false);

        let retry_button = gtk4::Button::with_label("Try Again");
        retry_button.add_css_class("pill");
        retry_button.add_css_class("suggested-action");
        retry_button.set_halign(gtk4::Align::Center);
        let page_weak = self.downgrade();
        retry_button.connect_clicked(move |_| {
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            // A retry is a user action.
            page.imp().backfill_budget.set(BACKFILL_CAP);
            if let Some(cb) = page.imp().retry_callback.borrow().as_ref() {
                cb();
            }
        });
        error_state.set_child(Some(&retry_button));

        // The composer. Enter and the button both send.
        let composer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        composer.set_margin_start(12);
        composer.set_margin_end(12);
        composer.set_margin_top(8);
        composer.set_margin_bottom(8);

        let entry = gtk4::Entry::new();
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some("Write a message"));
        let page_weak = self.downgrade();
        entry.connect_activate(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.try_send();
            }
        });
        let page_weak = self.downgrade();
        entry.connect_changed(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.update_send_state();
            }
        });
        composer.append(&entry);

        let send_button = gtk4::Button::with_label("Send");
        send_button.add_css_class("suggested-action");
        send_button.set_sensitive(false);
        let page_weak = self.downgrade();
        send_button.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.try_send();
            }
        });
        composer.append(&send_button);

        // Same width as the list above it.
        let composer_clamp = adw::Clamp::new();
        composer_clamp.set_maximum_size(800);
        composer_clamp.set_tightening_threshold(600);
        composer_clamp.set_child(Some(&composer));

        self.append(&overlay);
        self.append(&empty_state);
        self.append(&error_state);
        self.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        self.append(&composer_clamp);

        // Wired once here. Nearing the top asks for older history; the
        // pinned flag tracks whether the reader is at the bottom.
        let adj = scrolled.vadjustment();
        let page_weak = self.downgrade();
        adj.connect_value_changed(move |adj| {
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            let imp = page.imp();
            imp.pinned
                .set(adj.value() >= adj.upper() - adj.page_size() - BOTTOM_SLACK);
            if adj.value() < TOP_THRESHOLD
                && imp.loaded_once.get()
                && !imp.fetching.get()
                && imp.cursor.borrow().is_some()
            {
                // A real scroll re-arms the chained-fetch budget.
                imp.backfill_budget.set(BACKFILL_CAP);
                if let Some(cb) = imp.load_older_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        // Runs when the content size changes. A prepend keeps the same
        // messages in view; anything else keeps the bottom pinned while
        // the reader is there.
        let page_weak = self.downgrade();
        adj.connect_changed(move |adj| {
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            let imp = page.imp();
            if let Some(old_upper) = imp.anchor_upper.take() {
                let delta = adj.upper() - old_upper;
                if delta > 0.0 {
                    adj.set_value(adj.value() + delta);
                }
                return;
            }
            if imp.pinned.get() {
                adj.set_value(adj.upper() - adj.page_size());
            }
        });

        let imp = self.imp();
        imp.pinned.set(true);
        imp.model.replace(Some(model));
        imp.overlay.replace(Some(overlay));
        imp.spinner.replace(Some(spinner));
        imp.empty_state.replace(Some(empty_state));
        imp.error_state.replace(Some(error_state));
        imp.vadjustment.replace(Some(adj));
        imp.entry.replace(Some(entry));
        imp.send_button.replace(Some(send_button));
    }

    pub fn convo_id(&self) -> String {
        self.imp().convo_id.borrow().clone()
    }

    /// The other member's name, for the page header.
    pub fn conversation_title(&self) -> String {
        self.imp().title.borrow().clone()
    }

    /// Replace the whole list with the newest page of history, as the
    /// server sent it: newest first.
    pub fn set_initial_messages(&self, newest_first: Vec<ChatMessage>) {
        let imp = self.imp();
        imp.known_ids.borrow_mut().clear();
        let mut messages = newest_first;
        messages.reverse();
        if let Some(model) = imp.model.borrow().as_ref() {
            model.remove_all();
            for message in messages {
                imp.known_ids.borrow_mut().insert(message.id.clone());
                model.append(&MessageObject::new(message));
            }
        }
        imp.loaded_once.set(true);
        imp.pinned.set(true);
        self.refresh_visibility();
    }

    /// Splice a page of older history in at the top, newest first as the
    /// server sent it. Overlap with what is shown is dropped by id.
    pub fn prepend_older(&self, newest_first: Vec<ChatMessage>) {
        let imp = self.imp();
        let fresh: Vec<ChatMessage> = {
            let known = imp.known_ids.borrow();
            newest_first
                .into_iter()
                .filter(|m| !known.contains(&m.id))
                .collect()
        };
        if fresh.is_empty() {
            return;
        }
        // Keep the reader's place: record the extent now, and the changed
        // handler shifts the scroll by however much the prepend grew it.
        if let Some(adj) = imp.vadjustment.borrow().as_ref() {
            imp.anchor_upper.set(Some(adj.upper()));
        }
        let mut fresh = fresh;
        fresh.reverse();
        let objects: Vec<MessageObject> = fresh
            .into_iter()
            .map(|m| {
                imp.known_ids.borrow_mut().insert(m.id.clone());
                MessageObject::new(m)
            })
            .collect();
        if let Some(model) = imp.model.borrow().as_ref() {
            model.splice(0, 0, &objects);
        }
        self.refresh_visibility();
    }

    /// Fold a freshly polled page into the list and say how many messages
    /// were actually new. Zero new messages means nothing to mark read.
    pub fn merge_new(&self, newest_first: Vec<ChatMessage>) -> u32 {
        let imp = self.imp();
        let (fresh, known_again): (Vec<ChatMessage>, Vec<ChatMessage>) = {
            let known = imp.known_ids.borrow();
            newest_first
                .into_iter()
                .partition(|m| !known.contains(&m.id))
        };

        // Reactions land on messages already listed; carry them over.
        for message in known_again {
            self.update_message(message);
        }

        if fresh.is_empty() {
            return 0;
        }
        let mut fresh = fresh;
        fresh.reverse();
        let added = fresh.len() as u32;
        if let Some(model) = imp.model.borrow().as_ref() {
            for message in fresh {
                imp.known_ids.borrow_mut().insert(message.id.clone());
                model.append(&MessageObject::new(message));
            }
        }
        self.refresh_visibility();
        added
    }

    /// Replace a listed message with the server's newer copy, if it is
    /// actually newer. Reactions are the only part that changes in place.
    pub fn update_message(&self, message: ChatMessage) {
        let Some(model) = self.imp().model.borrow().clone() else {
            return;
        };
        for i in 0..model.n_items() {
            let Some(object) = model.item(i).and_downcast::<MessageObject>() else {
                continue;
            };
            let Some(listed) = object.message() else {
                continue;
            };
            if listed.id != message.id {
                continue;
            }
            if listed.reactions != message.reactions {
                model.splice(i, 1, &[MessageObject::new(message)]);
            }
            return;
        }
    }

    /// Take a message out of the list after a delete-for-me went through.
    /// Its id stays known, so a straggling poll cannot bring it back.
    pub fn remove_message(&self, message_id: &str) {
        let Some(model) = self.imp().model.borrow().clone() else {
            return;
        };
        for i in 0..model.n_items() {
            let matches = model
                .item(i)
                .and_downcast::<MessageObject>()
                .and_then(|o| o.message())
                .is_some_and(|m| m.id == message_id);
            if matches {
                model.remove(i);
                break;
            }
        }
        self.refresh_visibility();
    }

    /// The send went through: clear the draft and show the message the
    /// server stored.
    pub fn sent_ok(&self, message: ChatMessage) {
        let imp = self.imp();
        imp.sending.set(false);
        if let Some(entry) = imp.entry.borrow().as_ref() {
            entry.set_text("");
        }
        imp.pinned.set(true);
        self.append_new(message);
        self.update_send_state();
    }

    /// The send failed. The draft stays where it is; the caller says why.
    pub fn send_failed(&self) {
        self.set_sending(false);
    }

    /// Append one message unless a poll already brought it in.
    fn append_new(&self, message: ChatMessage) {
        let imp = self.imp();
        if !imp.known_ids.borrow_mut().insert(message.id.clone()) {
            return;
        }
        if let Some(model) = imp.model.borrow().as_ref() {
            model.append(&MessageObject::new(message));
        }
        self.refresh_visibility();
    }

    /// Decide between the list and the empty state. Only a spent cursor
    /// proves a conversation is empty; a page can be all deleted messages
    /// with more history behind it.
    fn refresh_visibility(&self) {
        let imp = self.imp();
        let empty = imp.model.borrow().as_ref().is_none_or(|m| m.n_items() == 0)
            && imp.cursor.borrow().is_none();
        if let Some(overlay) = imp.overlay.borrow().as_ref() {
            overlay.set_visible(!empty);
        }
        if let Some(empty_state) = imp.empty_state.borrow().as_ref() {
            empty_state.set_visible(empty && imp.loaded_once.get());
        }
        if let Some(error_state) = imp.error_state.borrow().as_ref() {
            error_state.set_visible(false);
        }
    }

    /// Mark a history fetch in flight and show or hide the spinner with
    /// it. Starting one also stands the error state down.
    pub fn set_fetching(&self, fetching: bool) {
        let imp = self.imp();
        imp.fetching.set(fetching);
        if fetching
            && let Some(error_state) = imp.error_state.borrow().as_ref()
            && error_state.is_visible()
        {
            error_state.set_visible(false);
            if let Some(overlay) = imp.overlay.borrow().as_ref() {
                overlay.set_visible(true);
            }
        }
        if let Some(spinner) = imp.spinner.borrow().as_ref() {
            spinner.set_visible(fetching);
            spinner.set_spinning(fetching);
        }
    }

    pub fn is_fetching(&self) -> bool {
        self.imp().fetching.get()
    }

    pub fn set_polling(&self, polling: bool) {
        self.imp().polling.set(polling);
    }

    pub fn is_polling(&self) -> bool {
        self.imp().polling.get()
    }

    pub fn set_sending(&self, sending: bool) {
        self.imp().sending.set(sending);
        self.update_send_state();
    }

    pub fn is_sending(&self) -> bool {
        self.imp().sending.get()
    }

    pub fn loaded_once(&self) -> bool {
        self.imp().loaded_once.get()
    }

    pub fn set_cursor(&self, cursor: Option<String>) {
        self.imp().cursor.replace(cursor);
    }

    /// The cursor for the next page of older history, if any.
    pub fn cursor(&self) -> Option<String> {
        self.imp().cursor.borrow().clone()
    }

    /// Swap in the error state after a failed fetch, unless rows are
    /// shown. With rows up the list stays; the caller toasts instead.
    pub fn show_load_failed(&self) {
        let imp = self.imp();
        let listed = imp.model.borrow().as_ref().is_some_and(|m| m.n_items() > 0);
        if listed {
            return;
        }
        if let Some(overlay) = imp.overlay.borrow().as_ref() {
            overlay.set_visible(false);
        }
        if let Some(empty_state) = imp.empty_state.borrow().as_ref() {
            empty_state.set_visible(false);
        }
        if let Some(error_state) = imp.error_state.borrow().as_ref() {
            error_state.set_visible(true);
        }
    }

    /// True when no fetch ever completed and none is under way, as after
    /// a failed first load. Reopening such a page fetches again.
    pub fn needs_reload(&self) -> bool {
        !self.imp().loaded_once.get() && !self.imp().fetching.get()
    }

    /// Chain another history fetch when there is a cursor and room to
    /// spare. `get_messages` filters deleted messages, so a short first
    /// page can leave the viewport unfilled with history still behind it.
    pub fn backfill_if_short(&self) {
        let imp = self.imp();
        if imp.fetching.get() || imp.cursor.borrow().is_none() {
            return;
        }
        if self.content_overflows() {
            return;
        }
        if imp.backfill_budget.get() == 0 {
            if imp.model.borrow().as_ref().is_none_or(|m| m.n_items() == 0) {
                if let Some(overlay) = imp.overlay.borrow().as_ref() {
                    overlay.set_visible(false);
                }
                if let Some(empty_state) = imp.empty_state.borrow().as_ref() {
                    empty_state.set_visible(true);
                }
            }
            return;
        }
        imp.backfill_budget.set(imp.backfill_budget.get() - 1);
        if let Some(cb) = imp.load_older_callback.borrow().as_ref() {
            cb();
        }
    }

    /// Whether the listed rows already extend past the viewport.
    fn content_overflows(&self) -> bool {
        self.imp()
            .vadjustment
            .borrow()
            .as_ref()
            .is_some_and(|adj| adj.upper() > adj.page_size() + 1.0)
    }

    /// Hand the composer's draft to the send callback, if it may go.
    fn try_send(&self) {
        let imp = self.imp();
        if imp.sending.get() {
            return;
        }
        let Some(entry) = imp.entry.borrow().clone() else {
            return;
        };
        let text = entry.text().to_string();
        if !message_sendable(&text) {
            return;
        }
        if let Some(cb) = imp.send_callback.borrow().as_ref() {
            cb(text);
        }
    }

    /// The send button follows the draft: something to say, within the
    /// limits, and no send already out.
    fn update_send_state(&self) {
        let imp = self.imp();
        let Some(entry) = imp.entry.borrow().clone() else {
            return;
        };
        let text = entry.text();
        let sendable = message_sendable(&text);
        if !sendable && !text.trim().is_empty() {
            entry.add_css_class("error");
        } else {
            entry.remove_css_class("error");
        }
        if let Some(button) = imp.send_button.borrow().as_ref() {
            button.set_sensitive(sendable && !imp.sending.get());
        }
    }

    /// Replace the handler run when the list nears its top.
    pub fn set_load_older_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .load_older_callback
            .replace(Some(Box::new(callback)));
    }

    /// Replace the handler run by the error state's retry button.
    pub fn set_retry_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp().retry_callback.replace(Some(Box::new(callback)));
    }

    /// Replace the handler given the composer's draft to send.
    pub fn set_send_callback<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp().send_callback.replace(Some(Box::new(callback)));
    }

    /// Replace the handler run when a mention in a message is activated.
    pub fn set_mention_clicked_callback<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp()
            .mention_clicked_callback
            .replace(Some(Box::new(callback)));
    }

    /// Replace the handler run when a shared post's card is activated.
    pub fn set_post_clicked_callback<F: Fn(Post) + 'static>(&self, callback: F) {
        self.imp()
            .post_clicked_callback
            .replace(Some(Box::new(callback)));
    }

    /// Replace the reaction handler: message id, emoji, add or remove.
    pub fn set_react_callback<F: Fn(String, String, bool) + 'static>(&self, callback: F) {
        self.imp().react_callback.replace(Some(Box::new(callback)));
    }

    /// Replace the handler run when Delete for Me is chosen on a message.
    pub fn set_delete_message_callback<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp()
            .delete_message_callback
            .replace(Some(Box::new(callback)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atproto::{ChatReaction, Profile};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn message(id: &str, sender: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            text: text.into(),
            sender_did: sender.into(),
            sent_at: "2026-01-01T12:00:00Z".into(),
            embed: None,
            reactions: vec![],
        }
    }

    fn shared_post(text: &str) -> Embed {
        Embed::Quote(QuoteEmbed {
            uri: "at://did:plc:author/app.bsky.feed.post/shared".into(),
            cid: "cid".into(),
            author: Profile::minimal(
                "did:plc:author".into(),
                "author.bsky.social".into(),
                Some("Author".into()),
                None,
            ),
            text: text.into(),
            indexed_at: "2026-01-01T00:00:00Z".into(),
            embed: None,
        })
    }

    fn conversation() -> Conversation {
        Conversation {
            id: "convo1".into(),
            members: vec![
                Profile::minimal("did:plc:me".into(), "me.bsky.social".into(), None, None),
                Profile::minimal(
                    "did:plc:them".into(),
                    "them.bsky.social".into(),
                    Some("Them".into()),
                    None,
                ),
            ],
            last_message: None,
            unread_count: 0,
            muted: false,
        }
    }

    fn a_page() -> MessagePage {
        MessagePage::new(&conversation(), Some("did:plc:me"))
    }

    fn listed_ids(page: &MessagePage) -> Vec<String> {
        let model = page.imp().model.borrow().clone().unwrap();
        (0..model.n_items())
            .map(|i| {
                model
                    .item(i)
                    .and_downcast::<MessageObject>()
                    .and_then(|o| o.message())
                    .map(|m| m.id)
                    .unwrap()
            })
            .collect()
    }

    /// The composer refuses empty drafts and drafts past the lexicon's
    /// grapheme and byte limits.
    #[test]
    fn drafts_are_bounded_by_the_lexicon_limits() {
        assert!(message_sendable("hello"));
        assert!(!message_sendable(""));
        assert!(!message_sendable("   \n "), "whitespace says nothing");
        assert!(message_sendable(&"a".repeat(1000)));
        assert!(!message_sendable(&"a".repeat(1001)), "grapheme limit");
        // 500 family emoji: well under the grapheme limit, far over the
        // byte limit.
        let heavy = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}".repeat(500);
        assert!(heavy.graphemes(true).count() <= 1000);
        assert!(!message_sendable(&heavy), "byte limit");
    }

    /// Message text is wire text: whatever hostile markup it carries must
    /// come out on screen as written, wrapped so it cannot widen the page.
    #[test]
    fn hostile_message_text_renders_as_written() {
        crate::ui::with_gtk(hostile_message_text_renders_as_written_body);
    }

    fn hostile_message_text_renders_as_written_body() {
        let row = MessageRow::new();
        let column = row.first_child().expect("the column");
        let bubble = column.first_child().expect("the bubble");
        let label = bubble
            .first_child()
            .and_downcast::<gtk4::Label>()
            .expect("the text label");

        for text in [
            "<b>bold</b> & <script>alert(1)</script>",
            "&amp;",
            "a & b < c > d",
            "&&&###@@@",
        ] {
            row.bind(
                &message("m1", "did:plc:them", text),
                false,
                Some("did:plc:me"),
            );
            assert_eq!(
                label.text().as_str(),
                text,
                "the label must show the message as written"
            );
        }

        assert!(label.wraps());
        assert_eq!(
            label.wrap_mode(),
            gtk4::pango::WrapMode::WordChar,
            "an unbroken run must wrap rather than widen the page"
        );
    }

    /// Profile links route in-app like mentions; anything deeper in the
    /// path is a post or list and stays a plain link. A message that is
    /// just a GIF link is recognised for the player.
    #[test]
    fn profile_and_gif_links_are_recognized() {
        assert_eq!(
            bsky_profile_target("https://bsky.app/profile/them.bsky.social").as_deref(),
            Some("them.bsky.social")
        );
        assert_eq!(
            bsky_profile_target("https://bsky.app/profile/did:plc:abc123").as_deref(),
            Some("did:plc:abc123")
        );
        assert_eq!(
            bsky_profile_target("https://bsky.app/profile/them.bsky.social?tab=media").as_deref(),
            Some("them.bsky.social")
        );
        assert!(
            bsky_profile_target("https://bsky.app/profile/them.bsky.social/post/3abc").is_none(),
            "post links stay links"
        );
        assert!(bsky_profile_target("https://example.com/profile/x").is_none());
        assert!(bsky_profile_target("https://bsky.app/profile/").is_none());

        let klipy = "https://static.klipy.com/ii/abc/1/2/deadbeef.gif?hh=270&ww=480&mp4=deadbeef";
        let (link, gif) = lone_gif_link(&format!("  {klipy} ")).expect("a lone GIF link");
        assert_eq!(link, klipy);
        assert_eq!(gif.aspect_ratio, (480, 270));
        assert!(
            lone_gif_link(&format!("look at this {klipy}")).is_none(),
            "words around the link keep it a link"
        );
        assert!(lone_gif_link("https://example.com/a.gif").is_none());
    }

    /// The clock setting decides how message times read.
    #[test]
    fn times_follow_the_clock_format() {
        use chrono::TimeZone;
        let local = chrono::Local
            .with_ymd_and_hms(2026, 1, 1, 15, 7, 0)
            .single()
            .unwrap();

        let (short, full) = format_local_time(local, false);
        assert_eq!(short, "Jan 1, 15:07");
        assert_eq!(full, "January 1, 2026 at 15:07");

        let (short, full) = format_local_time(local, true);
        assert_eq!(short, "Jan 1, 3:07 PM");
        assert_eq!(full, "January 1, 2026 at 3:07 PM");
    }

    /// Reaction chips group by emoji, mark the viewer's own, and clicking
    /// toggles: yours comes off, anyone else's joins in. A recycled row
    /// drops them with the message.
    #[test]
    fn reaction_chips_group_and_toggle() {
        crate::ui::with_gtk(reaction_chips_group_and_toggle_body);
    }

    fn reaction_chips_group_and_toggle_body() {
        let row = MessageRow::new();
        let asked: Rc<RefCell<Vec<(String, String, bool)>>> = Rc::new(RefCell::new(vec![]));
        let sink = asked.clone();
        row.set_react_callback(move |id, value, add| {
            sink.borrow_mut().push((id, value, add));
        });

        let mut reacted = message("m1", "did:plc:them", "hey");
        reacted.reactions = vec![
            ChatReaction {
                value: "\u{1F44D}".into(),
                sender_did: "did:plc:me".into(),
            },
            ChatReaction {
                value: "\u{1F44D}".into(),
                sender_did: "did:plc:them".into(),
            },
            ChatReaction {
                value: "\u{2764}".into(),
                sender_did: "did:plc:them".into(),
            },
        ];
        row.bind(&reacted, false, Some("did:plc:me"));

        let strip = row.imp().reactions_box.borrow().clone().unwrap();
        assert!(strip.is_visible());
        assert_eq!(
            strip.halign(),
            gtk4::Align::Start,
            "their message wears its reactions on its own side"
        );

        let mut chips = Vec::new();
        let mut child = strip.first_child();
        while let Some(c) = child {
            chips.push(c.clone().downcast::<gtk4::Button>().unwrap());
            child = c.next_sibling();
        }
        assert_eq!(chips.len(), 2, "one chip per distinct emoji");
        assert_eq!(chips[0].label().as_deref(), Some("\u{1F44D} 2"));
        assert!(chips[0].has_css_class("reaction-chip-mine"));
        assert_eq!(chips[1].label().as_deref(), Some("\u{2764} 1"));
        assert!(!chips[1].has_css_class("reaction-chip-mine"));

        chips[0].emit_clicked();
        chips[1].emit_clicked();
        assert_eq!(
            asked.borrow().as_slice(),
            [
                ("m1".to_string(), "\u{1F44D}".to_string(), false),
                ("m1".to_string(), "\u{2764}".to_string(), true),
            ],
            "your own chip removes, another's adds"
        );

        row.bind(
            &message("m2", "did:plc:them", "plain"),
            false,
            Some("did:plc:me"),
        );
        assert!(!strip.is_visible());
        assert!(strip.first_child().is_none());

        // The double-click shortcut hearts whatever is bound now.
        row.react_heart();
        assert_eq!(
            asked.borrow().last(),
            Some(&("m2".to_string(), "\u{2764}\u{FE0F}".to_string(), true)),
            "a double click asks to add the heart"
        );
    }

    /// The context menu serves the bound message: Delete for Me hands the
    /// current id over, not the one from a previous bind.
    #[test]
    fn the_menu_serves_the_current_message() {
        crate::ui::with_gtk(the_menu_serves_the_current_message_body);
    }

    fn the_menu_serves_the_current_message_body() {
        let row = MessageRow::new();
        let deleted: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let sink = deleted.clone();
        row.set_delete_callback(move |id| {
            sink.borrow_mut().push(id);
        });

        let menu = row.imp().context_menu.borrow().clone().unwrap();
        let menu_box = menu.child().unwrap();
        let mut items = Vec::new();
        let mut child = menu_box.first_child();
        while let Some(c) = child {
            items.push(c.clone().downcast::<gtk4::Button>().unwrap());
            child = c.next_sibling();
        }
        let labels: Vec<_> = items.iter().filter_map(|b| b.label()).collect();
        assert_eq!(labels, ["React", "Copy Text", "Delete for Me"]);

        row.bind(&message("m1", "did:plc:them", "first"), false, None);
        row.bind(&message("m2", "did:plc:them", "second"), false, None);
        items[2].emit_clicked();
        assert_eq!(
            deleted.borrow().as_slice(),
            ["m2"],
            "the menu acts on the current occupant"
        );
    }

    /// Poll pages carry reaction changes for messages already listed;
    /// merge_new applies them without duplicating rows. A removed message
    /// stays known, so a straggling poll cannot bring it back.
    #[test]
    fn merge_new_updates_reactions_and_removal_sticks() {
        crate::ui::with_gtk(merge_new_updates_reactions_and_removal_sticks_body);
    }

    fn merge_new_updates_reactions_and_removal_sticks_body() {
        let page = a_page();
        page.set_initial_messages(vec![
            message("m2", "did:plc:them", "two"),
            message("m1", "did:plc:them", "one"),
        ]);
        assert_eq!(listed_ids(&page), ["m1", "m2"]);

        let mut updated = message("m1", "did:plc:them", "one");
        updated.reactions = vec![ChatReaction {
            value: "\u{1F44D}".into(),
            sender_did: "did:plc:them".into(),
        }];
        let added = page.merge_new(vec![updated]);
        assert_eq!(added, 0, "a reaction change is not a new message");
        assert_eq!(listed_ids(&page), ["m1", "m2"]);
        let model = page.imp().model.borrow().clone().unwrap();
        let m1 = model
            .item(0)
            .and_downcast::<MessageObject>()
            .and_then(|o| o.message())
            .unwrap();
        assert_eq!(
            m1.reactions.len(),
            1,
            "the listed copy carries the reaction"
        );

        page.remove_message("m1");
        assert_eq!(listed_ids(&page), ["m2"]);
        let added = page.merge_new(vec![message("m1", "did:plc:them", "one")]);
        assert_eq!(added, 0);
        assert_eq!(
            listed_ids(&page),
            ["m2"],
            "a deleted message must not come back on the next poll"
        );
    }

    /// Own messages sit right in an outgoing bubble, everyone else's sit
    /// left, and a recycled row swaps sides cleanly.
    #[test]
    fn messages_take_their_sides_and_rebind_cleanly() {
        crate::ui::with_gtk(messages_take_their_sides_and_rebind_cleanly_body);
    }

    fn messages_take_their_sides_and_rebind_cleanly_body() {
        let row = MessageRow::new();
        let column = row.first_child().expect("the column");
        let bubble = column.first_child().expect("the bubble");

        // Without hexpand the column only ever gets its natural width and
        // halign has nothing to place it in; every message sat left.
        assert!(
            column.hexpands(),
            "halign cannot take a side unless the column takes the row"
        );

        row.bind(
            &message("m1", "did:plc:me", "mine"),
            true,
            Some("did:plc:me"),
        );
        assert_eq!(column.halign(), gtk4::Align::End);
        assert!(bubble.has_css_class("msg-out"));
        assert!(!bubble.has_css_class("msg-in"));

        row.bind(
            &message("m2", "did:plc:them", "theirs"),
            false,
            Some("did:plc:me"),
        );
        assert_eq!(column.halign(), gtk4::Align::Start);
        assert!(bubble.has_css_class("msg-in"));
        assert!(
            !bubble.has_css_class("msg-out"),
            "a recycled row must shed the other side's styling"
        );
    }

    /// A shared post renders as a card that opens the post once, and a
    /// recycled row drops the card with the message that owned it.
    #[test]
    fn a_shared_post_card_opens_its_post_and_leaves_with_it() {
        crate::ui::with_gtk(a_shared_post_card_opens_its_post_and_leaves_with_it_body);
    }

    fn a_shared_post_card_opens_its_post_and_leaves_with_it_body() {
        let row = MessageRow::new();
        let opened: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let sink = opened.clone();
        row.set_post_callback(move |post| {
            sink.borrow_mut().push(post.uri);
        });

        let mut shared = message("m1", "did:plc:them", "");
        shared.embed = Some(shared_post("the shared post"));
        row.bind(&shared, false, Some("did:plc:me"));

        let column = row.first_child().expect("the column");
        let bubble = column.first_child().expect("the bubble");
        let text_label = bubble
            .first_child()
            .and_downcast::<gtk4::Label>()
            .expect("the text label");
        assert!(
            !text_label.is_visible(),
            "a share without a comment shows no empty text line"
        );

        let slot = text_label.next_sibling().expect("the embed slot");
        let card = slot
            .first_child()
            .and_downcast::<gtk4::Button>()
            .expect("the shared post renders a card");

        let mut texts = Vec::new();
        collect_labels(&card.clone().upcast(), &mut texts);
        assert!(texts.iter().any(|t| t == "Author"));
        assert!(texts.iter().any(|t| t == "@author.bsky.social"));
        assert!(texts.iter().any(|t| t == "the shared post"));

        card.emit_clicked();
        assert_eq!(
            opened.borrow().as_slice(),
            ["at://did:plc:author/app.bsky.feed.post/shared"],
            "one click opens the post once"
        );

        // Rebind to a plain message: text back, card gone.
        row.bind(
            &message("m2", "did:plc:them", "just words"),
            false,
            Some("did:plc:me"),
        );
        assert!(text_label.is_visible());
        assert!(
            slot.first_child().is_none(),
            "a recycled row must not keep the previous message's card"
        );
    }

    fn collect_labels(widget: &gtk4::Widget, out: &mut Vec<String>) {
        if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
            out.push(label.text().to_string());
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            collect_labels(&c, out);
            child = c.next_sibling();
        }
    }

    /// The wire hands pages newest first; the screen reads oldest first.
    /// Older pages go in at the top, and overlap dedupes by id.
    #[test]
    fn history_displays_oldest_first_and_older_pages_go_on_top() {
        crate::ui::with_gtk(history_displays_oldest_first_and_older_pages_go_on_top_body);
    }

    fn history_displays_oldest_first_and_older_pages_go_on_top_body() {
        let page = a_page();
        page.set_cursor(Some("older".into()));
        page.set_initial_messages(vec![
            message("c", "did:plc:them", "third"),
            message("b", "did:plc:me", "second"),
            message("a", "did:plc:them", "first"),
        ]);
        assert_eq!(listed_ids(&page), vec!["a", "b", "c"]);

        // The older page overlaps at "a", as cursor pages can.
        page.set_cursor(None);
        page.prepend_older(vec![
            message("a", "did:plc:them", "first"),
            message("z", "did:plc:me", "zeroth"),
            message("y", "did:plc:them", "minus one"),
        ]);
        assert_eq!(listed_ids(&page), vec!["y", "z", "a", "b", "c"]);
    }

    /// A polled page folds in by id: only unseen messages append, in
    /// order, and the count tells the caller whether there is anything to
    /// mark read. An unchanged page merges to zero.
    #[test]
    fn polls_merge_only_unseen_messages() {
        crate::ui::with_gtk(polls_merge_only_unseen_messages_body);
    }

    fn polls_merge_only_unseen_messages_body() {
        let page = a_page();
        page.set_initial_messages(vec![
            message("b", "did:plc:me", "second"),
            message("a", "did:plc:them", "first"),
        ]);

        let added = page.merge_new(vec![
            message("d", "did:plc:them", "fourth"),
            message("c", "did:plc:them", "third"),
            message("b", "did:plc:me", "second"),
            message("a", "did:plc:them", "first"),
        ]);
        assert_eq!(added, 2);
        assert_eq!(listed_ids(&page), vec!["a", "b", "c", "d"]);

        let added = page.merge_new(vec![
            message("d", "did:plc:them", "fourth"),
            message("c", "did:plc:them", "third"),
        ]);
        assert_eq!(added, 0, "an unchanged page leaves nothing to mark read");
        assert_eq!(listed_ids(&page), vec!["a", "b", "c", "d"]);
    }

    /// The empty state waits for the first load, and the first sent
    /// message stands it down again.
    #[test]
    fn the_empty_state_waits_for_the_first_load_and_yields_to_a_send() {
        crate::ui::with_gtk(the_empty_state_waits_for_the_first_load_and_yields_to_a_send_body);
    }

    fn the_empty_state_waits_for_the_first_load_and_yields_to_a_send_body() {
        let page = a_page();
        let imp = page.imp();

        assert!(!imp.empty_state.borrow().as_ref().unwrap().is_visible());

        page.set_fetching(true);
        assert!(imp.spinner.borrow().as_ref().unwrap().is_visible());
        page.set_initial_messages(vec![]);
        page.set_fetching(false);

        assert!(imp.empty_state.borrow().as_ref().unwrap().is_visible());
        assert!(!imp.overlay.borrow().as_ref().unwrap().is_visible());
        assert!(
            imp.empty_state
                .borrow()
                .as_ref()
                .unwrap()
                .description()
                .unwrap()
                .contains("them.bsky.social"),
            "the empty state names the other member"
        );

        // The first message in a fresh conversation flips it to a list.
        imp.entry.borrow().as_ref().unwrap().set_text("hello");
        page.sent_ok(message("s1", "did:plc:me", "hello"));
        assert!(!imp.empty_state.borrow().as_ref().unwrap().is_visible());
        assert!(imp.overlay.borrow().as_ref().unwrap().is_visible());
        assert_eq!(listed_ids(&page), vec!["s1"]);
        assert_eq!(
            imp.entry.borrow().as_ref().unwrap().text().as_str(),
            "",
            "a delivered draft clears"
        );
    }

    /// One send at a time, and a failed send keeps the draft for another
    /// try instead of eating it.
    #[test]
    fn sending_gates_the_composer_and_a_failure_keeps_the_draft() {
        crate::ui::with_gtk(sending_gates_the_composer_and_a_failure_keeps_the_draft_body);
    }

    fn sending_gates_the_composer_and_a_failure_keeps_the_draft_body() {
        let page = a_page();
        let imp = page.imp();
        let entry = imp.entry.borrow().clone().unwrap();
        let button = imp.send_button.borrow().clone().unwrap();

        let sent: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let sink = sent.clone();
        page.set_send_callback(move |text| {
            sink.borrow_mut().push(text);
        });

        assert!(!button.is_sensitive(), "nothing to send yet");
        page.try_send();
        assert!(sent.borrow().is_empty());

        entry.set_text("hi there");
        assert!(button.is_sensitive());
        page.try_send();
        assert_eq!(*sent.borrow(), vec!["hi there".to_string()]);

        // In flight: the button drops and repeat sends are refused.
        page.set_sending(true);
        assert!(!button.is_sensitive());
        page.try_send();
        assert_eq!(sent.borrow().len(), 1, "one send at a time");

        // Failure: the draft survives and the composer re-arms.
        page.send_failed();
        assert_eq!(entry.text().as_str(), "hi there");
        assert!(button.is_sensitive());

        // Overlong drafts are refused before they go anywhere.
        entry.set_text(&"a".repeat(1001));
        assert!(!button.is_sensitive());
        page.try_send();
        assert_eq!(sent.borrow().len(), 1);
        assert!(entry.has_css_class("error"), "the draft is flagged");
        entry.set_text("short again");
        assert!(!entry.has_css_class("error"));
        assert!(button.is_sensitive());
    }

    /// An older page that is all deleted messages chains the next fetch,
    /// and the budget keeps a hostile server from looping forever.
    #[test]
    fn short_history_pages_chain_fetches_up_to_the_budget() {
        crate::ui::with_gtk(short_history_pages_chain_fetches_up_to_the_budget_body);
    }

    fn short_history_pages_chain_fetches_up_to_the_budget_body() {
        use std::cell::Cell;

        let page = a_page();
        let fetches = Rc::new(Cell::new(0u32));
        let counter = fetches.clone();
        let page_weak = page.downgrade();
        page.set_load_older_callback(move || {
            counter.set(counter.get() + 1);
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            page.set_cursor(Some(format!("page-{}", counter.get() + 2)));
            page.prepend_older(vec![]);
            page.backfill_if_short();
        });

        page.set_cursor(Some("page-2".into()));
        page.set_initial_messages(vec![]);
        assert!(
            !page
                .imp()
                .empty_state
                .borrow()
                .as_ref()
                .unwrap()
                .is_visible(),
            "a stored cursor means there may be history behind the page"
        );
        page.backfill_if_short();

        assert_eq!(fetches.get(), u32::from(super::BACKFILL_CAP));
        assert!(
            page.imp()
                .empty_state
                .borrow()
                .as_ref()
                .unwrap()
                .is_visible(),
            "with the budget spent and nothing listed, the page reads empty"
        );
    }
}
