// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]

use crate::atproto::{ComposeData, ImageAttachment, LinkCardData, Profile};
use crate::state::AppSettings;
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

/// Maximum images per post
const MAX_IMAGES: usize = 4;

/// Common languages for the language picker (code, English name, native name).
const LANGUAGES: &[(&str, &str, &str)] = &[
    ("en", "English", "English"),
    ("es", "Spanish", "Espa\u{f1}ol"),
    ("pt", "Portuguese", "Portugu\u{ea}s"),
    ("ja", "Japanese", "\u{65e5}\u{672c}\u{8a9e}"),
    ("ko", "Korean", "\u{d55c}\u{ad6d}\u{c5b4}"),
    ("fr", "French", "Fran\u{e7}ais"),
    ("de", "German", "Deutsch"),
    ("it", "Italian", "Italiano"),
    ("zh", "Chinese", "\u{4e2d}\u{6587}"),
    (
        "ar",
        "Arabic",
        "\u{627}\u{644}\u{639}\u{631}\u{628}\u{64a}\u{629}",
    ),
    ("hi", "Hindi", "\u{939}\u{93f}\u{928}\u{94d}\u{926}\u{940}"),
    (
        "ru",
        "Russian",
        "\u{420}\u{443}\u{441}\u{441}\u{43a}\u{438}\u{439}",
    ),
    ("nl", "Dutch", "Nederlands"),
    ("pl", "Polish", "Polski"),
    ("sv", "Swedish", "Svenska"),
    ("tr", "Turkish", "T\u{fc}rk\u{e7}e"),
    (
        "uk",
        "Ukrainian",
        "\u{423}\u{43a}\u{440}\u{430}\u{457}\u{43d}\u{441}\u{44c}\u{43a}\u{430}",
    ),
    ("vi", "Vietnamese", "Ti\u{1ebf}ng Vi\u{1ec7}t"),
    ("th", "Thai", "\u{e44}\u{e17}\u{e22}"),
    ("id", "Indonesian", "Bahasa Indonesia"),
    ("ms", "Malay", "Bahasa Melayu"),
    ("fi", "Finnish", "Suomi"),
    ("da", "Danish", "Dansk"),
    ("no", "Norwegian", "Norsk"),
    ("cs", "Czech", "\u{10c}e\u{161}tina"),
    (
        "el",
        "Greek",
        "\u{395}\u{3bb}\u{3bb}\u{3b7}\u{3bd}\u{3b9}\u{3ba}\u{3ac}",
    ),
    ("he", "Hebrew", "\u{5e2}\u{5d1}\u{5e8}\u{5d9}\u{5ea}"),
    ("ro", "Romanian", "Rom\u{e2}n\u{103}"),
    ("hu", "Hungarian", "Magyar"),
    ("ca", "Catalan", "Catal\u{e0}"),
    ("gl", "Galician", "Galego"),
    ("eu", "Basque", "Euskara"),
    ("tl", "Filipino", "Filipino"),
    ("bn", "Bengali", "\u{9ac}\u{9be}\u{982}\u{9b2}\u{9be}"),
    ("fa", "Persian", "\u{641}\u{627}\u{631}\u{633}\u{6cc}"),
    ("ta", "Tamil", "\u{ba4}\u{bae}\u{bbf}\u{bb4}\u{bcd}"),
    ("ur", "Urdu", "\u{627}\u{631}\u{62f}\u{648}"),
    ("sk", "Slovak", "Sloven\u{10d}ina"),
    (
        "bg",
        "Bulgarian",
        "\u{411}\u{44a}\u{43b}\u{433}\u{430}\u{440}\u{441}\u{43a}\u{438}",
    ),
    ("hr", "Croatian", "Hrvatski"),
    (
        "sr",
        "Serbian",
        "\u{421}\u{440}\u{43f}\u{441}\u{43a}\u{438}",
    ),
    ("sl", "Slovenian", "Sloven\u{161}\u{10d}ina"),
    ("lt", "Lithuanian", "Lietuvi\u{173}"),
    ("lv", "Latvian", "Latvie\u{161}u"),
    ("et", "Estonian", "Eesti"),
];

/// Get the display name for a language code.
fn language_display_name(code: &str) -> String {
    LANGUAGES
        .iter()
        .find(|(c, _, _)| *c == code)
        .map(|(_, name, _)| name.to_string())
        .unwrap_or_else(|| code.to_uppercase())
}

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

/// An image being composed for attachment to a post.
#[derive(Clone)]
pub struct ComposeImage {
    pub data: Vec<u8>,
    pub mime_type: String,
    pub alt_text: String,
    pub width: u32,
    pub height: u32,
    pub texture: gdk::Texture,
}

/// A single post block in the thread composer (Post 2, 3, etc.)
pub struct ThreadPostBlock {
    pub container: gtk4::Box,
    pub text_view: gtk4::TextView,
    pub image_strip: gtk4::Box,
    pub images: Vec<ComposeImage>,
    pub char_counter: gtk4::Label,
    /// Per-post content warning (each post can have its own CW)
    pub content_warning: Option<String>,
    /// "Add Content Warning..." button (visible when images attached)
    pub cw_button: gtk4::Button,
    /// "Remove All Images" button (visible when images attached)
    pub remove_all_button: gtk4::Button,
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
        pub post_callback: RefCell<Option<Box<dyn Fn(ComposeData) + 'static>>>,
        pub reply_callback: RefCell<Option<Box<dyn Fn(ComposeData, String, String) + 'static>>>,
        pub quote_callback: RefCell<Option<Box<dyn Fn(ComposeData, String, String) + 'static>>>,
        pub char_counter: RefCell<Option<gtk4::Label>>,
        // Image attachments
        pub image_strip: RefCell<Option<gtk4::Box>>,
        pub images: RefCell<Vec<ComposeImage>>,
        pub add_image_button: RefCell<Option<gtk4::Button>>,
        pub remove_all_images_button: RefCell<Option<gtk4::Button>>,
        // Language selection
        pub language_button: RefCell<Option<gtk4::Button>>,
        pub selected_language: RefCell<String>,
        // Content warning
        pub cw_button: RefCell<Option<gtk4::Button>>,
        pub content_warning: RefCell<Option<String>>,
        // Interaction settings — button shows dynamic text like "Everyone can reply"
        pub interaction_label: RefCell<Option<gtk4::Button>>,
        pub threadgate_config: RefCell<Option<crate::atproto::ThreadgateConfig>>,
        pub postgate_config: RefCell<Option<crate::atproto::PostgateConfig>>,
        // Link card preview
        pub link_preview_box: RefCell<Option<gtk4::Box>>,
        pub link_card_data: RefCell<Option<LinkCardData>>,
        /// The URL we've already fetched or are fetching (to avoid duplicate requests)
        pub link_preview_url: RefCell<Option<String>>,
        /// User dismissed the link card — don't re-fetch until text changes the URL
        pub link_preview_dismissed: Cell<bool>,
        /// Callback to fetch link card metadata (called with URL string)
        pub link_preview_fetch_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
        /// Counter for link preview debounce
        pub link_debounce_counter: Cell<u32>,
        // Thread composer
        /// Container holding all thread post blocks
        pub thread_container: RefCell<Option<gtk4::Box>>,
        /// Additional thread posts (Post 2, 3, ... — Post 1 is the main text_view)
        pub thread_posts: RefCell<Vec<ThreadPostBlock>>,
        /// "Add to thread" button
        pub add_thread_button: RefCell<Option<gtk4::Button>>,
        /// Thread post callback (called with Vec<ComposeData>)
        pub thread_callback: RefCell<Option<Box<dyn Fn(Vec<ComposeData>) + 'static>>>,
        /// Which post is currently focused (0 = main, 1+ = thread posts)
        pub focused_post_index: Cell<usize>,
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
        dialog.set_content_width(480);
        dialog.set_content_height(360);
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

    /// Generate a human-readable summary of the current threadgate configuration.
    fn interaction_summary_text(threadgate: &Option<crate::atproto::ThreadgateConfig>) -> String {
        use crate::atproto::ThreadgateRule;
        match threadgate {
            None => "Everyone can reply".to_string(),
            Some(tg) if tg.allow_rules.is_empty() => "Replies disabled".to_string(),
            Some(tg) => {
                let mut parts = Vec::new();
                if tg.allow_rules.contains(&ThreadgateRule::FollowingRule) {
                    parts.push("People you follow");
                }
                if tg.allow_rules.contains(&ThreadgateRule::MentionRule) {
                    parts.push("People you mention");
                }
                if tg.allow_rules.contains(&ThreadgateRule::FollowersRule) {
                    parts.push("Your followers");
                }
                if parts.is_empty() {
                    "Replies disabled".to_string()
                } else {
                    format!("{} can reply", parts.join(", "))
                }
            }
        }
    }

    /// Update the interaction settings label to reflect current config.
    fn update_interaction_label(&self) {
        let imp = self.imp();
        let tg = imp.threadgate_config.borrow().clone();
        let text = Self::interaction_summary_text(&tg);
        if let Some(btn) = imp.interaction_label.borrow().as_ref() {
            btn.set_label(&text);
            btn.set_tooltip_text(Some(&format!("Interaction settings: {}", text)));
            btn.update_property(&[gtk4::accessible::Property::Label(&format!(
                "Interaction settings: {}",
                text
            ))]);
        }
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
                    &context.text[..super::floor_char_boundary(&context.text, 100)]
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
        // Header bar with Cancel (start), action buttons + Post (end)
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

        // Attach image button in header bar
        let add_image_btn = gtk4::Button::from_icon_name("image-x-generic-symbolic");
        add_image_btn.add_css_class("flat");
        add_image_btn.set_tooltip_text(Some("Attach image"));
        add_image_btn.update_property(&[gtk4::accessible::Property::Label("Attach image")]);
        header.pack_end(&add_image_btn);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);

