// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

/// Decoupled from atrium's internal representation so we own the API boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub did: String,
    pub handle: String,
    pub access_jwt: String,
    pub refresh_jwt: String,
}

/// External link card embed (URLs with previews)
#[derive(Debug, Clone)]
pub struct ExternalEmbed {
    pub uri: String,
    pub title: String,
    pub description: String,
    pub thumb: Option<String>,
}

/// Video embed
#[derive(Debug, Clone)]
pub struct VideoEmbed {
    pub playlist: String,
    pub thumbnail: Option<String>,
    pub alt: Option<String>,
    pub aspect_ratio: Option<(u32, u32)>,
}

/// Quote post embed (embedded record view)
#[derive(Debug, Clone)]
pub struct QuoteEmbed {
    pub uri: String,
    pub cid: String,
    pub author: Profile,
    pub text: String,
    pub indexed_at: String,
    /// Nested embed within the quoted post
    pub embed: Option<Box<Embed>>,
}

/// All possible embed types for a post
#[derive(Debug, Clone)]
pub enum Embed {
    Images(Vec<ImageEmbed>),
    External(ExternalEmbed),
    Video(VideoEmbed),
    Quote(QuoteEmbed),
    /// Quote post with additional media attached
    QuoteWithMedia {
        quote: QuoteEmbed,
        media: Box<Embed>,
    },
}

/// Single image with metadata
#[derive(Debug, Clone)]
pub struct ImageEmbed {
    pub thumb: String,
    pub fullsize: String,
    pub alt: String,
    pub aspect_ratio: Option<(u32, u32)>,
}

/// Repost attribution (who reposted this into the feed)
#[derive(Debug, Clone)]
pub struct RepostReason {
    pub by: Profile,
    pub indexed_at: String,
}

/// Reply context (who this post is replying to)
#[derive(Debug, Clone)]
pub struct ReplyContext {
    pub parent_author: Profile,
    pub root_author: Profile,
}

#[derive(Debug, Clone)]
pub struct Post {
    pub uri: String,
    pub cid: String,
    pub author: Profile,
    pub text: String,
    pub created_at: String,
    pub indexed_at: String,
    pub like_count: Option<u32>,
    pub repost_count: Option<u32>,
    pub reply_count: Option<u32>,
    /// Rich embed content (images, external links, videos, quotes)
    pub embed: Option<Embed>,
    /// URI of the viewer's like record, if they liked this post
    pub viewer_like: Option<String>,
    /// URI of the viewer's repost record, if they reposted this post
    pub viewer_repost: Option<String>,
    /// Repost attribution if this appeared in feed via repost
    pub repost_reason: Option<RepostReason>,
    /// Reply context if this post is a reply
    pub reply_context: Option<ReplyContext>,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub did: String,
    pub handle: String,
    pub display_name: Option<String>,
    pub avatar: Option<String>,
}

/// Represents a feed that the user can switch to
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFeed {
    /// The type of feed: "timeline" for home, "feed" for custom generators
    pub feed_type: String,
    /// The AT-URI of the feed generator (empty for home timeline)
    pub uri: String,
    /// Display name shown in the feed selector
    pub display_name: String,
    /// Description of what the feed contains
    pub description: Option<String>,
    /// Whether this feed is pinned (shown in selector)
    pub pinned: bool,
}

impl SavedFeed {
    /// Create the default "Following" (home timeline) feed
    pub fn home() -> Self {
        Self {
            feed_type: "timeline".to_string(),
            uri: String::new(),
            display_name: "Following".to_string(),
            description: Some("A feed of content from the people you follow".to_string()),
            pinned: true,
        }
    }

    /// Check if this is the home timeline
    pub fn is_home(&self) -> bool {
        self.feed_type == "timeline" || self.uri.is_empty()
    }
}
