// SPDX-License-Identifier: MPL-2.0

//! Rich text facet detection for AT Protocol posts.
//!
//! Detects mentions (@handle), links (URLs), and hashtags (#tag) in post text,
//! computes UTF-8 byte offsets, and builds the JSON facets array for inclusion
//! in post records.

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// A detected span in the post text, before network resolution.
pub enum RawFacet {
    Link {
        byte_start: usize,
        byte_end: usize,
        uri: String,
    },
    Mention {
        byte_start: usize,
        byte_end: usize,
        handle: String,
    },
    Tag {
        byte_start: usize,
        byte_end: usize,
        tag: String,
    },
}

impl RawFacet {
    fn byte_range(&self) -> (usize, usize) {
        match self {
            RawFacet::Link {
                byte_start,
                byte_end,
                ..
            } => (*byte_start, *byte_end),
            RawFacet::Mention {
                byte_start,
                byte_end,
                ..
            } => (*byte_start, *byte_end),
            RawFacet::Tag {
                byte_start,
                byte_end,
                ..
            } => (*byte_start, *byte_end),
        }
    }
}

// Compile regexes once.
static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://[^\s<>\[\]\{}|\\^`\x00-\x1f\x7f]+").unwrap());

static MENTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s\(\[])(@(([a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z]([a-zA-Z0-9-]*[a-zA-Z0-9])?))")
        .unwrap()
});

static HASHTAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s\(\[])#([a-zA-Z][a-zA-Z0-9_]*)").unwrap());

/// Check if a byte range overlaps with any existing facet.
fn overlaps(byte_start: usize, byte_end: usize, existing: &[RawFacet]) -> bool {
    existing.iter().any(|f| {
        let (fs, fe) = f.byte_range();
        byte_start < fe && byte_end > fs
    })
}

/// Trim trailing punctuation that is likely sentence-ending, not part of the URL.
fn trim_url_trailing(url: &str) -> &str {
    url.trim_end_matches(|c| matches!(c, '.' | ',' | ';' | '!' | '?'))
}

/// Parse all facets from post text. Pure text processing, no network calls.
///
/// Detection order (for overlap prevention):
/// 1. URLs (highest priority)
/// 2. Mentions (skip if overlapping a URL)
/// 3. Hashtags (skip if overlapping a URL or mention)
pub fn parse_facets(text: &str) -> Vec<RawFacet> {
    let mut facets = Vec::new();

    // 1. URLs
    for m in URL_RE.find_iter(text) {
        let trimmed = trim_url_trailing(m.as_str());
        let byte_end = m.start() + trimmed.len();
        facets.push(RawFacet::Link {
            byte_start: m.start(),
            byte_end,
            uri: trimmed.to_string(),
        });
    }

    // 2. Mentions — the regex captures a leading boundary character, so we
    //    use capture group 1 for the @handle portion and group 2 for the
    //    bare handle (without @).
    for caps in MENTION_RE.captures_iter(text) {
        let at_handle = caps.get(1).unwrap(); // "@handle.domain"
        let handle = caps.get(2).unwrap(); // "handle.domain"
        let byte_start = at_handle.start();
        let byte_end = at_handle.end();

        if !overlaps(byte_start, byte_end, &facets) {
            facets.push(RawFacet::Mention {
                byte_start,
                byte_end,
                handle: handle.as_str().to_string(),
            });
        }
    }

    // 3. Hashtags — regex captures a leading boundary, so we find the '#'
    //    position within the match and use group 1 for the tag name.
    for caps in HASHTAG_RE.captures_iter(text) {
        let full_match = caps.get(0).unwrap();
        let tag = caps.get(1).unwrap();

        // Find the '#' within the match (may be preceded by whitespace/bracket)
        let hash_offset = text[full_match.start()..].find('#').unwrap_or(0);
        let byte_start = full_match.start() + hash_offset;
        let byte_end = tag.end();

        if !overlaps(byte_start, byte_end, &facets) {
            facets.push(RawFacet::Tag {
                byte_start,
                byte_end,
                tag: tag.as_str().to_string(),
            });
        }
    }

    facets
}

