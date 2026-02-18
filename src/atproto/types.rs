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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalEmbed {
    pub uri: String,
    pub title: String,
    pub description: String,
    pub thumb: Option<String>,
}

/// Video embed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoEmbed {
    pub playlist: String,
    pub thumbnail: Option<String>,
    pub alt: Option<String>,
    pub aspect_ratio: Option<(u32, u32)>,
}

/// Quote post embed (embedded record view)
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageEmbed {
    pub thumb: String,
    pub fullsize: String,
    pub alt: String,
    pub aspect_ratio: Option<(u32, u32)>,
}

/// Repost attribution (who reposted this into the feed)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepostReason {
    pub by: Profile,
    pub indexed_at: String,
}

/// Reply context (who this post is replying to)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyContext {
    pub parent_author: Profile,
    pub root_author: Profile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub did: String,
    pub handle: String,
    pub display_name: Option<String>,
    pub avatar: Option<String>,
    /// Profile banner image URL
    pub banner: Option<String>,
    /// Profile bio/description
    pub description: Option<String>,
    /// Number of followers
    pub followers_count: Option<u32>,
    /// Number of accounts this user follows
    pub following_count: Option<u32>,
    /// Number of posts
    pub posts_count: Option<u32>,
    /// Whether the viewer follows this user (URI of follow record)
    pub viewer_following: Option<String>,
    /// Whether this user follows the viewer (URI of their follow record)
    pub viewer_followed_by: Option<String>,
}

impl Profile {
    /// Create a minimal profile (used when extracting from posts, etc.)
    pub fn minimal(
        did: String,
        handle: String,
        display_name: Option<String>,
        avatar: Option<String>,
    ) -> Self {
        Self {
            did,
            handle,
            display_name,
            avatar,
            banner: None,
            description: None,
            followers_count: None,
            following_count: None,
            posts_count: None,
            viewer_following: None,
            viewer_followed_by: None,
        }
    }
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

/// Notification from the AT Protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub uri: String,
    pub cid: String,
    pub author: Profile,
    /// The reason for the notification: "mention", "reply", "quote", "like", "repost", "follow"
    pub reason: String,
    /// When the notification was indexed
    pub indexed_at: String,
    /// Whether this notification has been seen
    pub is_read: bool,
    /// The post associated with this notification (for mentions, replies, quotes)
    pub post: Option<Post>,
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

// ─── Compose data types (passed from UI → client when creating posts) ───

/// Everything needed to create a post, assembled by the compose dialog.
#[derive(Debug, Clone, Default)]
pub struct ComposeData {
    pub text: String,
    /// Images to attach (up to 4)
    pub images: Vec<ImageAttachment>,
    /// BCP 47 language tags (e.g. ["en"])
    pub langs: Vec<String>,
    /// Content warning self-label value (e.g. "sexual", "nudity", "graphic-media")
    pub content_warning: Option<String>,
    /// External link card metadata (auto-detected from URLs in text)
    pub link_card: Option<LinkCardData>,
    /// Threadgate: who can reply. `None` means everyone (no threadgate record).
    pub threadgate: Option<ThreadgateConfig>,
    /// Postgate: quote controls. `None` means quoting allowed (no postgate record).
    pub postgate: Option<PostgateConfig>,
}

/// An image the user wants to attach to a post.
#[derive(Debug, Clone)]
pub struct ImageAttachment {
    pub data: Vec<u8>,
    pub mime_type: String,
    pub alt_text: String,
    pub width: u32,
    pub height: u32,
}

/// Metadata for an external link card (fetched from OG tags).
#[derive(Debug, Clone)]
pub struct LinkCardData {
    pub url: String,
    pub title: String,
    pub description: String,
    /// Thumbnail image bytes + MIME type (fetched from og:image)
    pub thumb: Option<(Vec<u8>, String)>,
}

/// Threadgate configuration — controls who can reply to a post.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadgateConfig {
    /// Which groups are allowed to reply. Empty vec = nobody can reply.
    pub allow_rules: Vec<ThreadgateRule>,
}

/// Individual threadgate allow rule.
/// Names match the AT Protocol spec (mentionRule, followingRule, followerRule).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum ThreadgateRule {
    /// Users mentioned in the post can reply
    MentionRule,
    /// Users the author follows can reply
    FollowingRule,
    /// Users who follow the author can reply
    FollowersRule,
}

/// Postgate configuration — controls quoting of a post.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PostgateConfig {
    /// If true, quoting this post is disabled.
    pub disable_quoting: bool,
}

/// A direct message conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    /// Unique conversation ID
    pub id: String,
    /// Participants in the conversation
    pub members: Vec<Profile>,
    /// Last message in the conversation (if any)
    pub last_message: Option<ChatMessage>,
    /// Number of unread messages
    pub unread_count: i64,
    /// Whether the conversation is muted
    pub muted: bool,
}

/// A chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique message ID
    pub id: String,
    /// Message text content
    pub text: String,
    /// DID of the sender
    pub sender_did: String,
    /// When the message was sent
    pub sent_at: String,
}
