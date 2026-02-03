// SPDX-License-Identifier: MPL-2.0

use crate::atproto::{Embed, Post, Profile, ReplyContext, RepostReason};
use crate::cache::{CacheDb, CacheError, PostCache};
use rusqlite::params;

/// Feed key for the home timeline
#[allow(dead_code)]
pub const FEED_HOME: &str = "home";

/// State of a feed's pagination and anchor
#[derive(Debug, Clone, Default)]
pub struct FeedState {
    /// Cursor for loading older posts
    pub oldest_cursor: Option<String>,
    /// Whether there are more posts to load
    pub has_more: bool,
    /// URI of the newest post (anchor for new post detection)
    pub newest_post_uri: Option<String>,
    /// Sort timestamp of the newest post
    pub newest_sort_timestamp: Option<String>,
    /// When we last refreshed this feed
    pub last_refresh_at: Option<i64>,
}

/// Cache operations for feeds
pub struct FeedCache<'a> {
    db: &'a CacheDb,
}

impl<'a> FeedCache<'a> {
    pub fn new(db: &'a CacheDb) -> Self {
        Self { db }
    }

    /// Store a page of posts in a feed
    /// `start_position` is the position of the first post in this page
    pub fn store_page(
        &self,
        feed_key: &str,
        posts: &[Post],
        start_position: i64,
    ) -> Result<(), CacheError> {
        // First store all posts
        let post_cache = PostCache::new(self.db);
        post_cache.store_batch(posts)?;

        // Then create feed item entries
        let mut conn = self.db.conn();
        let tx = conn.transaction()?;
        let now = CacheDb::now();

        for (i, post) in posts.iter().enumerate() {
            let position = start_position + i as i64;
            // Use indexed_at as sort timestamp, or repost time if this is a repost
            let sort_timestamp = post
                .repost_reason
                .as_ref()
                .map(|r| r.indexed_at.clone())
                .unwrap_or_else(|| post.indexed_at.clone());

            tx.execute(
                r#"
                INSERT INTO feed_items (feed_key, post_uri, position, sort_timestamp, fetched_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(feed_key, post_uri) DO UPDATE SET
                    position = excluded.position,
                    sort_timestamp = excluded.sort_timestamp,
                    fetched_at = excluded.fetched_at
                "#,
                params![feed_key, post.uri, position, sort_timestamp, now],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Get a page of cached posts from a feed
    pub fn get_page(
        &self,
        feed_key: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Post>, CacheError> {
        let conn = self.db.conn();

        let mut stmt = conn.prepare(
            r#"
            SELECT
                p.uri, p.cid, p.author_did, p.text, p.created_at, p.indexed_at,
                p.like_count, p.repost_count, p.reply_count,
                p.embed_json, p.repost_reason_json, p.reply_context_json,
                p.viewer_like, p.viewer_repost,
                pr.handle, pr.display_name, pr.avatar
            FROM feed_items fi
            JOIN posts p ON fi.post_uri = p.uri
            LEFT JOIN profiles pr ON p.author_did = pr.did
            WHERE fi.feed_key = ?
            ORDER BY fi.position ASC
            LIMIT ? OFFSET ?
            "#,
        )?;

        let mut rows = stmt.query(params![feed_key, limit as i64, offset as i64])?;
        let mut posts = Vec::new();

        while let Some(row) = rows.next()? {
            posts.push(Self::row_to_post(row)?);
        }

        Ok(posts)
    }

    /// Get feed state (cursor, anchor)
    pub fn get_state(&self, feed_key: &str) -> Result<FeedState, CacheError> {
        let conn = self.db.conn();

        let mut stmt = conn.prepare(
            r#"
            SELECT oldest_cursor, has_more, newest_post_uri, newest_sort_timestamp, last_refresh_at
            FROM feed_state
            WHERE feed_key = ?
            "#,
        )?;

        let state = stmt
            .query_row([feed_key], |row| {
                Ok(FeedState {
                    oldest_cursor: row.get(0)?,
                    has_more: row.get::<_, i32>(1)? != 0,
                    newest_post_uri: row.get(2)?,
                    newest_sort_timestamp: row.get(3)?,
                    last_refresh_at: row.get(4)?,
                })
            })
            .unwrap_or_default();

        Ok(state)
    }

    /// Update feed state
    pub fn set_state(&self, feed_key: &str, state: &FeedState) -> Result<(), CacheError> {
        let conn = self.db.conn();

        conn.execute(
            r#"
            INSERT INTO feed_state (
                feed_key, oldest_cursor, has_more, newest_post_uri, newest_sort_timestamp, last_refresh_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(feed_key) DO UPDATE SET
                oldest_cursor = excluded.oldest_cursor,
                has_more = excluded.has_more,
                newest_post_uri = excluded.newest_post_uri,
                newest_sort_timestamp = excluded.newest_sort_timestamp,
                last_refresh_at = excluded.last_refresh_at
            "#,
            params![
                feed_key,
                state.oldest_cursor,
                state.has_more as i32,
                state.newest_post_uri,
                state.newest_sort_timestamp,
                state.last_refresh_at,
            ],
        )?;

        Ok(())
    }

    /// Clear a feed (on switch or full refresh)
    pub fn clear_feed(&self, feed_key: &str) -> Result<(), CacheError> {
        let conn = self.db.conn();

        conn.execute("DELETE FROM feed_items WHERE feed_key = ?", [feed_key])?;
        conn.execute("DELETE FROM feed_state WHERE feed_key = ?", [feed_key])?;

        Ok(())
    }

    /// Get count of cached items for feed
    pub fn count(&self, feed_key: &str) -> Result<usize, CacheError> {
        let conn = self.db.conn();

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM feed_items WHERE feed_key = ?",
            [feed_key],
            |row| row.get(0),
        )?;

        Ok(count as usize)
    }

    /// Check if the feed was refreshed recently
    #[allow(dead_code)]
    pub fn is_fresh(&self, feed_key: &str, max_age_secs: i64) -> bool {
        if let Ok(state) = self.get_state(feed_key)
            && let Some(last_refresh) = state.last_refresh_at
        {
            let now = CacheDb::now();
            return (now - last_refresh) < max_age_secs;
        }
        false
    }

    /// Convert a database row to a Post
    fn row_to_post(row: &rusqlite::Row) -> Result<Post, rusqlite::Error> {
        let embed_json: Option<String> = row.get(9)?;
        let repost_reason_json: Option<String> = row.get(10)?;
        let reply_context_json: Option<String> = row.get(11)?;

        let embed: Option<Embed> = embed_json
            .as_ref()
            .and_then(|j| serde_json::from_str(j).ok());
        let repost_reason: Option<RepostReason> = repost_reason_json
            .as_ref()
            .and_then(|j| serde_json::from_str(j).ok());
        let reply_context: Option<ReplyContext> = reply_context_json
            .as_ref()
            .and_then(|j| serde_json::from_str(j).ok());

        Ok(Post {
            uri: row.get(0)?,
            cid: row.get(1)?,
            author: Profile {
                did: row.get(2)?,
                handle: row.get::<_, Option<String>>(14)?.unwrap_or_default(),
                display_name: row.get(15)?,
                avatar: row.get(16)?,
                banner: None,
                description: None,
                followers_count: None,
                following_count: None,
                posts_count: None,
                viewer_following: None,
                viewer_followed_by: None,
            },
            text: row.get(3)?,
            created_at: row.get(4)?,
            indexed_at: row.get(5)?,
            like_count: row.get(6)?,
            repost_count: row.get(7)?,
            reply_count: row.get(8)?,
            embed,
            viewer_like: row.get(12)?,
            viewer_repost: row.get(13)?,
            repost_reason,
            reply_context,
        })
    }
}