/// Convert parsed facets into a JSON array for inclusion in a post record.
///
/// Mentions are only included if their handle was successfully resolved to a DID
/// (present in `resolved_dids`). Unresolved mentions are silently dropped.
pub fn build_facets_json(
    raw_facets: &[RawFacet],
    resolved_dids: &HashMap<String, String>,
) -> serde_json::Value {
    let facets: Vec<serde_json::Value> = raw_facets
        .iter()
        .filter_map(|f| match f {
            RawFacet::Link {
                byte_start,
                byte_end,
                uri,
            } => Some(serde_json::json!({
                "index": { "byteStart": byte_start, "byteEnd": byte_end },
                "features": [{
                    "$type": "app.bsky.richtext.facet#link",
                    "uri": uri
                }]
            })),
            RawFacet::Mention {
                byte_start,
                byte_end,
                handle,
            } => {
                // Only include if the handle was resolved to a DID
                resolved_dids.get(handle).map(|did| {
                    serde_json::json!({
                        "index": { "byteStart": byte_start, "byteEnd": byte_end },
                        "features": [{
                            "$type": "app.bsky.richtext.facet#mention",
                            "did": did
                        }]
                    })
                })
            }
            RawFacet::Tag {
                byte_start,
                byte_end,
                tag,
            } => Some(serde_json::json!({
                "index": { "byteStart": byte_start, "byteEnd": byte_end },
                "features": [{
                    "$type": "app.bsky.richtext.facet#tag",
                    "tag": tag
                }]
            })),
        })
        .collect();

    serde_json::Value::Array(facets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_no_facets() {
        let facets = parse_facets("Hello world, no links here");
        assert!(facets.is_empty());
    }

    #[test]
    fn test_url_detection() {
        let facets = parse_facets("Check out https://example.com today");
        assert_eq!(facets.len(), 1);
        let (start, end) = facets[0].byte_range();
        assert_eq!(
            &"Check out https://example.com today"[start..end],
            "https://example.com"
        );
    }

    #[test]
    fn test_url_trailing_punctuation_trimmed() {
        let text = "Visit https://example.com.";
        let facets = parse_facets(text);
        assert_eq!(facets.len(), 1);
        if let RawFacet::Link { uri, .. } = &facets[0] {
            assert_eq!(uri, "https://example.com");
        } else {
            panic!("expected Link facet");
        }
    }

    #[test]
    fn test_mention_detection() {
        let text = "Hello @user.bsky.social how are you";
        let facets = parse_facets(text);
        assert_eq!(facets.len(), 1);
        if let RawFacet::Mention {
            handle,
            byte_start,
            byte_end,
            ..
        } = &facets[0]
        {
            assert_eq!(handle, "user.bsky.social");
            assert_eq!(&text[*byte_start..*byte_end], "@user.bsky.social");
        } else {
            panic!("expected Mention facet");
        }
    }

    #[test]
    fn test_mention_at_start() {
        let text = "@user.bsky.social hello";
        let facets = parse_facets(text);
        assert_eq!(facets.len(), 1);
        if let RawFacet::Mention { handle, .. } = &facets[0] {
            assert_eq!(handle, "user.bsky.social");
        } else {
            panic!("expected Mention facet");
        }
    }

    #[test]
    fn test_hashtag_detection() {
        let text = "Loving #rust today";
        let facets = parse_facets(text);
        assert_eq!(facets.len(), 1);
        if let RawFacet::Tag {
            tag,
            byte_start,
            byte_end,
            ..
        } = &facets[0]
        {
            assert_eq!(tag, "rust");
            assert_eq!(&text[*byte_start..*byte_end], "#rust");
        } else {
            panic!("expected Tag facet");
        }
    }

    #[test]
    fn test_hashtag_at_start() {
        let text = "#hello world";
        let facets = parse_facets(text);
        assert_eq!(facets.len(), 1);
        if let RawFacet::Tag { tag, .. } = &facets[0] {
            assert_eq!(tag, "hello");
        } else {
            panic!("expected Tag facet");
        }
    }

    #[test]
    fn test_multiple_facets() {
        let text = "Hey @user.bsky.social check https://example.com #cool";
        let facets = parse_facets(text);
        assert_eq!(facets.len(), 3);
    }

    #[test]
    fn test_url_takes_priority_over_mention_overlap() {
        // A URL containing an @ should not also produce a mention
        let text = "See https://example.com/@user.bsky.social/post";
        let facets = parse_facets(text);
        // Should only have the URL, not a mention
        assert_eq!(facets.len(), 1);
        assert!(matches!(facets[0], RawFacet::Link { .. }));
    }

    #[test]
    fn test_build_json_with_resolved_mentions() {
        let facets = vec![
            RawFacet::Link {
                byte_start: 0,
                byte_end: 19,
                uri: "https://example.com".to_string(),
            },
            RawFacet::Mention {
                byte_start: 20,
                byte_end: 37,
                handle: "user.bsky.social".to_string(),
            },
        ];
        let mut dids = HashMap::new();
        dids.insert("user.bsky.social".to_string(), "did:plc:1234".to_string());

        let json = build_facets_json(&facets, &dids);
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn test_build_json_drops_unresolved_mentions() {
        let facets = vec![RawFacet::Mention {
            byte_start: 0,
            byte_end: 17,
            handle: "nobody.bsky.social".to_string(),
        }];
        let dids = HashMap::new(); // No resolutions

        let json = build_facets_json(&facets, &dids);
        let arr = json.as_array().unwrap();
        assert!(arr.is_empty());
    }

    #[test]
    fn test_unicode_byte_offsets() {
        // Emoji are multi-byte in UTF-8: each basic emoji is 4 bytes
        let text = "\u{1F600} @user.bsky.social";
        let facets = parse_facets(text);
        assert_eq!(facets.len(), 1);
        if let RawFacet::Mention {
            byte_start,
            byte_end,
            ..
        } = &facets[0]
        {
            // The emoji is 4 bytes + 1 space = offset 5
            assert_eq!(*byte_start, 5);
            assert_eq!(&text[*byte_start..*byte_end], "@user.bsky.social");
        } else {
            panic!("expected Mention facet");
        }
    }
}
