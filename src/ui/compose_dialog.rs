// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]

use crate::atproto::Profile;
use crate::ui::avatar_cache;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;
use regex::Regex;
use std::cell::Cell;
use std::sync::LazyLock;
use unicode_segmentation::UnicodeSegmentation;

/// Bluesky post character limit (grapheme clusters)
const MAX_GRAPHEMES: i32 = 300;
/// Show warning color when this many characters remain
const WARN_THRESHOLD: i32 = 20;

/// Tag names for rich text highlighting in the compose buffer
const TAG_MENTION: &str = "mention";
const TAG_HASHTAG: &str = "hashtag";
const TAG_URL: &str = "url";
const TAG_EMOJI: &str = "emoji";

// Reuse the same regex patterns as facets.rs for consistent detection
static HL_URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://[^\s<>\[\]\{}|\\^`\x00-\x1f\x7f]+").unwrap());

static HL_MENTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s\(\[])(@(([a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z]([a-zA-Z0-9-]*[a-zA-Z0-9])?))")
        .unwrap()
});

static HL_HASHTAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s\(\[])(#[a-zA-Z][a-zA-Z0-9_]*)").unwrap());

/// Check if a character is an emoji (or emoji component) that renders taller than text.
fn is_emoji(ch: char) -> bool {
    let cp = ch as u32;
    matches!(cp,
        // Zero-width joiner (emoji sequences like 👨‍👩‍👧‍👦)
        0x200D |
        // Misc symbols preceding emoji range
        0x203C | 0x2049 | 0x2122 | 0x2139 |
        0x2194..=0x2199 |
        0x21A9..=0x21AA |
        // Miscellaneous Technical (⌚ ⏰ etc.)
        0x2300..=0x23FF |
        0x25AA..=0x25AB | 0x25B6 | 0x25C0 | 0x25FB..=0x25FE |
        // Miscellaneous Symbols & Dingbats (☀️–✿)
        0x2600..=0x27BF |
        0x2934..=0x2935 |
        0x2B05..=0x2B07 | 0x2B1B..=0x2B1C | 0x2B50..=0x2B55 |
        0x3030 | 0x303D | 0x3297 | 0x3299 |
        // Variation selectors (emoji presentation)
        0xFE00..=0xFE0F |
        // Regional Indicator Symbols (flags)
        0x1F1E0..=0x1F1FF |
        // All main emoji blocks (emoticons, transport, symbols, etc.)
        0x1F300..=0x1F9FF |
        // Symbols and Pictographs Extended-A & B
        0x1FA00..=0x1FAFF
    )
}

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
        pub char_counter: RefCell<Option<gtk4::Label>>,
        // Mention autocomplete
        pub mention_popover: RefCell<Option<gtk4::Popover>>,
        pub mention_list: RefCell<Option<gtk4::ListBox>>,
        pub mention_results: RefCell<Vec<Profile>>,
        pub mention_search_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
        pub debounce_counter: Cell<u32>,
        /// Tracks the char offset of the '@' that triggered the current autocomplete
        pub mention_at_offset: Cell<i32>,
        /// Guard to prevent recursive buffer change notifications during highlighting
        pub highlighting: Cell<bool>,
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

        fn dispose(&self) {
            // Popover was manually parented to the TextView via set_parent(),
            // so we MUST unparent it before the widget tree is torn down.
            // Otherwise GTK spams "GtkPopover is not a child of GtkTextView"
            // warnings and the UI freezes.
            if let Some(popover) = self.mention_popover.borrow_mut().take() {
                popover.unparent();
            }
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
                format!(
                    "{}...",
                    &context.text[..context.text.floor_char_boundary(100)]
                )
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
        // Header bar with Cancel (start) and Post (end) — GNOME HIG pattern
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
        post_btn.set_sensitive(false); // Disabled until text is entered
        header.pack_end(&post_btn);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(12);
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
        text_view.set_accepts_tab(false);
        text_view.update_property(&[gtk4::accessible::Property::Label("Post content")]);

        // Create text tags for rich text highlighting (mentions, hashtags, URLs).
        // Colors are applied once the widget is realized so we can read the theme accent.
        let buffer = text_view.buffer();
        let tag_table = buffer.tag_table();

        // Determine accent color for facet highlighting.
        // Use Adw.StyleManager to pick the right blue for the current color scheme,
        // with a reliable fallback to standard GNOME blue.
        let accent = {
            let style_manager = adw::StyleManager::default();
            let is_dark = style_manager.is_dark();
            if is_dark {
                // Adwaita dark accent blue
                gdk::RGBA::new(0.47, 0.68, 0.93, 1.0) // #78aeed
            } else {
                // Adwaita light accent blue
                gdk::RGBA::new(0.21, 0.52, 0.89, 1.0) // #3584e4
            }
        };

        // All facet tags use accent color only — no extra weight, consistent with
        // surrounding text. URLs additionally get an underline per WCAG 1.4.1.
        let mention_tag = gtk4::TextTag::new(Some(TAG_MENTION));
        mention_tag.set_foreground_rgba(Some(&accent));
        tag_table.add(&mention_tag);

        let hashtag_tag = gtk4::TextTag::new(Some(TAG_HASHTAG));
        hashtag_tag.set_foreground_rgba(Some(&accent));
        tag_table.add(&hashtag_tag);

        let url_tag = gtk4::TextTag::new(Some(TAG_URL));
        url_tag.set_foreground_rgba(Some(&accent));
        url_tag.set_underline(gtk4::pango::Underline::Single);
        tag_table.add(&url_tag);

        // Emoji tag: increase line height on lines containing emoji so they
        // don't overlap with adjacent text. Only applied to emoji characters.
        let emoji_tag = gtk4::TextTag::new(Some(TAG_EMOJI));
        emoji_tag.set_line_height(1.5);
        tag_table.add(&emoji_tag);

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_min_content_height(120);
        scrolled.set_child(Some(&text_view));
        content.append(&scrolled);

        // --- Mention autocomplete popover ---
        let mention_list = gtk4::ListBox::new();
        mention_list.set_selection_mode(gtk4::SelectionMode::Single);
        mention_list.add_css_class("mention-list");
        mention_list.update_property(&[gtk4::accessible::Property::Label("Mention suggestions")]);

        let mention_scroll = gtk4::ScrolledWindow::new();
        mention_scroll.set_child(Some(&mention_list));
        mention_scroll.set_max_content_height(250);
        mention_scroll.set_propagate_natural_height(true);

        let mention_popover = gtk4::Popover::new();
        mention_popover.set_child(Some(&mention_scroll));
        mention_popover.set_parent(&text_view);
        mention_popover.set_has_arrow(true);
        mention_popover.set_autohide(false);
        mention_popover.add_css_class("mention-popover");
        mention_popover.set_position(gtk4::PositionType::Bottom);
        mention_popover.set_size_request(320, -1);

        // Connect row activation (click) to insert mention
        let dialog_weak = self.downgrade();
        mention_list.connect_row_activated(move |_, row| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.insert_mention(row.index() as usize);
            }
        });

        // --- Key controller for mention navigation ---
        let key_controller = gtk4::EventControllerKey::new();
        let dialog_weak = self.downgrade();
        key_controller.connect_key_pressed(move |_, keyval, _, _| {
            let Some(dialog) = dialog_weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let imp = dialog.imp();

            // Only intercept keys when popover is visible
            let popover_visible = imp
                .mention_popover
                .borrow()
                .as_ref()
                .is_some_and(|p| p.is_visible());

            if !popover_visible {
                return glib::Propagation::Proceed;
            }

            match keyval {
                gdk::Key::Down => {
                    dialog.move_mention_selection(1);
                    glib::Propagation::Stop
                }
                gdk::Key::Up => {
                    dialog.move_mention_selection(-1);
                    glib::Propagation::Stop
                }
                gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                    // Tab cycles down, Shift+Tab cycles up
                    if keyval == gdk::Key::ISO_Left_Tab {
                        dialog.move_mention_selection(-1);
                    } else {
                        dialog.move_mention_selection(1);
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Return => {
                    if let Some(list) = imp.mention_list.borrow().as_ref() {
                        if let Some(row) = list.selected_row() {
                            dialog.insert_mention(row.index() as usize);
                            return glib::Propagation::Stop;
                        }
                    }
                    glib::Propagation::Proceed
                }
                gdk::Key::Escape => {
                    dialog.hide_mention_popover();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        text_view.add_controller(key_controller);

        // --- Buffer change handler for @ detection ---
        let buffer = text_view.buffer();
        let dialog_weak = self.downgrade();
        buffer.connect_changed(move |buf| {
            let Some(dialog) = dialog_weak.upgrade() else {
                return;
            };
            dialog.on_text_changed(buf);
        });

        // Character counter — right-aligned below the text area
        let char_counter = gtk4::Label::new(Some(&MAX_GRAPHEMES.to_string()));
        char_counter.set_halign(gtk4::Align::End);
        char_counter.add_css_class("dim-label");
        char_counter.add_css_class("caption");
        char_counter.add_css_class("char-counter");
        char_counter.set_tooltip_text(Some("Characters remaining"));
        char_counter.update_property(&[gtk4::accessible::Property::Label(
            "300 characters remaining",
        )]);
        content.append(&char_counter);

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

        toolbar.set_content(Some(&content));

        self.set_child(Some(&toolbar));

        let imp = self.imp();
        imp.text_view.replace(Some(text_view));
        imp.post_button.replace(Some(post_btn));
        imp.error_label.replace(Some(error_label));
        imp.reply_label.replace(Some(reply_label));
        imp.quote_preview.replace(Some(quote_preview));
        imp.mention_popover.replace(Some(mention_popover));
        imp.mention_list.replace(Some(mention_list));
        imp.char_counter.replace(Some(char_counter));

        let dialog_weak = self.downgrade();
        if let Some(btn) = imp.post_button.borrow().as_ref() {
            btn.connect_clicked(move |_| {
                if let Some(dialog) = dialog_weak.upgrade() {
                    dialog.emit_post();
                }
            });
        }
    }

    /// Update the character counter label, Post button state, and styling.
    fn update_char_counter(&self, buffer: &gtk4::TextBuffer) {
        let imp = self.imp();
        let text = {
            let start = buffer.start_iter();
            let end = buffer.end_iter();
            buffer.text(&start, &end, false).to_string()
        };

        let grapheme_count = text.graphemes(true).count() as i32;
        let remaining = MAX_GRAPHEMES - grapheme_count;

        if let Some(counter) = imp.char_counter.borrow().as_ref() {
            counter.set_text(&remaining.to_string());

            // Update accessible label
            let a11y = if remaining >= 0 {
                format!("{} characters remaining", remaining)
            } else {
                format!("{} characters over limit", -remaining)
            };
            counter.update_property(&[gtk4::accessible::Property::Label(&a11y)]);

            // Style: red when warning or over
            if remaining < 0 {
                counter.remove_css_class("dim-label");
                counter.remove_css_class("char-counter-warn");
                counter.add_css_class("char-counter-over");
            } else if remaining <= WARN_THRESHOLD {
                counter.remove_css_class("dim-label");
                counter.remove_css_class("char-counter-over");
                counter.add_css_class("char-counter-warn");
            } else {
                counter.remove_css_class("char-counter-warn");
                counter.remove_css_class("char-counter-over");
                counter.add_css_class("dim-label");
            }
        }

        // Disable Post button when over limit or empty
        if let Some(btn) = imp.post_button.borrow().as_ref() {
            btn.set_sensitive(grapheme_count > 0 && remaining >= 0);
        }
    }

    /// Apply syntax highlighting for mentions, hashtags, and URLs in the buffer.
    fn highlight_facets(&self, buffer: &gtk4::TextBuffer) {
        let text = {
            let start = buffer.start_iter();
            let end = buffer.end_iter();
            buffer.text(&start, &end, false).to_string()
        };

        // Remove all existing facet tags first
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        buffer.remove_tag_by_name(TAG_MENTION, &start, &end);
        buffer.remove_tag_by_name(TAG_HASHTAG, &start, &end);
        buffer.remove_tag_by_name(TAG_URL, &start, &end);
        buffer.remove_tag_by_name(TAG_EMOJI, &start, &end);

        // Pre-compute byte→char offset mapping for the text.
        // char_offsets[byte_index] = char_index for each char boundary.
        let byte_to_char: Vec<i32> = {
            let mut map = vec![0i32; text.len() + 1];
            let mut char_idx = 0i32;
            for (byte_idx, _) in text.char_indices() {
                map[byte_idx] = char_idx;
                char_idx += 1;
            }
            map[text.len()] = char_idx; // sentinel for end
            map
        };

        // Apply URL tags
        for m in HL_URL_RE.find_iter(&text) {
            let url = m
                .as_str()
                .trim_end_matches(|c| matches!(c, '.' | ',' | ';' | '!' | '?'));
            let byte_end = m.start() + url.len();
            let char_start = byte_to_char[m.start()];
            let char_end = byte_to_char[byte_end];
            let s = buffer.iter_at_offset(char_start);
            let e = buffer.iter_at_offset(char_end);
            buffer.apply_tag_by_name(TAG_URL, &s, &e);
        }

        // Apply mention tags (capture group 1 = @handle)
        for caps in HL_MENTION_RE.captures_iter(&text) {
            if let Some(m) = caps.get(1) {
                let char_start = byte_to_char[m.start()];
                let char_end = byte_to_char[m.end()];
                let s = buffer.iter_at_offset(char_start);
                let e = buffer.iter_at_offset(char_end);
                buffer.apply_tag_by_name(TAG_MENTION, &s, &e);
            }
        }

        // Apply hashtag tags (capture group 1 = #tag)
        for caps in HL_HASHTAG_RE.captures_iter(&text) {
            if let Some(m) = caps.get(1) {
                let char_start = byte_to_char[m.start()];
                let char_end = byte_to_char[m.end()];
                let s = buffer.iter_at_offset(char_start);
                let e = buffer.iter_at_offset(char_end);
                buffer.apply_tag_by_name(TAG_HASHTAG, &s, &e);
            }
        }

        // Apply emoji line-height tag to any emoji characters so they don't
        // overlap adjacent lines. We detect emoji by Unicode category.
        let mut char_offset = 0i32;
        for ch in text.chars() {
            if is_emoji(ch) {
                let s = buffer.iter_at_offset(char_offset);
                let e = buffer.iter_at_offset(char_offset + 1);
                buffer.apply_tag_by_name(TAG_EMOJI, &s, &e);
            }
            char_offset += 1;
        }
    }

    /// Detect @mention context from current cursor position and update char counter.
    fn on_text_changed(&self, buffer: &gtk4::TextBuffer) {
        let imp = self.imp();

        // Guard against re-entrant calls from tag application
        if imp.highlighting.get() {
            return;
        }

        // --- Update character counter + highlighting ---
        self.update_char_counter(buffer);

        imp.highlighting.set(true);
        self.highlight_facets(buffer);
        imp.highlighting.set(false);

        // Get cursor position
        let cursor_offset = buffer.cursor_position();
        let cursor_iter = buffer.iter_at_offset(cursor_offset);

        // Walk backwards from cursor to find @ with a word boundary before it
        let mut search_iter = cursor_iter;
        let mut found_at = false;
        let mut at_offset = 0i32;

        while search_iter.backward_char() {
            let ch = search_iter.char();
            if ch == '@' {
                // Check that @ is at start of text or preceded by whitespace/punctuation
                let mut before = search_iter;
                if !before.backward_char()
                    || before.char().is_whitespace()
                    || matches!(before.char(), '(' | '[' | '{')
                {
                    found_at = true;
                    at_offset = search_iter.offset();
                }
                break;
            }
            // Stop if we hit whitespace (no @ in this word)
            if ch.is_whitespace() {
                break;
            }
        }

        if !found_at {
            self.hide_mention_popover();
            return;
        }

        // Extract the partial query (text between @ and cursor, exclusive of @)
        let query_start = buffer.iter_at_offset(at_offset + 1); // skip the @
        let query_text = buffer.text(&query_start, &cursor_iter, false).to_string();

        // Need at least 1 character to search
        if query_text.is_empty() {
            self.hide_mention_popover();
            return;
        }

        // Store the @ position for later insertion
        imp.mention_at_offset.set(at_offset);

        // Debounce: increment counter, fire search after delay if counter still matches
        let counter = imp.debounce_counter.get().wrapping_add(1);
        imp.debounce_counter.set(counter);

        let dialog_weak = self.downgrade();
        let query = query_text.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            let Some(dialog) = dialog_weak.upgrade() else {
                return;
            };
            // Only fire if no newer keystrokes have happened
            if dialog.imp().debounce_counter.get() != counter {
                return;
            }
            if let Some(cb) = dialog.imp().mention_search_callback.borrow().as_ref() {
                cb(query);
            }
        });
    }

    /// Populate the mention popover with search results and show it.
    pub fn set_mention_results(&self, profiles: Vec<Profile>) {
        let imp = self.imp();

        if profiles.is_empty() {
            self.hide_mention_popover();
            return;
        }

        if let Some(list) = imp.mention_list.borrow().as_ref() {
            // Clear existing rows
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }

            for profile in &profiles {
                let row = self.build_mention_row(profile);
                list.append(&row);
            }

            // Select first row
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        }

        imp.mention_results.replace(profiles);

        // Position the popover near the cursor in the text view
        self.position_mention_popover();

        if let Some(popover) = imp.mention_popover.borrow().as_ref() {
            if !popover.is_visible() {
                popover.popup();
            }
        }
    }

    /// Position the mention popover at the current @ location in the text view.
    fn position_mention_popover(&self) {
        let imp = self.imp();
        let tv = match imp.text_view.borrow().as_ref() {
            Some(tv) => tv.clone(),
            None => return,
        };
        let popover = match imp.mention_popover.borrow().as_ref() {
            Some(p) => p.clone(),
            None => return,
        };

        let buffer = tv.buffer();
        let at_offset = imp.mention_at_offset.get();
        let iter = buffer.iter_at_offset(at_offset);

        // Get the location of the @ character in TextView coordinates
        let (strong, _weak) = tv.cursor_locations(Some(&iter));

        // Convert buffer coords to widget coords
        let (wx, wy) =
            tv.buffer_to_window_coords(gtk4::TextWindowType::Widget, strong.x(), strong.y());

        let rect = gdk::Rectangle::new(wx, wy + strong.height(), 1, 1);
        popover.set_pointing_to(Some(&rect));
    }

    /// Build a single mention autocomplete row showing avatar + name + handle + follow status.
    fn build_mention_row(&self, profile: &Profile) -> gtk4::ListBoxRow {
        let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        row_box.set_margin_start(6);
        row_box.set_margin_end(6);
        row_box.set_margin_top(6);
        row_box.set_margin_bottom(6);

        let display_name = profile
            .display_name
            .clone()
            .unwrap_or_else(|| profile.handle.clone());

        // Circular avatar — show initials immediately, load real image async
        let avatar = adw::Avatar::new(32, Some(&display_name), true);
        if let Some(url) = &profile.avatar {
            avatar_cache::load_avatar(avatar.clone(), url.clone());
        }
        row_box.append(&avatar);

        // Name + handle + follow status stacked vertically
        let info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
        info_box.set_hexpand(true);
        info_box.set_valign(gtk4::Align::Center);

        let name_label = gtk4::Label::new(Some(&display_name));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        name_label.set_max_width_chars(28);
        name_label.add_css_class("heading");
        info_box.append(&name_label);

        // Handle + follow status on one line
        let handle_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

        let handle_label = gtk4::Label::new(Some(&format!("@{}", profile.handle)));
        handle_label.set_halign(gtk4::Align::Start);
        handle_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        handle_label.set_max_width_chars(22);
        handle_label.add_css_class("dim-label");
        handle_label.add_css_class("caption");
        handle_row.append(&handle_label);

        // Follow status as a subtle indicator
        let follow_text = match (&profile.viewer_following, &profile.viewer_followed_by) {
            (Some(_), Some(_)) => Some("· Mutuals"),
            (Some(_), None) => Some("· Following"),
            (None, Some(_)) => Some("· Follows you"),
            _ => None,
        };
        if let Some(text) = follow_text {
            let follow_label = gtk4::Label::new(Some(text));
            follow_label.add_css_class("dim-label");
            follow_label.add_css_class("caption");
            handle_row.append(&follow_label);
        }

        info_box.append(&handle_row);
        row_box.append(&info_box);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&row_box));

        // Accessible label
        let a11y_label = format!("{}, @{}", display_name, profile.handle);
        row.update_property(&[gtk4::accessible::Property::Label(&a11y_label)]);

        row
    }

    /// Insert the selected mention handle into the text buffer.
    fn insert_mention(&self, index: usize) {
        let imp = self.imp();
        let handle = {
            let results = imp.mention_results.borrow();
            match results.get(index) {
                Some(p) => p.handle.clone(),
                None => return,
            }
        };

        if let Some(tv) = imp.text_view.borrow().as_ref() {
            let buffer = tv.buffer();
            let at_offset = imp.mention_at_offset.get();
            let cursor_offset = buffer.cursor_position();

            // Delete from @ through cursor (the partial query)
            let start = buffer.iter_at_offset(at_offset);
            let end = buffer.iter_at_offset(cursor_offset);
            buffer.delete(&mut start.clone(), &mut end.clone());

            // Insert full @handle + trailing space
            let mention_text = format!("@{} ", handle);
            let mut insert_iter = buffer.iter_at_offset(at_offset);
            buffer.insert(&mut insert_iter, &mention_text);
        }

        self.hide_mention_popover();
    }

    /// Move the mention list selection up or down, wrapping at boundaries.
    fn move_mention_selection(&self, delta: i32) {
        let imp = self.imp();
        let result_count = imp.mention_results.borrow().len() as i32;
        if result_count == 0 {
            return;
        }
        if let Some(list) = imp.mention_list.borrow().as_ref() {
            let current = list.selected_row().map(|r| r.index()).unwrap_or(-1);
            // Wrap around: going past the end wraps to top, going before top wraps to bottom
            let next = (current + delta).rem_euclid(result_count);
            if let Some(row) = list.row_at_index(next) {
                list.select_row(Some(&row));

                // Scroll the row into view after GTK has done its layout pass.
                // We schedule an idle callback so allocation coordinates are valid.
                let row_ref = row.clone();
                glib::idle_add_local_once(move || {
                    // Walk up the parent chain to find the ScrolledWindow.
                    // GTK4 ScrolledWindow wraps children in a GtkViewport,
                    // so the chain is: row → ListBox → Viewport → ScrolledWindow.
                    let mut ancestor = row_ref.parent();
                    while let Some(widget) = ancestor {
                        if let Some(sw) = widget.downcast_ref::<gtk4::ScrolledWindow>() {
                            let adj = sw.vadjustment();
                            // Compute row position relative to the ListBox (the scrollable content)
                            if let Some(list_parent) = row_ref.parent() {
                                let (_, row_y) = row_ref
                                    .translate_coordinates(&list_parent, 0.0, 0.0)
                                    .unwrap_or((0.0, 0.0));
                                let row_h = row_ref.height() as f64;
                                let page_top = adj.value();
                                let page_bottom = page_top + adj.page_size();

                                if row_y + row_h > page_bottom {
                                    adj.set_value(row_y + row_h - adj.page_size());
                                } else if row_y < page_top {
                                    adj.set_value(row_y);
                                }
                            }
                            break;
                        }
                        ancestor = widget.parent();
                    }
                });
            }
        }
    }

    fn hide_mention_popover(&self) {
        let imp = self.imp();
        if let Some(popover) = imp.mention_popover.borrow().as_ref() {
            popover.popdown();
        }
        imp.mention_results.borrow_mut().clear();
    }

    fn emit_post(&self) {
        self.hide_mention_popover();

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

    /// Register a callback for mention typeahead search.
    /// Called with the partial handle text (without @) when the user types @query.
    pub fn connect_mention_search<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp()
            .mention_search_callback
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
