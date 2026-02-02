// SPDX-License-Identifier: MPL-2.0

/// SQL schema for the cache database
pub const SCHEMA: &str = r#"
-- Database version for migrations
PRAGMA user_version = 1;

-- profiles: DID-keyed, minimal vs full
CREATE TABLE IF NOT EXISTS profiles (
    did TEXT PRIMARY KEY,
    handle TEXT NOT NULL,
    display_name TEXT,
    avatar TEXT,
    banner TEXT,
    description TEXT,
    followers_count INTEGER,
    following_count INTEGER,
    posts_count INTEGER,
    viewer_following TEXT,
    viewer_followed_by TEXT,
    fetched_at INTEGER NOT NULL,
    is_full INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_profiles_handle ON profiles(handle);

-- posts: Core post data + JSON for complex fields
CREATE TABLE IF NOT EXISTS posts (
    uri TEXT PRIMARY KEY,
    cid TEXT NOT NULL,
    author_did TEXT NOT NULL,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL,
    indexed_at TEXT NOT NULL,
    like_count INTEGER,
    repost_count INTEGER,
    reply_count INTEGER,
    embed_json TEXT,
    repost_reason_json TEXT,
    reply_context_json TEXT,
    viewer_like TEXT,
    viewer_repost TEXT,
    fetched_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_posts_author ON posts(author_did);
CREATE INDEX IF NOT EXISTS idx_posts_indexed_at ON posts(indexed_at DESC);
CREATE INDEX IF NOT EXISTS idx_posts_fetched_at ON posts(fetched_at);

-- feed_items: Maps posts to feed positions
CREATE TABLE IF NOT EXISTS feed_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    feed_key TEXT NOT NULL,
    post_uri TEXT NOT NULL,
    position INTEGER NOT NULL,
    sort_timestamp TEXT NOT NULL,
    fetched_at INTEGER NOT NULL,
    UNIQUE(feed_key, post_uri)
);

CREATE INDEX IF NOT EXISTS idx_feed_items_feed_key ON feed_items(feed_key, position);
CREATE INDEX IF NOT EXISTS idx_feed_items_sort ON feed_items(feed_key, sort_timestamp DESC);

-- feed_state: Cursor and anchor per feed
CREATE TABLE IF NOT EXISTS feed_state (
    feed_key TEXT PRIMARY KEY,
    oldest_cursor TEXT,
    has_more INTEGER NOT NULL DEFAULT 1,
    newest_post_uri TEXT,
    newest_sort_timestamp TEXT,
    last_refresh_at INTEGER
);

-- notifications: Cached notifications
CREATE TABLE IF NOT EXISTS notifications (
    uri TEXT PRIMARY KEY,
    cid TEXT NOT NULL,
    author_did TEXT NOT NULL,
    reason TEXT NOT NULL,
    indexed_at TEXT NOT NULL,
    is_read INTEGER NOT NULL DEFAULT 0,
    post_json TEXT,
    fetched_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_notifications_indexed ON notifications(indexed_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_reason ON notifications(reason);
"#;