        // ── Bottom bar: status labels (pinned, outside scroll) ──
        let status_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        status_row.add_css_class("compose-status-row");
        status_row.set_margin_start(24);
        status_row.set_margin_end(24);
        status_row.set_margin_bottom(8);
        status_row.set_margin_top(4);

        // Spacer pushes labels to the right
        let status_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        status_spacer.set_hexpand(true);
        status_row.append(&status_spacer);

        // Language button
        let default_lang = AppSettings::load()
            .default_post_language
            .unwrap_or_else(|| "en".to_string());
        let lang_display = language_display_name(&default_lang);
        let lang_btn = gtk4::Button::with_label(&lang_display);
        lang_btn.add_css_class("flat");
        lang_btn.add_css_class("caption");
        lang_btn.set_tooltip_text(Some(&format!("Post language: {}", lang_display)));
        lang_btn.update_property(&[gtk4::accessible::Property::Label(&format!(
            "Post language: {}",
            lang_display
        ))]);
        status_row.append(&lang_btn);

        // Interaction settings button — shows dynamic text
        let settings = AppSettings::load();
        let initial_interaction_text = Self::interaction_summary_text(&settings.default_threadgate);
        let interaction_btn = gtk4::Button::with_label(&initial_interaction_text);
        interaction_btn.add_css_class("flat");
        interaction_btn.add_css_class("caption");
        interaction_btn.set_tooltip_text(Some("Interaction settings"));
        interaction_btn.update_property(&[gtk4::accessible::Property::Label(&format!(
            "Interaction settings: {}",
            initial_interaction_text
        ))]);
        status_row.append(&interaction_btn);

        toolbar.add_bottom_bar(&status_row);

        // ── Scrollable content area ──
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(12);
        content.set_margin_bottom(8);
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
        text_view.add_css_class("compose-text");
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

