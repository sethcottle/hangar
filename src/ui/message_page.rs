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

/// Whether the composer's draft may go on the wire.
fn message_sendable(text: &str) -> bool {
    !text.trim().is_empty()
        && text.len() <= MAX_MESSAGE_BYTES
        && text.graphemes(true).count() <= MAX_MESSAGE_GRAPHEMES
}

/// Short label text and full tooltip text for a message timestamp.
fn format_message_time(sent_at: &str) -> (String, String) {
    use chrono::{DateTime, Local};

    let Ok(time) = DateTime::parse_from_rfc3339(sent_at) else {
        return (String::new(), String::new());
    };
    let local = time.with_timezone(&Local);
    let short = if local.date_naive() == Local::now().date_naive() {
        local.format("%H:%M").to_string()
    } else {
        local.format("%b %-d, %H:%M").to_string()
    };
    (short, local.format("%B %-d, %Y at %H:%M").to_string())
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
            pub bubble: RefCell<Option<gtk4::Box>>,
            pub text_label: RefCell<Option<gtk4::Label>>,
            pub embed_slot: RefCell<Option<gtk4::Box>>,
            pub time_label: RefCell<Option<gtk4::Label>>,
            pub mention_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
            pub post_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
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

            // Without hexpand the row gives the bubble its natural width
            // and the halign flip in bind never moves anything; every
            // message sat left. With it, halign places the bubble and the
            // background still hugs the content.
            let bubble = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            bubble.add_css_class("msg-bubble");
            bubble.set_hexpand(true);

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

            self.append(&bubble);

            let imp = self.imp();
            imp.bubble.replace(Some(bubble));
            imp.text_label.replace(Some(text_label));
            imp.embed_slot.replace(Some(embed_slot));
            imp.time_label.replace(Some(time_label));
        }

        /// Fill the row for one message. Recycled rows pass through here
        /// again, so every setter overwrites what the last message left.
        pub fn bind(&self, message: &ChatMessage, mine: bool) {
            let imp = self.imp();

            if let Some(bubble) = imp.bubble.borrow().as_ref() {
                if mine {
                    bubble.add_css_class("msg-out");
                    bubble.remove_css_class("msg-in");
                    bubble.set_halign(gtk4::Align::End);
                } else {
                    bubble.add_css_class("msg-in");
                    bubble.remove_css_class("msg-out");
                    bubble.set_halign(gtk4::Align::Start);
                }
                let (_, full) = format_message_time(&message.sent_at);
                bubble.set_tooltip_text(if full.is_empty() { None } else { Some(&full) });
            }

            if let Some(label) = imp.text_label.borrow().as_ref() {
                crate::ui::rich_text::set_linkified(label, &message.text);
                // A post shared without a comment has no text of its own.
                label.set_visible(!message.text.is_empty());
            }

            if let Some(slot) = imp.embed_slot.borrow().as_ref() {
                while let Some(child) = slot.first_child() {
                    slot.remove(&child);
                }
                if let Some(Embed::Quote(quote)) = &message.embed {
                    slot.append(&self.shared_post_card(quote));
                }
            }

            if let Some(label) = imp.time_label.borrow().as_ref() {
                let (short, _) = format_message_time(&message.sent_at);
                label.set_text(&short);
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
                let mine =
                    page.imp().my_did.borrow().as_deref() == Some(message.sender_did.as_str());
                row.bind(&message, mine);
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
        let fresh: Vec<ChatMessage> = {
            let known = imp.known_ids.borrow();
            newest_first
                .into_iter()
                .filter(|m| !known.contains(&m.id))
                .collect()
        };
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atproto::Profile;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn message(id: &str, sender: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            text: text.into(),
            sender_did: sender.into(),
            sent_at: "2026-01-01T12:00:00Z".into(),
            embed: None,
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
        let bubble = row.first_child().expect("the bubble");
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
            row.bind(&message("m1", "did:plc:them", text), false);
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

    /// Own messages sit right in an outgoing bubble, everyone else's sit
    /// left, and a recycled row swaps sides cleanly.
    #[test]
    fn messages_take_their_sides_and_rebind_cleanly() {
        crate::ui::with_gtk(messages_take_their_sides_and_rebind_cleanly_body);
    }

    fn messages_take_their_sides_and_rebind_cleanly_body() {
        let row = MessageRow::new();
        let bubble = row.first_child().expect("the bubble");

        // Without hexpand the bubble only ever gets its natural width and
        // halign has nothing to place it in; every message sat left.
        assert!(
            bubble.hexpands(),
            "halign cannot take a side unless the bubble takes the row"
        );

        row.bind(&message("m1", "did:plc:me", "mine"), true);
        assert_eq!(bubble.halign(), gtk4::Align::End);
        assert!(bubble.has_css_class("msg-out"));
        assert!(!bubble.has_css_class("msg-in"));

        row.bind(&message("m2", "did:plc:them", "theirs"), false);
        assert_eq!(bubble.halign(), gtk4::Align::Start);
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
        row.bind(&shared, false);

        let bubble = row.first_child().expect("the bubble");
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
        row.bind(&message("m2", "did:plc:them", "just words"), false);
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
