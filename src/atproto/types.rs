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