        // Emoji tag: fix vertical line overlap and horizontal overshoot.
        // line_height(1.4) prevents emoji from overlapping adjacent lines.
        // letter_spacing adds trailing space after the emoji glyph so the
        // next character doesn't render under the emoji's right edge.
        // Base is 6px (6144 Pango units), scaled by the font size setting so
        // the gap stays proportional at larger text sizes. Note that Pango
        // splits letter_spacing across both sides of the glyph, so higher
        // values also increase the left gap. 6px is the best compromise
        // between right-side clearance and left-side inflation.
        let font_scale = AppSettings::load().font_size.scale_factor();
        let emoji_spacing = (6144.0 * font_scale) as i32;
        let emoji_tag = gtk4::TextTag::new(Some(TAG_EMOJI));
        emoji_tag.set_line_height(1.4);
        emoji_tag.set_letter_spacing(emoji_spacing);
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
                    if let Some(list) = imp.mention_list.borrow().as_ref()
                        && let Some(row) = list.selected_row()
                    {
                        dialog.insert_mention(row.index() as usize);
                        return glib::Propagation::Stop;
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

        // --- Image attachment strip ---
        let image_strip = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        image_strip.add_css_class("compose-image-strip");
        image_strip.set_visible(false);
        content.append(&image_strip);

        // --- Link card preview ---
        let link_preview_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        link_preview_box.add_css_class("compose-link-card");
        link_preview_box.set_visible(false);
        content.append(&link_preview_box);

        // Character counter for main post — right-aligned
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

        // Main post action row: "Add Content Warning..." + "Remove All Images"
        // (shown below image strip when images are attached)
        let main_action_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        main_action_row.set_visible(false);

        let cw_btn = gtk4::Button::with_label("Add Content Warning\u{2026}");
        cw_btn.add_css_class("flat");
        cw_btn.add_css_class("caption");
        cw_btn.set_tooltip_text(Some("Set content warning for attached media"));
        cw_btn.update_property(&[gtk4::accessible::Property::Label(
            "Set content warning for attached media",
        )]);
        main_action_row.append(&cw_btn);

        let remove_all_btn = gtk4::Button::with_label("Remove All Images");
        remove_all_btn.add_css_class("flat");
        remove_all_btn.add_css_class("caption");
        remove_all_btn.add_css_class("destructive-action");
        remove_all_btn.set_tooltip_text(Some("Remove all attached images"));
        main_action_row.append(&remove_all_btn);

        content.append(&main_action_row);

        // Wire up add image button — targets the currently focused post
        let dialog_weak = self.downgrade();
        add_image_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                let focused = dialog.imp().focused_post_index.get();
                if focused == 0 {
                    dialog.open_image_chooser();
                } else {
                    dialog.open_thread_image_chooser(focused - 1);
                }
            }
        });

        // Wire up remove all images button for main post
        let dialog_weak = self.downgrade();
        remove_all_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.remove_all_images();
            }
        });

        // Wire up content warning button — opens a dialog
        let dialog_weak = self.downgrade();
        cw_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.show_content_warning_dialog();
            }
        });

        // Wire up language button
        let dialog_weak = self.downgrade();
        lang_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.show_language_picker();
            }
        });

        // Wire up interaction settings button
        let dialog_weak = self.downgrade();
        interaction_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.show_interaction_settings();
            }
        });

        // Track focus on the main text view (post index 0)
        let dialog_weak = self.downgrade();
        let focus_controller = gtk4::EventControllerFocus::new();
        focus_controller.connect_enter(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.imp().focused_post_index.set(0);
            }
        });
        text_view.add_controller(focus_controller);

        // --- Thread posts container ---
        let thread_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        thread_container.set_visible(false);
        content.append(&thread_container);

        // "Add to thread" button
        let add_thread_btn = gtk4::Button::new();
        let add_thread_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        add_thread_content.set_halign(gtk4::Align::Center);
        let add_icon = gtk4::Image::from_icon_name("list-add-symbolic");
        add_thread_content.append(&add_icon);
        let add_label = gtk4::Label::new(Some("Add to thread"));
        add_thread_content.append(&add_label);
        add_thread_btn.set_child(Some(&add_thread_content));
        add_thread_btn.add_css_class("flat");
        add_thread_btn.set_tooltip_text(Some("Add another post to this thread"));
        add_thread_btn.update_property(&[gtk4::accessible::Property::Label(
            "Add another post to this thread",
        )]);
        content.append(&add_thread_btn);

        let dialog_weak = self.downgrade();
        add_thread_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.add_thread_post();
            }
        });

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

        // Wrap content in a ScrolledWindow so the dialog can handle
        // overflow when many thread posts are added.
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_propagate_natural_height(true);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&content));

        toolbar.set_content(Some(&scrolled));

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
        imp.image_strip.replace(Some(image_strip));
        imp.add_image_button.replace(Some(add_image_btn));
        imp.remove_all_images_button.replace(Some(remove_all_btn));
        imp.link_preview_box.replace(Some(link_preview_box));
        imp.language_button.replace(Some(lang_btn));
        imp.selected_language.replace(default_lang);
        imp.cw_button.replace(Some(cw_btn));
        imp.interaction_label.replace(Some(interaction_btn));
        imp.thread_container.replace(Some(thread_container));
        imp.add_thread_button.replace(Some(add_thread_btn));
        // Load default threadgate/postgate from settings
        let settings = AppSettings::load();
        imp.threadgate_config.replace(settings.default_threadgate);
        imp.postgate_config.replace(settings.default_postgate);

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

        // Disable Post button when over limit or empty (unless images are attached)
        let has_images = !imp.images.borrow().is_empty();
        if let Some(btn) = imp.post_button.borrow().as_ref() {
            btn.set_sensitive((grapheme_count > 0 || has_images) && remaining >= 0);
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
            let url = m.as_str().trim_end_matches(['.', ',', ';', '!', '?']);
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

        // Apply emoji tags for vertical line-height fix
        let chars: Vec<char> = text.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            if is_emoji(ch) {
                let s = buffer.iter_at_offset(i as i32);
                let e = buffer.iter_at_offset(i as i32 + 1);
                buffer.apply_tag_by_name(TAG_EMOJI, &s, &e);
            }
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

        // --- Link card detection (debounced) ---
        self.check_for_link_card(buffer);

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

        if let Some(popover) = imp.mention_popover.borrow().as_ref()
            && !popover.is_visible()
        {
            popover.popup();
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

    // ─── Image attachment methods ───

    /// Open a file chooser dialog for selecting images.
    fn open_image_chooser(&self) {
        let image_count = self.imp().images.borrow().len();
        if image_count >= MAX_IMAGES {
            return;
        }

        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("Images"));
        filter.add_mime_type("image/jpeg");
        filter.add_mime_type("image/png");
        filter.add_mime_type("image/gif");
        filter.add_mime_type("image/webp");

        let window = self.root().and_then(|r| r.downcast::<gtk4::Window>().ok());

        let chooser = gtk4::FileChooserNative::new(
            Some("Select Image"),
            window.as_ref(),
            gtk4::FileChooserAction::Open,
            Some("Open"),
            Some("Cancel"),
        );
        chooser.add_filter(&filter);

        let dialog_weak = self.downgrade();
        chooser.connect_response(move |chooser, response| {
            if response == gtk4::ResponseType::Accept
                && let Some(file) = chooser.file()
                && let Some(path) = file.path()
                && let Some(dialog) = dialog_weak.upgrade()
            {
                dialog.load_image_from_path(&path);
            }
        });

        chooser.show();
    }

    /// Load an image file and add it to the compose image strip.
    fn load_image_from_path(&self, path: &std::path::Path) {
        let imp = self.imp();
        if imp.images.borrow().len() >= MAX_IMAGES {
            return;
        }

        // Read file bytes
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Failed to read image: {}", e);
                return;
            }
        };

        // Determine MIME type from extension
        let mime_type = match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
        {
            Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg".to_string(),
            Some(ext) if ext == "png" => "image/png".to_string(),
            Some(ext) if ext == "gif" => "image/gif".to_string(),
            Some(ext) if ext == "webp" => "image/webp".to_string(),
            _ => "image/jpeg".to_string(),
        };

        // Get image dimensions using the image crate
        let (width, height) = match image::image_dimensions(path) {
            Ok((w, h)) => (w, h),
            Err(_) => (1, 1), // fallback
        };

        // Create GDK texture for preview
        let texture = match gdk::Texture::from_filename(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Failed to create texture: {}", e);
                return;
            }
        };

        let compose_image = ComposeImage {
            data,
            mime_type,
            alt_text: String::new(),
            width,
            height,
            texture,
        };

        imp.images.borrow_mut().push(compose_image);
        self.rebuild_image_strip();
        self.update_image_button_state();
        // Images take precedence over link cards — hide link preview
        self.clear_link_preview();

        // Re-evaluate post button state (images allow posting without text)
        if let Some(tv) = imp.text_view.borrow().as_ref() {
            self.update_char_counter(&tv.buffer());
        }
    }

    /// Rebuild the image strip thumbnails from current images list.
    fn rebuild_image_strip(&self) {
        let imp = self.imp();
        let strip = match imp.image_strip.borrow().as_ref() {
            Some(s) => s.clone(),
            None => return,
        };

        // Clear existing thumbnails
        while let Some(child) = strip.first_child() {
            strip.remove(&child);
        }

        let images = imp.images.borrow();
        let has_images = !images.is_empty();

        for (i, img) in images.iter().enumerate() {
            let thumb_box = gtk4::Overlay::new();
            thumb_box.set_size_request(80, 80);
            thumb_box.add_css_class("compose-thumbnail");

            // Image
            let picture = gtk4::Picture::new();
            picture.set_paintable(Some(&img.texture));
            picture.set_can_shrink(true);
            picture.set_size_request(80, 80);
            picture.add_css_class("compose-thumbnail-image");
            thumb_box.set_child(Some(&picture));

            // Remove button overlay (top-right)
            let remove_btn = gtk4::Button::from_icon_name("window-close-symbolic");
            remove_btn.add_css_class("circular");
            remove_btn.add_css_class("osd");
            remove_btn.set_halign(gtk4::Align::End);
            remove_btn.set_valign(gtk4::Align::Start);
            remove_btn.set_margin_top(4);
            remove_btn.set_margin_end(4);
            remove_btn.set_tooltip_text(Some(&format!("Remove image {}", i + 1)));
            remove_btn.update_property(&[gtk4::accessible::Property::Label(&format!(
                "Remove image {}",
                i + 1
            ))]);
            let dialog_weak = self.downgrade();
            let idx = i;
            remove_btn.connect_clicked(move |_| {
                if let Some(dialog) = dialog_weak.upgrade() {
                    dialog.remove_image(idx);
                }
            });
            thumb_box.add_overlay(&remove_btn);

            // ALT badge (bottom-left) — shown when alt text is set
            if !img.alt_text.is_empty() {
                let alt_badge = gtk4::Label::new(Some("ALT"));
                alt_badge.add_css_class("compose-alt-badge");
                alt_badge.add_css_class("osd");
                alt_badge.set_halign(gtk4::Align::Start);
                alt_badge.set_valign(gtk4::Align::End);
                alt_badge.set_margin_bottom(4);
                alt_badge.set_margin_start(4);
                alt_badge
                    .update_property(&[gtk4::accessible::Property::Label("Alt text provided")]);
                thumb_box.add_overlay(&alt_badge);
            }

            // Make the thumbnail clickable to edit alt text
            let click = gtk4::GestureClick::new();
            let dialog_weak = self.downgrade();
            let idx = i;
            click.connect_released(move |gesture, _, _, _| {
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                if let Some(dialog) = dialog_weak.upgrade() {
                    dialog.show_alt_text_dialog(idx);
                }
            });
            picture.add_controller(click);

            // Accessible label for the thumbnail
            let alt_desc = if img.alt_text.is_empty() {
                "No alt text".to_string()
            } else {
                format!("Alt text: {}", img.alt_text)
            };
            thumb_box.update_property(&[gtk4::accessible::Property::Label(&format!(
                "Image {}. {}. Click to edit alt text.",
                i + 1,
                alt_desc
            ))]);

            strip.append(&thumb_box);
        }

        strip.set_visible(has_images);
    }

    /// Remove a specific image by index.
    fn remove_image(&self, index: usize) {
        let imp = self.imp();
        {
            let mut images = imp.images.borrow_mut();
            if index < images.len() {
                images.remove(index);
            }
        }
        self.rebuild_image_strip();
        self.update_image_button_state();

        // Re-evaluate post button state
        if let Some(tv) = imp.text_view.borrow().as_ref() {
            let buffer = tv.buffer();
            self.update_char_counter(&buffer);
            // If all images removed, re-check for link cards
            if imp.images.borrow().is_empty() {
                self.check_for_link_card(&buffer);
            }
        }
    }

    /// Remove all attached images.
    fn remove_all_images(&self) {
        self.imp().images.borrow_mut().clear();
        self.rebuild_image_strip();
        self.update_image_button_state();

        // Re-evaluate post button state
        if let Some(tv) = self.imp().text_view.borrow().as_ref() {
            let buffer = tv.buffer();
            self.update_char_counter(&buffer);
            // Re-check for link cards now that images are cleared
            self.check_for_link_card(&buffer);
        }
    }

    /// Update the add image button and action row visibility/state for the main post.
    fn update_image_button_state(&self) {
        let imp = self.imp();
        let count = imp.images.borrow().len();
        let at_max = count >= MAX_IMAGES;
        let has_images = count > 0;

        if let Some(btn) = imp.add_image_button.borrow().as_ref() {
            btn.set_sensitive(!at_max);
            btn.update_property(&[gtk4::accessible::Property::Label(&format!(
                "Attach image, {} of {} attached",
                count, MAX_IMAGES
            ))]);
        }

        // Show/hide the action row (CW + Remove All) based on whether images are attached
        if let Some(btn) = imp.remove_all_images_button.borrow().as_ref() {
            btn.set_visible(has_images);
            // Show/hide the parent action row
            if let Some(parent) = btn.parent() {
                parent.set_visible(has_images);
            }
        }

        // Content warning button — update label text based on current CW
        if let Some(btn) = imp.cw_button.borrow().as_ref() {
            btn.set_visible(has_images);
            if has_images {
                let cw = imp.content_warning.borrow();
                if let Some(val) = cw.as_ref() {
                    let display = match val.as_str() {
                        "sexual" => "Suggestive",
                        "nudity" => "Nudity",
                        "porn" => "Pornography",
                        "graphic-media" => "Graphic Media",
                        _ => val.as_str(),
                    };
                    btn.set_label(&format!("Content Warning: {}", display));
                    btn.set_tooltip_text(Some(&format!("Content warning: {}", display)));
                    btn.add_css_class("accent");
                } else {
                    btn.set_label("Add Content Warning\u{2026}");
                    btn.set_tooltip_text(Some("Set content warning for attached media"));
                    btn.remove_css_class("accent");
                }
            }
        }

        // Clear content warning if all images removed
        if !has_images {
            imp.content_warning.replace(None);
        }
    }

    /// Show a dialog for editing the alt text of an image.
    fn show_alt_text_dialog(&self, image_index: usize) {
        let imp = self.imp();
        let current_alt = {
            let images = imp.images.borrow();
            match images.get(image_index) {
                Some(img) => img.alt_text.clone(),
                None => return,
            }
        };

        let alt_dialog = adw::Dialog::new();
        alt_dialog.set_title("Add Descriptive Text");
        alt_dialog.set_content_width(360);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let done_btn = gtk4::Button::with_label("Done");
        done_btn.add_css_class("suggested-action");
        header.pack_end(&done_btn);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(12);
        content.set_margin_bottom(24);

        let desc_label = gtk4::Label::new(Some(
            "Help the blind and vision impaired to understand your posts by adding descriptive text to your images.",
        ));
        desc_label.set_wrap(true);
        desc_label.set_halign(gtk4::Align::Start);
        desc_label.add_css_class("dim-label");
        content.append(&desc_label);

        let label = gtk4::Label::new(Some("Describe this image"));
        label.set_halign(gtk4::Align::Start);
        label.add_css_class("heading");
        content.append(&label);

        let text_view = gtk4::TextView::new();
        text_view.set_wrap_mode(gtk4::WrapMode::WordChar);
        text_view.set_vexpand(true);
        text_view.set_left_margin(8);
        text_view.set_right_margin(8);
        text_view.set_top_margin(8);
        text_view.set_bottom_margin(8);
        text_view.add_css_class("card");
        text_view.update_property(&[gtk4::accessible::Property::Label("Image description")]);

        if !current_alt.is_empty() {
            text_view.buffer().set_text(&current_alt);
        }

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&text_view));
        content.append(&scrolled);

        toolbar.set_content(Some(&content));
        alt_dialog.set_child(Some(&toolbar));

        let dialog_weak = self.downgrade();
        let alt_dialog_weak = alt_dialog.downgrade();
        let text_view_for_done = text_view.clone();
        done_btn.connect_clicked(move |_| {
            let buffer = text_view_for_done.buffer();
            let start = buffer.start_iter();
            let end = buffer.end_iter();
            let alt_text = buffer
                .text(&start, &end, false)
                .to_string()
                .trim()
                .to_string();

            if let Some(dialog) = dialog_weak.upgrade() {
                {
                    let mut images = dialog.imp().images.borrow_mut();
                    if let Some(img) = images.get_mut(image_index) {
                        img.alt_text = alt_text;
                    }
                }
                dialog.rebuild_image_strip();
            }

            if let Some(alt_dlg) = alt_dialog_weak.upgrade() {
                alt_dlg.close();
            }
        });

        // Present the alt text dialog
        let window = self.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
        if let Some(win) = window.as_ref() {
            alt_dialog.present(Some(win));
        }
    }

    // ─── Language picker ───

    /// Show the language selection dialog.
    fn show_language_picker(&self) {
        let lang_dialog = adw::Dialog::new();
        lang_dialog.set_title("Select Post Language");
        lang_dialog.set_content_width(360);
        lang_dialog.set_content_height(420);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let done_btn = gtk4::Button::with_label("Done");
        done_btn.add_css_class("suggested-action");
        header.pack_end(&done_btn);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_margin_start(16);
        content.set_margin_end(16);
        content.set_margin_top(8);
        content.set_margin_bottom(16);

        let desc_label = gtk4::Label::new(Some(
            "Pick the appropriate language for your post so it can appear in community-created feeds that filter by language.",
        ));
        desc_label.set_wrap(true);
        desc_label.set_xalign(0.0);
        desc_label.add_css_class("dim-label");
        content.append(&desc_label);

        let search = gtk4::SearchEntry::new();
        search.set_placeholder_text(Some("Search Languages..."));
        search.set_hexpand(true);
        search.update_property(&[gtk4::accessible::Property::Label("Search languages")]);
        content.append(&search);

        let list = gtk4::ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::Single);
        list.add_css_class("boxed-list");

        let current_lang = self.imp().selected_language.borrow().clone();

        for (code, english_name, native_name) in LANGUAGES {
            let row = adw::ActionRow::new();
            row.set_title(english_name);
            row.set_subtitle(native_name);

            if *code == current_lang {
                let check = gtk4::Image::from_icon_name("object-select-symbolic");
                check.set_valign(gtk4::Align::Center);
                row.add_suffix(&check);
            }

            // Store code in widget name for retrieval on selection
            row.set_widget_name(code);
            list.append(&row);
        }

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&list));
        content.append(&scrolled);

        toolbar.set_content(Some(&content));
        lang_dialog.set_child(Some(&toolbar));

        // Search filtering
        let list_ref = list.clone();
        search.connect_search_changed(move |entry| {
            let query = entry.text().to_string().to_lowercase();
            let mut idx = 0;
            while let Some(row) = list_ref.row_at_index(idx) {
                if query.is_empty() {
                    row.set_visible(true);
                } else if let Some(action_row) = row
                    .child()
                    .and_then(|c| c.downcast::<adw::ActionRow>().ok())
                {
                    let title = action_row.title().to_string().to_lowercase();
                    let subtitle = action_row
                        .subtitle()
                        .map(|s| s.to_string().to_lowercase())
                        .unwrap_or_default();
                    row.set_visible(title.contains(&query) || subtitle.contains(&query));
                } else {
                    row.set_visible(true);
                }
                idx += 1;
            }
        });

        // Row selection
        let dialog_weak = self.downgrade();
        let lang_dialog_weak = lang_dialog.downgrade();
        list.connect_row_activated(move |_, row| {
            if let Some(dialog) = dialog_weak.upgrade() {
                // Get the code from the child ActionRow's widget name
                if let Some(child) = row.child() {
                    let code = child.widget_name().to_string();
                    if !code.is_empty() {
                        dialog.set_language(&code);
                    }
                }
            }
            if let Some(dlg) = lang_dialog_weak.upgrade() {
                dlg.close();
            }
        });

        // Done button closes without changing
        let lang_dialog_weak = lang_dialog.downgrade();
        done_btn.connect_clicked(move |_| {
            if let Some(dlg) = lang_dialog_weak.upgrade() {
                dlg.close();
            }
        });

        let window = self.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
        if let Some(win) = window.as_ref() {
            lang_dialog.present(Some(win));
        }
    }

    /// Set the selected post language.
    fn set_language(&self, code: &str) {
        let imp = self.imp();
        imp.selected_language.replace(code.to_string());
        let display = language_display_name(code);
        if let Some(btn) = imp.language_button.borrow().as_ref() {
            btn.set_label(&display);
            btn.set_tooltip_text(Some(&format!("Post language: {}", display)));
            btn.update_property(&[gtk4::accessible::Property::Label(&format!(
                "Post language: {}",
                display
            ))]);
        }
    }

    // ─── Content warning ───

    /// Show the content warning dialog (matches Bluesky's native UI).
    fn show_content_warning_dialog(&self) {
        let imp = self.imp();

        let cw_dialog = adw::Dialog::new();
        cw_dialog.set_title("Content Warning");
        cw_dialog.set_content_width(400);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let done_btn = gtk4::Button::with_label("Done");
        done_btn.add_css_class("suggested-action");
        header.pack_end(&done_btn);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(12);
        content.set_margin_bottom(24);

        let group = adw::PreferencesGroup::new();

        let current_cw = imp.content_warning.borrow().clone();

        // Track the selection in a shared cell
        let selected = std::rc::Rc::new(std::cell::RefCell::new(current_cw.unwrap_or_default()));

        // Options: None, Suggestive, Nudity, Pornography, Graphic Media
        let cw_options: &[(&str, &str, &str)] = &[
            (
                "",
                "None",
                "The images attached to this post are suitable for everyone",
            ),
            (
                "sexual",
                "Suggestive",
                "The images attached to this post are meant for adults",
            ),
            (
                "nudity",
                "Nudity",
                "The images attached to this post contain artistic or non-erotic nudity",
            ),
            (
                "porn",
                "Pornography",
                "The images attached to this post contain sexual activity or erotic nudity",
            ),
            (
                "graphic-media",
                "Graphic Media",
                "The images attached to this post contain disturbing content such as violence or gore",
            ),
        ];

        // Use CheckButtons in a group so only one can be selected
        let mut first_check: Option<gtk4::CheckButton> = None;

        for (value, label, description) in cw_options {
            let row = adw::ActionRow::new();
            row.set_title(label);
            row.set_subtitle(description);
            row.set_activatable(true);

            let check = gtk4::CheckButton::new();
            let is_active = if value.is_empty() {
                selected.borrow().is_empty()
            } else {
                selected.borrow().as_str() == *value
            };
            check.set_active(is_active);

            // Group all checkbuttons together
            if let Some(ref first) = first_check {
                check.set_group(Some(first));
            } else {
                first_check = Some(check.clone());
            }

            // Update the shared selection when toggled
            let selected_ref = selected.clone();
            let val = value.to_string();
            check.connect_toggled(move |cb| {
                if cb.is_active() {
                    *selected_ref.borrow_mut() = val.clone();
                }
            });

            row.add_suffix(&check);
            row.set_activatable_widget(Some(&check));

            group.add(&row);
        }

        content.append(&group.clone().upcast::<gtk4::Widget>());
        toolbar_view.set_content(Some(&content));
        cw_dialog.set_child(Some(&toolbar_view));

        // Done button: apply the selection
        let dialog_weak = self.downgrade();
        let cw_dialog_weak = cw_dialog.downgrade();
        let selected_for_done = selected;
        done_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                let val = selected_for_done.borrow().clone();
                let imp = dialog.imp();
                if val.is_empty() {
                    imp.content_warning.replace(None);
                } else {
                    imp.content_warning.replace(Some(val));
                }
                dialog.update_image_button_state();
            }

            if let Some(dlg) = cw_dialog_weak.upgrade() {
                dlg.close();
            }
        });

        let window = self.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
        if let Some(win) = window.as_ref() {
            cw_dialog.present(Some(win));
        }
    }

    // ─── Interaction settings ───

    /// Show the interaction settings dialog (threadgate + postgate).
    fn show_interaction_settings(&self) {
        let imp = self.imp();
        let int_dialog = adw::Dialog::new();
        int_dialog.set_title("Interaction Settings");
        int_dialog.set_content_width(400);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let done_btn = gtk4::Button::with_label("Done");
        done_btn.add_css_class("suggested-action");
        header.pack_end(&done_btn);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(12);
        content.set_margin_bottom(24);

        // --- Threadgate section ---
        let tg_group = adw::PreferencesGroup::new();

        // Get current state
        let current_tg = imp.threadgate_config.borrow().clone();
        let replies_enabled = current_tg.is_none();

        let reply_switch = adw::SwitchRow::new();
        reply_switch.set_title("Allow replies to this thread");
        reply_switch.set_active(replies_enabled);
        tg_group.add(&reply_switch);

        content.append(&tg_group.clone().upcast::<gtk4::Widget>());

        // Limit replies section
        let limit_group = adw::PreferencesGroup::new();
        limit_group.set_title("Limit replies to");
        limit_group.set_visible(!replies_enabled);

        let following_check = gtk4::CheckButton::with_label("People you follow");
        following_check.set_active(current_tg.as_ref().is_some_and(|tg| {
            tg.allow_rules
                .contains(&crate::atproto::ThreadgateRule::FollowingRule)
        }));
        limit_group.add(&following_check);

        let mention_check = gtk4::CheckButton::with_label("People you mention");
        mention_check.set_active(current_tg.as_ref().is_some_and(|tg| {
            tg.allow_rules
                .contains(&crate::atproto::ThreadgateRule::MentionRule)
        }));
        limit_group.add(&mention_check);

        let followers_check = gtk4::CheckButton::with_label("Your followers");
        followers_check.set_active(current_tg.as_ref().is_some_and(|tg| {
            tg.allow_rules
                .contains(&crate::atproto::ThreadgateRule::FollowersRule)
        }));
        limit_group.add(&followers_check);

        content.append(&limit_group.clone().upcast::<gtk4::Widget>());

        // Toggle limit section visibility
        let limit_group_ref = limit_group.clone();
        reply_switch.connect_active_notify(move |switch| {
            limit_group_ref.set_visible(!switch.is_active());
        });

        // --- Postgate section ---
        let pg_group = adw::PreferencesGroup::new();

        let current_pg = imp.postgate_config.borrow().clone();
        let quoting_enabled = !current_pg.as_ref().is_some_and(|pg| pg.disable_quoting);

        let quote_switch = adw::SwitchRow::new();
        quote_switch.set_title("Allow people to quote your posts");
        quote_switch.set_active(quoting_enabled);
        pg_group.add(&quote_switch);

        content.append(&pg_group.clone().upcast::<gtk4::Widget>());

        // Note about settings
        let note = gtk4::Label::new(Some(
            "These settings only apply to other people\u{2014}you can always reply to and quote your own posts.",
        ));
        note.set_wrap(true);
        note.set_xalign(0.0);
        note.add_css_class("dim-label");
        note.add_css_class("caption");
        content.append(&note);

        // Use as default button
        let default_btn = gtk4::Button::with_label("Use these settings by default");
        default_btn.add_css_class("flat");
        default_btn.set_halign(gtk4::Align::Center);
        content.append(&default_btn);

        toolbar_view.set_content(Some(&content));
        int_dialog.set_child(Some(&toolbar_view));

        // Done button: apply settings, update label, and close
        let dialog_weak = self.downgrade();
        let int_dialog_weak = int_dialog.downgrade();
        let reply_switch_ref = reply_switch.clone();
        let following_check_ref = following_check.clone();
        let mention_check_ref = mention_check.clone();
        let followers_check_ref = followers_check.clone();
        let quote_switch_ref = quote_switch.clone();
        done_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                let imp = dialog.imp();

                // Build threadgate config
                if reply_switch_ref.is_active() {
                    // Everyone can reply — no threadgate
                    imp.threadgate_config.replace(None);
                } else {
                    let mut rules = Vec::new();
                    if following_check_ref.is_active() {
                        rules.push(crate::atproto::ThreadgateRule::FollowingRule);
                    }
                    if mention_check_ref.is_active() {
                        rules.push(crate::atproto::ThreadgateRule::MentionRule);
                    }
                    if followers_check_ref.is_active() {
                        rules.push(crate::atproto::ThreadgateRule::FollowersRule);
                    }
                    imp.threadgate_config
                        .replace(Some(crate::atproto::ThreadgateConfig {
                            allow_rules: rules,
                        }));
                }

                // Build postgate config
                if quote_switch_ref.is_active() {
                    imp.postgate_config.replace(None);
                } else {
                    imp.postgate_config
                        .replace(Some(crate::atproto::PostgateConfig {
                            disable_quoting: true,
                        }));
                }

                // Update the interaction label
                dialog.update_interaction_label();
            }

            if let Some(dlg) = int_dialog_weak.upgrade() {
                dlg.close();
            }
        });

        // "Use as default" saves to settings
        let reply_switch_ref2 = reply_switch;
        let following_check_ref2 = following_check;
        let mention_check_ref2 = mention_check;
        let followers_check_ref2 = followers_check;
        let quote_switch_ref2 = quote_switch;
        default_btn.connect_clicked(move |_| {
            let mut settings = AppSettings::load();

            if reply_switch_ref2.is_active() {
                settings.default_threadgate = None;
            } else {
                let mut rules = Vec::new();
                if following_check_ref2.is_active() {
                    rules.push(crate::atproto::ThreadgateRule::FollowingRule);
                }
                if mention_check_ref2.is_active() {
                    rules.push(crate::atproto::ThreadgateRule::MentionRule);
                }
                if followers_check_ref2.is_active() {
                    rules.push(crate::atproto::ThreadgateRule::FollowersRule);
                }
                settings.default_threadgate =
                    Some(crate::atproto::ThreadgateConfig { allow_rules: rules });
            }

            if quote_switch_ref2.is_active() {
                settings.default_postgate = None;
            } else {
                settings.default_postgate = Some(crate::atproto::PostgateConfig {
                    disable_quoting: true,
                });
            }

            let _ = settings.save();
        });

        let window = self.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
        if let Some(win) = window.as_ref() {
            int_dialog.present(Some(win));
        }
    }

    // ─── Thread composer ───

    /// Maximum posts in a thread
    const MAX_THREAD_POSTS: usize = 10;

    /// Add a new post to the thread.
    fn add_thread_post(&self) {
        let imp = self.imp();
        let total = 1 + imp.thread_posts.borrow().len(); // 1 = main post
        if total >= Self::MAX_THREAD_POSTS {
            return;
        }

        let container = match imp.thread_container.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };

        container.set_visible(true);

        let post_index = imp.thread_posts.borrow().len(); // 0-indexed within thread_posts

        // Separator with connecting line
        let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        separator.add_css_class("thread-post-separator");

        let block_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        block_box.set_margin_top(8);
        block_box.set_margin_bottom(4);

        // Header row: "Post N" label + remove button
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let post_label = gtk4::Label::new(Some(&format!("Post {}", post_index + 2)));
        post_label.add_css_class("dim-label");
        post_label.add_css_class("caption");
        post_label.set_halign(gtk4::Align::Start);
        post_label.set_hexpand(true);
        header.append(&post_label);

        let remove_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        remove_btn.add_css_class("flat");
        remove_btn.add_css_class("circular");
        remove_btn.set_tooltip_text(Some(&format!("Remove post {}", post_index + 2)));
        remove_btn.update_property(&[gtk4::accessible::Property::Label(&format!(
            "Remove post {}",
            post_index + 2
        ))]);
        let dialog_weak = self.downgrade();
        let idx = post_index;
        remove_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.remove_thread_post(idx);
            }
        });
        header.append(&remove_btn);
        block_box.append(&header);

        // Text view for this post
        let text_view = gtk4::TextView::new();
        text_view.set_wrap_mode(gtk4::WrapMode::WordChar);
        text_view.set_vexpand(false);
        text_view.set_left_margin(8);
        text_view.set_right_margin(8);
        text_view.set_top_margin(8);
        text_view.set_bottom_margin(8);
        text_view.add_css_class("compose-text");
        text_view.update_property(&[gtk4::accessible::Property::Label(&format!(
            "Post {} content",
            post_index + 2
        ))]);

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_min_content_height(80);
        scrolled.set_child(Some(&text_view));
        block_box.append(&scrolled);

        // Image strip for this post
        let image_strip = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        image_strip.add_css_class("compose-image-strip");
        image_strip.set_visible(false);
        block_box.append(&image_strip);

        // Per-post action row: "Add Content Warning..." + "Remove All Images"
        let action_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        action_row.set_visible(false); // Hidden until images are attached

        let cw_button = gtk4::Button::with_label("Add Content Warning\u{2026}");
        cw_button.add_css_class("flat");
        cw_button.add_css_class("caption");
        cw_button.set_tooltip_text(Some("Set content warning for attached media"));
        cw_button.update_property(&[gtk4::accessible::Property::Label(
            "Set content warning for attached media",
        )]);
        action_row.append(&cw_button);

        let remove_all_button = gtk4::Button::with_label("Remove All Images");
        remove_all_button.add_css_class("flat");
        remove_all_button.add_css_class("caption");
        remove_all_button.add_css_class("destructive-action");
        remove_all_button.set_tooltip_text(Some("Remove all attached images"));
        action_row.append(&remove_all_button);

        block_box.append(&action_row);

        // Wire up CW button for this thread post
        let dialog_weak = self.downgrade();
        let pi = post_index;
        cw_button.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.show_thread_content_warning_dialog(pi);
            }
        });

        // Wire up remove all images button for this thread post
        let dialog_weak = self.downgrade();
        let pi = post_index;
        remove_all_button.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.remove_all_thread_images(pi);
            }
        });

        // Character counter — right-aligned
        let char_counter = gtk4::Label::new(Some(&MAX_GRAPHEMES.to_string()));
        char_counter.set_halign(gtk4::Align::End);
        char_counter.add_css_class("dim-label");
        char_counter.add_css_class("caption");
        char_counter.add_css_class("char-counter");
        block_box.append(&char_counter);

        // Wire up text change handler for char counter
        let dialog_weak = self.downgrade();
        let idx = post_index;
        text_view.buffer().connect_changed(move |buf| {
            let Some(dialog) = dialog_weak.upgrade() else {
                return;
            };
            dialog.update_thread_post_counter(idx, buf);
        });

        // Track focus on this thread post text view
        let dialog_weak = self.downgrade();
        let focus_idx = post_index + 1; // 0 = main post, 1+ = thread posts
        let focus_controller = gtk4::EventControllerFocus::new();
        focus_controller.connect_enter(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.imp().focused_post_index.set(focus_idx);
            }
        });
        text_view.add_controller(focus_controller);

        // Append to container
        container.append(&separator);
        container.append(&block_box);

        let block = ThreadPostBlock {
            container: block_box,
            text_view,
            image_strip,
            images: Vec::new(),
            char_counter,
            content_warning: None,
            cw_button,
            remove_all_button,
        };

        imp.thread_posts.borrow_mut().push(block);
        self.update_post_button_label();
        self.update_add_thread_button();

        // Expand dialog height for the first 2 thread posts, then let scroll take over
        let thread_count = imp.thread_posts.borrow().len();
        if thread_count <= 2 {
            // Each thread post adds roughly 120px of content
            let new_height = 360 + (thread_count as i32 * 120);
            self.set_content_height(new_height);
        }
    }

    /// Remove a thread post by index.
    fn remove_thread_post(&self, index: usize) {
        let imp = self.imp();
        let container = match imp.thread_container.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };

        // Remove UI elements
        {
            let posts = imp.thread_posts.borrow();
            if let Some(block) = posts.get(index) {
                // Remove the block container and its preceding separator
                // We need to find the separator before this block
                let block_widget = block.container.clone();
                if let Some(prev) = block_widget.prev_sibling() {
                    container.remove(&prev); // separator
                }
                container.remove(&block_widget);
            }
        }

        imp.thread_posts.borrow_mut().remove(index);

        // Re-number remaining posts
        self.renumber_thread_posts();

        if imp.thread_posts.borrow().is_empty() {
            container.set_visible(false);
        }

        self.update_post_button_label();
        self.update_add_thread_button();

        // Shrink dialog height when thread posts are removed
        let thread_count = imp.thread_posts.borrow().len();
        if thread_count <= 2 {
            let new_height = 360 + (thread_count as i32 * 120);
            self.set_content_height(new_height);
        }
    }

    /// Re-number thread post headers after removal.
    fn renumber_thread_posts(&self) {
        let imp = self.imp();
        let posts = imp.thread_posts.borrow();
        for (i, block) in posts.iter().enumerate() {
            // Find the header label (first child of container -> first child of header box)
            if let Some(header) = block.container.first_child()
                && let Some(label) = header.first_child()
                && let Some(label) = label.downcast_ref::<gtk4::Label>()
            {
                label.set_text(&format!("Post {}", i + 2));
            }
        }
    }

    /// Update the Post button label to reflect thread count.
    fn update_post_button_label(&self) {
        let imp = self.imp();
        let thread_count = 1 + imp.thread_posts.borrow().len();
        if let Some(btn) = imp.post_button.borrow().as_ref() {
            if thread_count > 1 {
                btn.set_label(&format!("Post Thread ({} posts)", thread_count));
            } else {
                btn.set_label("Post");
            }
        }
    }

    /// Update the "Add to thread" button state.
    fn update_add_thread_button(&self) {
        let imp = self.imp();
        let total = 1 + imp.thread_posts.borrow().len();
        if let Some(btn) = imp.add_thread_button.borrow().as_ref() {
            btn.set_sensitive(total < Self::MAX_THREAD_POSTS);
        }
    }

    /// Update char counter for a thread post.
    fn update_thread_post_counter(&self, index: usize, buffer: &gtk4::TextBuffer) {
        let imp = self.imp();
        let posts = imp.thread_posts.borrow();
        let Some(block) = posts.get(index) else {
            return;
        };

        let text = {
            let start = buffer.start_iter();
            let end = buffer.end_iter();
            buffer.text(&start, &end, false).to_string()
        };
        let grapheme_count = text.graphemes(true).count() as i32;
        let remaining = MAX_GRAPHEMES - grapheme_count;

        block.char_counter.set_text(&remaining.to_string());

        // Style
        if remaining < 0 {
            block.char_counter.remove_css_class("dim-label");
            block.char_counter.remove_css_class("char-counter-warn");
            block.char_counter.add_css_class("char-counter-over");
        } else if remaining <= WARN_THRESHOLD {
            block.char_counter.remove_css_class("dim-label");
            block.char_counter.remove_css_class("char-counter-over");
            block.char_counter.add_css_class("char-counter-warn");
        } else {
            block.char_counter.remove_css_class("char-counter-warn");
            block.char_counter.remove_css_class("char-counter-over");
            block.char_counter.add_css_class("dim-label");
        }

        // Update main post button state
        self.update_thread_post_button_state();
    }

    /// Recheck whether the Post button should be enabled based on all thread posts.
    fn update_thread_post_button_state(&self) {
        let imp = self.imp();

        // Check main post
        let main_ok = imp
            .text_view
            .borrow()
            .as_ref()
            .map(|tv| {
                let buffer = tv.buffer();
                let text = {
                    let start = buffer.start_iter();
                    let end = buffer.end_iter();
                    buffer.text(&start, &end, false).to_string()
                };
                let count = text.graphemes(true).count() as i32;
                let has_images = !imp.images.borrow().is_empty();
                (count > 0 || has_images) && count <= MAX_GRAPHEMES
            })
            .unwrap_or(false);

        // Check thread posts
        let thread_ok = imp.thread_posts.borrow().iter().all(|block| {
            let buffer = block.text_view.buffer();
            let text = {
                let start = buffer.start_iter();
                let end = buffer.end_iter();
                buffer.text(&start, &end, false).to_string()
            };
            let count = text.graphemes(true).count() as i32;
            let has_images = !block.images.is_empty();
            (count > 0 || has_images) && count <= MAX_GRAPHEMES
        });

        if let Some(btn) = imp.post_button.borrow().as_ref() {
            btn.set_sensitive(main_ok && thread_ok);
        }
    }

    /// Open file chooser for a thread post's image attachment.
    fn open_thread_image_chooser(&self, post_index: usize) {
        let image_count = {
            let posts = self.imp().thread_posts.borrow();
            posts
                .get(post_index)
                .map(|b| b.images.len())
                .unwrap_or(MAX_IMAGES)
        };
        if image_count >= MAX_IMAGES {
            return;
        }

        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("Images"));
        filter.add_mime_type("image/jpeg");
        filter.add_mime_type("image/png");
        filter.add_mime_type("image/gif");
        filter.add_mime_type("image/webp");

        let window = self.root().and_then(|r| r.downcast::<gtk4::Window>().ok());

        let chooser = gtk4::FileChooserNative::new(
            Some("Select Image"),
            window.as_ref(),
            gtk4::FileChooserAction::Open,
            Some("Open"),
            Some("Cancel"),
        );
        chooser.add_filter(&filter);

        let dialog_weak = self.downgrade();
        chooser.connect_response(move |chooser, response| {
            if response == gtk4::ResponseType::Accept
                && let Some(file) = chooser.file()
                && let Some(path) = file.path()
                && let Some(dialog) = dialog_weak.upgrade()
            {
                dialog.load_thread_image(post_index, &path);
            }
        });

        chooser.show();
    }

    /// Load an image for a thread post.
    fn load_thread_image(&self, post_index: usize, path: &std::path::Path) {
        let imp = self.imp();
        {
            let posts = imp.thread_posts.borrow();
            if posts
                .get(post_index)
                .map(|b| b.images.len())
                .unwrap_or(MAX_IMAGES)
                >= MAX_IMAGES
            {
                return;
            }
        }

        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => return,
        };

        let mime_type = match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
        {
            Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg".to_string(),
            Some(ext) if ext == "png" => "image/png".to_string(),
            Some(ext) if ext == "gif" => "image/gif".to_string(),
            Some(ext) if ext == "webp" => "image/webp".to_string(),
            _ => "image/jpeg".to_string(),
        };

        let (width, height) = image::image_dimensions(path).unwrap_or((1, 1));

        let texture = match gdk::Texture::from_filename(path) {
            Ok(t) => t,
            Err(_) => return,
        };

        let compose_image = ComposeImage {
            data,
            mime_type,
            alt_text: String::new(),
            width,
            height,
            texture,
        };

        {
            let mut posts = imp.thread_posts.borrow_mut();
            if let Some(block) = posts.get_mut(post_index) {
                block.images.push(compose_image);
            }
        }

        self.rebuild_thread_image_strip(post_index);
        self.update_thread_post_button_state();
    }

    /// Rebuild image strip for a thread post.
    fn rebuild_thread_image_strip(&self, post_index: usize) {
        let imp = self.imp();
        let posts = imp.thread_posts.borrow();
        let Some(block) = posts.get(post_index) else {
            return;
        };

        // Clear existing
        while let Some(child) = block.image_strip.first_child() {
            block.image_strip.remove(&child);
        }

        let has_images = !block.images.is_empty();

        for (i, img) in block.images.iter().enumerate() {
            let thumb_box = gtk4::Overlay::new();
            thumb_box.set_size_request(80, 80);
            thumb_box.add_css_class("compose-thumbnail");

            let picture = gtk4::Picture::new();
            picture.set_paintable(Some(&img.texture));
            picture.set_can_shrink(true);
            picture.set_size_request(80, 80);
            picture.add_css_class("compose-thumbnail-image");
            thumb_box.set_child(Some(&picture));

            // Remove button
            let remove_btn = gtk4::Button::from_icon_name("window-close-symbolic");
            remove_btn.add_css_class("circular");
            remove_btn.add_css_class("osd");
            remove_btn.set_halign(gtk4::Align::End);
            remove_btn.set_valign(gtk4::Align::Start);
            remove_btn.set_margin_top(4);
            remove_btn.set_margin_end(4);
            remove_btn.set_tooltip_text(Some(&format!("Remove image {}", i + 1)));
            let dialog_weak = self.downgrade();
            let pi = post_index;
            let ii = i;
            remove_btn.connect_clicked(move |_| {
                if let Some(dialog) = dialog_weak.upgrade() {
                    dialog.remove_thread_image(pi, ii);
                }
            });
            thumb_box.add_overlay(&remove_btn);

            // ALT badge
            if !img.alt_text.is_empty() {
                let alt_badge = gtk4::Label::new(Some("ALT"));
                alt_badge.add_css_class("compose-alt-badge");
                alt_badge.add_css_class("osd");
                alt_badge.set_halign(gtk4::Align::Start);
                alt_badge.set_valign(gtk4::Align::End);
                alt_badge.set_margin_bottom(4);
                alt_badge.set_margin_start(4);
                thumb_box.add_overlay(&alt_badge);
            }

            // Click to edit alt text
            let click = gtk4::GestureClick::new();
            let dialog_weak = self.downgrade();
            let pi = post_index;
            let ii = i;
            click.connect_released(move |gesture, _, _, _| {
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                if let Some(dialog) = dialog_weak.upgrade() {
                    dialog.show_thread_alt_text_dialog(pi, ii);
                }
            });
            picture.add_controller(click);

            block.image_strip.append(&thumb_box);
        }

        block.image_strip.set_visible(has_images);

        // Show/hide the per-post action buttons (CW + Remove All)
        block.cw_button.set_visible(has_images);
        block.remove_all_button.set_visible(has_images);
        // The action row parent is visible when either button is visible
        if let Some(parent) = block.cw_button.parent() {
            parent.set_visible(has_images);
        }

        // Update CW button label based on current content warning
        if has_images {
            if let Some(ref cw) = block.content_warning {
                let display = match cw.as_str() {
                    "sexual" => "Suggestive",
                    "nudity" => "Nudity",
                    "porn" => "Pornography",
                    "graphic-media" => "Graphic Media",
                    _ => cw.as_str(),
                };
                block
                    .cw_button
                    .set_label(&format!("Content Warning: {}", display));
                block.cw_button.add_css_class("accent");
            } else {
                block.cw_button.set_label("Add Content Warning\u{2026}");
                block.cw_button.remove_css_class("accent");
            }
        }

        // Clear CW if all images removed
        if !has_images {
            // Need to drop borrow first, then re-borrow mutably
            drop(posts);
            let mut posts = imp.thread_posts.borrow_mut();
            if let Some(block) = posts.get_mut(post_index) {
                block.content_warning = None;
            }
        }
    }

    /// Remove an image from a thread post.
    fn remove_thread_image(&self, post_index: usize, image_index: usize) {
        {
            let mut posts = self.imp().thread_posts.borrow_mut();
            if let Some(block) = posts.get_mut(post_index)
                && image_index < block.images.len()
            {
                block.images.remove(image_index);
            }
        }
        self.rebuild_thread_image_strip(post_index);
        self.update_thread_post_button_state();
    }

    /// Remove all images from a thread post.
    fn remove_all_thread_images(&self, post_index: usize) {
        {
            let mut posts = self.imp().thread_posts.borrow_mut();
            if let Some(block) = posts.get_mut(post_index) {
                block.images.clear();
                block.content_warning = None;
            }
        }
        self.rebuild_thread_image_strip(post_index);
        self.update_thread_post_button_state();
    }

    /// Show content warning dialog for a specific thread post.
    fn show_thread_content_warning_dialog(&self, post_index: usize) {
        let current_cw = {
            let posts = self.imp().thread_posts.borrow();
            posts
                .get(post_index)
                .and_then(|b| b.content_warning.clone())
                .unwrap_or_default()
        };

        let cw_dialog = adw::Dialog::new();
        cw_dialog.set_title("Content Warning");
        cw_dialog.set_content_width(400);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let done_btn = gtk4::Button::with_label("Done");
        done_btn.add_css_class("suggested-action");
        header.pack_end(&done_btn);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(12);
        content.set_margin_bottom(24);

        let group = adw::PreferencesGroup::new();

        let selected = std::rc::Rc::new(std::cell::RefCell::new(current_cw));

        let cw_options: &[(&str, &str, &str)] = &[
            (
                "",
                "None",
                "The images attached to this post are suitable for everyone",
            ),
            (
                "sexual",
                "Suggestive",
                "The images attached to this post are meant for adults",
            ),
            (
                "nudity",
                "Nudity",
                "The images attached to this post contain artistic or non-erotic nudity",
            ),
            (
                "porn",
                "Pornography",
                "The images attached to this post contain sexual activity or erotic nudity",
            ),
            (
                "graphic-media",
                "Graphic Media",
                "The images attached to this post contain disturbing content such as violence or gore",
            ),
        ];

        let mut first_check: Option<gtk4::CheckButton> = None;

        for (value, label, description) in cw_options {
            let row = adw::ActionRow::new();
            row.set_title(label);
            row.set_subtitle(description);
            row.set_activatable(true);

            let check = gtk4::CheckButton::new();
            let is_active = if value.is_empty() {
                selected.borrow().is_empty()
            } else {
                selected.borrow().as_str() == *value
            };
            check.set_active(is_active);

            if let Some(ref first) = first_check {
                check.set_group(Some(first));
            } else {
                first_check = Some(check.clone());
            }

            let selected_ref = selected.clone();
            let val = value.to_string();
            check.connect_toggled(move |cb| {
                if cb.is_active() {
                    *selected_ref.borrow_mut() = val.clone();
                }
            });

            row.add_suffix(&check);
            row.set_activatable_widget(Some(&check));

            group.add(&row);
        }

        content.append(&group.clone().upcast::<gtk4::Widget>());
        toolbar_view.set_content(Some(&content));
        cw_dialog.set_child(Some(&toolbar_view));

        let dialog_weak = self.downgrade();
        let cw_dialog_weak = cw_dialog.downgrade();
        let selected_for_done = selected;
        let pi = post_index;
        done_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                let val = selected_for_done.borrow().clone();
                let mut posts = dialog.imp().thread_posts.borrow_mut();
                if let Some(block) = posts.get_mut(pi) {
                    if val.is_empty() {
                        block.content_warning = None;
                    } else {
                        block.content_warning = Some(val);
                    }
                }
                drop(posts);
                dialog.rebuild_thread_image_strip(pi);
            }

            if let Some(dlg) = cw_dialog_weak.upgrade() {
                dlg.close();
            }
        });

        let window = self.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
        if let Some(win) = window.as_ref() {
            cw_dialog.present(Some(win));
        }
    }

    /// Show alt text dialog for a thread post image.
    fn show_thread_alt_text_dialog(&self, post_index: usize, image_index: usize) {
        let current_alt = {
            let posts = self.imp().thread_posts.borrow();
            posts
                .get(post_index)
                .and_then(|b| b.images.get(image_index))
                .map(|img| img.alt_text.clone())
                .unwrap_or_default()
        };

        let alt_dialog = adw::Dialog::new();
        alt_dialog.set_title("Add Descriptive Text");
        alt_dialog.set_content_width(360);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let done_btn = gtk4::Button::with_label("Done");
        done_btn.add_css_class("suggested-action");
        header.pack_end(&done_btn);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(12);
        content.set_margin_bottom(24);

        let label = gtk4::Label::new(Some("Describe this image"));
        label.set_halign(gtk4::Align::Start);
        label.add_css_class("heading");
        content.append(&label);

        let text_view = gtk4::TextView::new();
        text_view.set_wrap_mode(gtk4::WrapMode::WordChar);
        text_view.set_vexpand(true);
        text_view.set_left_margin(8);
        text_view.set_right_margin(8);
        text_view.set_top_margin(8);
        text_view.set_bottom_margin(8);
        text_view.add_css_class("card");
        text_view.update_property(&[gtk4::accessible::Property::Label("Image description")]);
        if !current_alt.is_empty() {
            text_view.buffer().set_text(&current_alt);
        }

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&text_view));
        content.append(&scrolled);

        toolbar.set_content(Some(&content));
        alt_dialog.set_child(Some(&toolbar));

        let dialog_weak = self.downgrade();
        let alt_dialog_weak = alt_dialog.downgrade();
        let text_view_ref = text_view.clone();
        done_btn.connect_clicked(move |_| {
            let buffer = text_view_ref.buffer();
            let start = buffer.start_iter();
            let end = buffer.end_iter();
            let alt_text = buffer
                .text(&start, &end, false)
                .to_string()
                .trim()
                .to_string();

            if let Some(dialog) = dialog_weak.upgrade() {
                {
                    let mut posts = dialog.imp().thread_posts.borrow_mut();
                    if let Some(block) = posts.get_mut(post_index)
                        && let Some(img) = block.images.get_mut(image_index)
                    {
                        img.alt_text = alt_text;
                    }
                }
                dialog.rebuild_thread_image_strip(post_index);
            }

            if let Some(dlg) = alt_dialog_weak.upgrade() {
                dlg.close();
            }
        });

        let window = self.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
        if let Some(win) = window.as_ref() {
            alt_dialog.present(Some(win));
        }
    }

    // ─── Link card preview ───

    /// Check the compose text for URLs and trigger a link card fetch if appropriate.
    fn check_for_link_card(&self, buffer: &gtk4::TextBuffer) {
        let imp = self.imp();

        // Don't show link cards when images are attached (images take precedence)
        if !imp.images.borrow().is_empty() {
            self.clear_link_preview();
            return;
        }

        let text = {
            let start = buffer.start_iter();
            let end = buffer.end_iter();
            buffer.text(&start, &end, false).to_string()
        };

        // Find the first URL in the text
        let first_url = HL_URL_RE.find(&text).map(|m| {
            m.as_str()
                .trim_end_matches(['.', ',', ';', '!', '?'])
                .to_string()
        });

        let Some(url) = first_url else {
            // No URL in text — clear any existing preview
            self.clear_link_preview();
            imp.link_preview_url.replace(None);
            imp.link_preview_dismissed.set(false);
            return;
        };

        // Check if this URL is different from what we already fetched
        let current_url = imp.link_preview_url.borrow().clone();
        if current_url.as_deref() == Some(&url) {
            // Same URL — keep existing preview (or dismissed state)
            return;
        }

        // New URL — reset dismissed state
        imp.link_preview_dismissed.set(false);
        imp.link_preview_url.replace(Some(url.clone()));

        // Debounce: wait 500ms before fetching
        let counter = imp.link_debounce_counter.get().wrapping_add(1);
        imp.link_debounce_counter.set(counter);

        let dialog_weak = self.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
            let Some(dialog) = dialog_weak.upgrade() else {
                return;
            };
            let imp = dialog.imp();
            // Only fire if no newer text changes have happened
            if imp.link_debounce_counter.get() != counter {
                return;
            }
            // Don't fetch if dismissed
            if imp.link_preview_dismissed.get() {
                return;
            }
            if let Some(cb) = imp.link_preview_fetch_callback.borrow().as_ref() {
                cb(url);
            }
        });
    }

    /// Set the fetched link card data and rebuild the preview widget.
    pub fn set_link_card_data(&self, data: LinkCardData) {
        let imp = self.imp();
        // Don't show if user dismissed or images are attached
        if imp.link_preview_dismissed.get() || !imp.images.borrow().is_empty() {
            return;
        }
        imp.link_card_data.replace(Some(data));
        self.rebuild_link_preview();
    }

    /// Rebuild the link card preview widget from current link_card_data.
    fn rebuild_link_preview(&self) {
        let imp = self.imp();
        let preview_box = match imp.link_preview_box.borrow().as_ref() {
            Some(b) => b.clone(),
            None => return,
        };

        // Clear existing children
        while let Some(child) = preview_box.first_child() {
            preview_box.remove(&child);
        }

        let data = match imp.link_card_data.borrow().as_ref() {
            Some(d) => d.clone(),
            None => {
                preview_box.set_visible(false);
                return;
            }
        };

        // Thumbnail (if available)
        if let Some((ref thumb_bytes, _)) = data.thumb {
            let bytes = glib::Bytes::from(thumb_bytes);
            if let Ok(texture) = gdk::Texture::from_bytes(&bytes) {
                let picture = gtk4::Picture::new();
                picture.set_paintable(Some(&texture));
                picture.set_can_shrink(true);
                picture.set_size_request(72, 72);
                picture.add_css_class("compose-link-thumb");
                preview_box.append(&picture);
            }
        }

        // Text info column
        let info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        info_box.set_hexpand(true);
        info_box.set_valign(gtk4::Align::Center);

        if !data.title.is_empty() {
            let title_label = gtk4::Label::new(Some(&data.title));
            title_label.set_halign(gtk4::Align::Start);
            title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            title_label.set_max_width_chars(40);
            title_label.add_css_class("heading");
            title_label.add_css_class("caption");
            info_box.append(&title_label);
        }

        if !data.description.is_empty() {
            let desc_label = gtk4::Label::new(Some(&data.description));
            desc_label.set_halign(gtk4::Align::Start);
            desc_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            desc_label.set_lines(2);
            desc_label.set_max_width_chars(40);
            desc_label.set_wrap(true);
            desc_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            desc_label.add_css_class("dim-label");
            desc_label.add_css_class("caption");
            info_box.append(&desc_label);
        }

        // Domain label
        if let Some(domain) = data
            .url
            .strip_prefix("https://")
            .or_else(|| data.url.strip_prefix("http://"))
        {
            let domain = domain.split('/').next().unwrap_or(&data.url);
            let domain_label = gtk4::Label::new(Some(domain));
            domain_label.set_halign(gtk4::Align::Start);
            domain_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            domain_label.add_css_class("dim-label");
            domain_label.add_css_class("caption");
            info_box.append(&domain_label);
        }

        preview_box.append(&info_box);

        // Remove button
        let remove_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        remove_btn.add_css_class("flat");
        remove_btn.add_css_class("circular");
        remove_btn.set_valign(gtk4::Align::Start);
        remove_btn.set_tooltip_text(Some("Remove link card"));
        remove_btn.update_property(&[gtk4::accessible::Property::Label("Remove link card")]);
        let dialog_weak = self.downgrade();
        remove_btn.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.dismiss_link_card();
            }
        });
        preview_box.append(&remove_btn);

        // Accessible label
        let a11y = format!("Link preview: {}", data.title);
        preview_box.update_property(&[gtk4::accessible::Property::Label(&a11y)]);

        preview_box.set_visible(true);
    }

    /// User dismissed the link card — hide it and don't re-fetch the same URL.
    fn dismiss_link_card(&self) {
        let imp = self.imp();
        imp.link_preview_dismissed.set(true);
        imp.link_card_data.replace(None);
        self.clear_link_preview();
    }

    /// Clear the link preview box without changing dismissed state.
    fn clear_link_preview(&self) {
        let imp = self.imp();
        if let Some(preview_box) = imp.link_preview_box.borrow().as_ref() {
            while let Some(child) = preview_box.first_child() {
                preview_box.remove(&child);
            }
            preview_box.set_visible(false);
        }
    }

    /// Build a `ComposeData` from the current dialog state.
    fn build_compose_data(&self) -> Option<ComposeData> {
        let imp = self.imp();

        let text = imp
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

        // Build image attachments
        let images: Vec<ImageAttachment> = imp
            .images
            .borrow()
            .iter()
            .map(|img| ImageAttachment {
                data: img.data.clone(),
                mime_type: img.mime_type.clone(),
                alt_text: img.alt_text.clone(),
                width: img.width,
                height: img.height,
            })
            .collect();

        // Allow posting with images even if text is empty
        if text.is_empty() && images.is_empty() {
            return None;
        }

        // Language
        let lang = imp.selected_language.borrow().clone();
        let langs = if lang.is_empty() { vec![] } else { vec![lang] };

        // Content warning
        let content_warning = imp.content_warning.borrow().clone();

        // Threadgate / postgate
        let threadgate = imp.threadgate_config.borrow().clone();
        let postgate = imp.postgate_config.borrow().clone();

        // Link card (only when no images attached — images take precedence)
        let link_card = if images.is_empty() {
            imp.link_card_data.borrow().clone()
        } else {
            None
        };

        Some(ComposeData {
            text,
            images,
            langs,
            content_warning,
            link_card,
            threadgate,
            postgate,
        })
    }

    /// Build ComposeData for all thread posts (Post 2, 3, etc.)
    fn build_thread_data(&self) -> Vec<ComposeData> {
        let imp = self.imp();
        let lang = imp.selected_language.borrow().clone();
        let langs = if lang.is_empty() { vec![] } else { vec![lang] };

        imp.thread_posts
            .borrow()
            .iter()
            .filter_map(|block| {
                let buffer = block.text_view.buffer();
                let text = {
                    let start = buffer.start_iter();
                    let end = buffer.end_iter();
                    buffer.text(&start, &end, false).to_string()
                }
                .trim()
                .to_string();

                let images: Vec<ImageAttachment> = block
                    .images
                    .iter()
                    .map(|img| ImageAttachment {
                        data: img.data.clone(),
                        mime_type: img.mime_type.clone(),
                        alt_text: img.alt_text.clone(),
                        width: img.width,
                        height: img.height,
                    })
                    .collect();

                if text.is_empty() && images.is_empty() {
                    return None;
                }

                // Each thread post has its own content warning
                let content_warning = block.content_warning.clone();

                Some(ComposeData {
                    text,
                    images,
                    langs: langs.clone(),
                    content_warning,
                    link_card: None,
                    threadgate: None, // only on root post
                    postgate: None,   // only on root post
                })
            })
            .collect()
    }

    fn emit_post(&self) {
        self.hide_mention_popover();

        let Some(data) = self.build_compose_data() else {
            return;
        };

        let imp = self.imp();
        let has_thread_posts = !imp.thread_posts.borrow().is_empty();

        if has_thread_posts {
            // Thread mode: collect all posts and call thread callback
            let mut all_posts = vec![data];
            all_posts.extend(self.build_thread_data());

            // Thread callback handles reply context on the app side
            if let Some(cb) = imp.thread_callback.borrow().as_ref() {
                cb(all_posts);
            }
        } else if let Some(ctx) = imp.reply_context.borrow().as_ref() {
            if let Some(cb) = imp.reply_callback.borrow().as_ref() {
                cb(data, ctx.uri.clone(), ctx.cid.clone());
            }
        } else if let Some(ctx) = imp.quote_context.borrow().as_ref() {
            if let Some(cb) = imp.quote_callback.borrow().as_ref() {
                cb(data, ctx.uri.clone(), ctx.cid.clone());
            }
        } else if let Some(cb) = imp.post_callback.borrow().as_ref() {
            cb(data);
        }
    }

    pub fn connect_post<F: Fn(ComposeData) + 'static>(&self, callback: F) {
        self.imp().post_callback.replace(Some(Box::new(callback)));
    }

    pub fn connect_reply<F: Fn(ComposeData, String, String) + 'static>(&self, callback: F) {
        self.imp().reply_callback.replace(Some(Box::new(callback)));
    }

    pub fn connect_quote<F: Fn(ComposeData, String, String) + 'static>(&self, callback: F) {
        self.imp().quote_callback.replace(Some(Box::new(callback)));
    }

    /// Register a callback for mention typeahead search.
    /// Called with the partial handle text (without @) when the user types @query.
    pub fn connect_mention_search<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp()
            .mention_search_callback
            .replace(Some(Box::new(callback)));
    }

    /// Register a callback for link card metadata fetching.
    /// Called with the URL string when a URL is detected in the compose text.
    pub fn connect_link_preview_fetch<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp()
            .link_preview_fetch_callback
            .replace(Some(Box::new(callback)));
    }

    /// Register a callback for posting a thread (multiple posts).
    /// Called with Vec<ComposeData> where first element is the root post.
    pub fn connect_thread<F: Fn(Vec<ComposeData>) + 'static>(&self, callback: F) {
        self.imp().thread_callback.replace(Some(Box::new(callback)));
    }

    pub fn set_loading(&self, loading: bool) {
        let imp = self.imp();
        if let Some(btn) = imp.post_button.borrow().as_ref() {
            if loading {
                // Replace button content with a spinner + "Posting…" label
                let loading_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
                loading_box.set_halign(gtk4::Align::Center);
                let spinner = gtk4::Spinner::new();
                spinner.set_spinning(true);
                loading_box.append(&spinner);
                let label = gtk4::Label::new(Some("Posting\u{2026}"));
                loading_box.append(&label);
                btn.set_child(Some(&loading_box));
                btn.set_sensitive(false);
            } else {
                // Restore normal label
                btn.set_child(None::<&gtk4::Widget>);
                self.update_post_button_label();
                btn.set_sensitive(true);
            }
        }

        // Disable/enable text editing during post
        if let Some(tv) = imp.text_view.borrow().as_ref() {
            tv.set_sensitive(!loading);
        }
        // Disable thread post text views
        for block in imp.thread_posts.borrow().iter() {
            block.text_view.set_sensitive(!loading);
        }
        // Disable add-to-thread button
        if let Some(btn) = imp.add_thread_button.borrow().as_ref() {
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
