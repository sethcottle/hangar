// SPDX-License-Identifier: MPL-2.0

//! Linkified label text for posts and profile bios.
//!
//! The lexicon has no facets for bios; `getProfile` returns a bare
//! description string and every client finds the links itself. The span
//! detection and escaping rules live here once; `PostRow::format_post_text`
//! delegates to [`linkify`].

use gtk4::glib;

/// One linkified span, as byte offsets into the raw text.
fn text_links(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    use std::sync::LazyLock;

    static URL: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"https?://[^\s<>\[\]{}|\\^`\x00-\x1f\x7f]+").unwrap());
    static MENTION: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"@([a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z]([a-zA-Z0-9-]*[a-zA-Z0-9])?",
        )
        .unwrap()
    });
    // Bare domains, matched after mentions so that a handle ending in
    // `.social` is a mention rather than a link to a website.
    static BARE_URL: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"\b([a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+(?:com|org|net|io|co|app|dev|edu|gov|me|info|biz|social)[/a-zA-Z0-9._~:/?#@!$&'()*+,;=-]*",
        )
        .unwrap()
    });
    static HASHTAG: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"#[a-zA-Z][a-zA-Z0-9_]*").unwrap());

    let mut links: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut claim = |range: std::ops::Range<usize>, href: String| {
        if !links
            .iter()
            .any(|(taken, _)| range.start < taken.end && range.end > taken.start)
        {
            links.push((range, href));
        }
    };

    for m in URL.find_iter(text) {
        claim(m.range(), m.as_str().to_string());
    }
    for m in MENTION.find_iter(text) {
        claim(m.range(), format!("bsky-mention://{}", &m.as_str()[1..]));
    }
    for m in BARE_URL.find_iter(text) {
        claim(m.range(), format!("https://{}", m.as_str()));
    }
    for m in HASHTAG.find_iter(text) {
        claim(m.range(), format!("bsky-tag://{}", &m.as_str()[1..]));
    }

    links.sort_by_key(|(range, _)| range.start);
    links
}

/// Bio text as Pango markup with clickable spans. URLs become `<a>` tags;
/// mentions use the `bsky-mention://` scheme and hashtags `bsky-tag://`,
/// both handled in the label's `activate-link` handler.
///
/// Spans are found in the raw text and each piece is escaped on the way
/// out. Escaping first lets a pattern match inside an entity the escaper
/// just produced, Pango then rejects the string, and a bio with `&` or `<`
/// comes up blank. Post text had exactly that bug.
pub(crate) fn linkify(text: &str) -> String {
    let links = text_links(text);
    let mut out = String::with_capacity(text.len() + links.len() * 48);
    let mut cursor = 0;

    for (range, href) in &links {
        out.push_str(&glib::markup_escape_text(&text[cursor..range.start]));
        out.push_str("<a href=\"");
        // The href is wire text too.
        out.push_str(&glib::markup_escape_text(href));
        out.push_str("\">");
        out.push_str(&glib::markup_escape_text(&text[range.clone()]));
        out.push_str("</a>");
        cursor = range.end;
    }
    out.push_str(&glib::markup_escape_text(&text[cursor..]));

    out
}

/// Set bio text on `label`, linkified, with a plain-text fallback.
///
/// `gtk_label_set_markup` on markup it cannot parse emits a `Gtk-WARNING`
/// and changes nothing, so the label keeps whatever it showed before.
/// Clear, set markup, and if nothing arrived set it unformatted.
pub(crate) fn set_linkified(label: &gtk4::Label, text: &str) {
    let markup = linkify(text);
    label.set_text("");
    label.set_markup(&markup);
    if label.text().is_empty() && !text.is_empty() {
        eprintln!(
            "hangar: bio text could not be marked up; showing it unformatted.\n  \
             text: {text:?}\n  markup: {markup:?}"
        );
        label.set_text(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hostile characters must come out escaped, so the markup parses and
    /// the label shows the bio instead of going blank.
    #[test]
    fn hostile_bios_escape_cleanly() {
        assert_eq!(linkify("a & b"), "a &amp; b");
        assert_eq!(linkify("<b>bold</b>"), "&lt;b&gt;bold&lt;/b&gt;");
        // A pre-escaped entity is data here and gets escaped again.
        assert_eq!(linkify("&amp;"), "&amp;amp;");
        // The control-character case that once blanked post text: GLib
        // escapes U+001F as `&#x1f;` and the hashtag pattern must not eat
        // `#x1f` out of the middle of it.
        assert_eq!(linkify("\u{1f}"), "&#x1f;");

        // Everything without an `<a>` span has to satisfy Pango itself.
        // parse_markup rejects `<a>`, a GtkLabel extension, so anything
        // that linkifies is checked in the GTK test below instead. That
        // includes a literal `&#x1f;`, where `#x1f` is a valid hashtag.
        for text in ["&", "<", ">", "\"", "&&&###@@@", "#", "\u{1f}"] {
            let markup = linkify(text);
            assert!(
                gtk4::pango::parse_markup(&markup, '\0').is_ok(),
                "{text:?} produced markup Pango rejects: {markup:?}"
            );
        }
    }

    #[test]
    fn bios_linkify_urls_mentions_and_tags() {
        let markup =
            linkify("hi @user.bsky.social see https://example.com and example.org too #rust");
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

        // An `&` inside a URL is escaped in the href as well as in the
        // body, and the two agree.
        assert_eq!(
            linkify("https://example.com/?a=1&b=2"),
            r#"<a href="https://example.com/?a=1&amp;b=2">https://example.com/?a=1&amp;b=2</a>"#
        );
    }

    /// The label-level guarantee: hostile bios render as text, linked bios
    /// keep their markup.
    #[test]
    fn set_linkified_never_blanks_the_label() {
        crate::ui::with_gtk(set_linkified_never_blanks_the_label_body);
    }

    fn set_linkified_never_blanks_the_label_body() {
        let label = gtk4::Label::new(None);
        label.set_use_markup(true);

        for text in [
            "plain bio",
            "bio with a link https://example.com & an ampersand",
            "@someone.bsky.social <3 #rust",
            "&",
            "<",
            "&#x1f;",
            "&&&###@@@",
        ] {
            set_linkified(&label, text);
            assert_eq!(
                label.text().as_str(),
                text,
                "the label must show the bio, not an empty string\n  markup: {}",
                linkify(text)
            );
            // `use-markup` still on means the markup was accepted rather
            // than having gone down the unformatted fallback.
            assert!(
                label.uses_markup(),
                "{text:?} fell back to plain text; the markup was rejected\n  markup: {}",
                linkify(text)
            );
        }
    }
}
