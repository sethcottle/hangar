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
    pub images: Vec<String>,
    /// URI of the viewer's like record, if they liked this post
    pub viewer_like: Option<String>,
    /// URI of the viewer's repost record, if they reposted this post
    pub viewer_repost: Option<String>,
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
