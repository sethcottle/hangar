// SPDX-License-Identifier: MPL-2.0

use crate::atproto::{Embed, Post, Profile, ReplyContext, RepostReason};
use crate::cache::{CacheDb, CacheError};
use rusqlite::params;

/// Cache operations for posts
pub struct PostCache<'a> {
    db: &'a CacheDb,
}

impl<'a> PostCache<'a> {
    pub fn new(db: &'a CacheDb) -> Self {
        Self { db }
    }

    /// Store a single post (upserts)
    #[allow(dead_code)]
    pub fn store(&self, post: &Post) -> Result<(), CacheError> {
        let conn = self.db.conn();
        let now = CacheDb::now();

        // Serialize complex fields to JSON
        let embed_json = post.embed.as_ref().map(serde_json::to_string).transpose()?;
        let repost_reason_json = post
            .repost_reason
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let reply_context_json = post
            .reply_context
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        conn.execute(
            r#"
            INSERT INTO posts (
                uri, cid, author_did, text, created_at, indexed_at,
                like_count, repost_count, reply_count,
                embed_json, repost_reason_json, reply_context_json,
                viewer_like, viewer_repost, fetched_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(uri) DO UPDATE SET
                cid = excluded.cid,
                text = excluded.text,
                like_count = excluded.like_count,
                repost_count = excluded.repost_count,
                reply_count = excluded.reply_count,
                embed_json = excluded.embed_json,
                repost_reason_json = excluded.repost_reason_json,
                reply_context_json = excluded.reply_context_json,
                viewer_like = excluded.viewer_like,
                viewer_repost = excluded.viewer_repost,
                fetched_at = excluded.fetched_at
            "#,
            params![
                post.uri,
                post.cid,
                post.author.did,
                post.text,
                post.created_at,
                post.indexed_at,
                post.like_count,
                post.repost_count,
                post.reply_count,
                embed_json,
                repost_reason_json,
                reply_context_json,
                post.viewer_like,
                post.viewer_repost,
                now,
            ],
        )?;

        // Also store the author profile (minimal)
        Self::store_minimal_profile(&post.author, now, &conn)?;

        Ok(())
    }

    /// Store multiple posts in a transaction
    pub fn store_batch(&self, posts: &[Post]) -> Result<(), CacheError> {
        let mut conn = self.db.conn();
        let tx = conn.transaction()?;
        let now = CacheDb::now();

        for post in posts {
            let embed_json = post.embed.as_ref().map(serde_json::to_string).transpose()?;
            let repost_reason_json = post
                .repost_reason
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            let reply_context_json = post
                .reply_context
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;

            tx.execute(
                r#"
                INSERT INTO posts (
                    uri, cid, author_did, text, created_at, indexed_at,
                    like_count, repost_count, reply_count,
                    embed_json, repost_reason_json, reply_context_json,
                    viewer_like, viewer_repost, fetched_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                ON CONFLICT(uri) DO UPDATE SET
                    cid = excluded.cid,
                    text = excluded.text,
                    like_count = excluded.like_count,
                    repost_count = excluded.repost_count,
                    reply_count = excluded.reply_count,
                    embed_json = excluded.embed_json,
                    repost_reason_json = excluded.repost_reason_json,
                    reply_context_json = excluded.reply_context_json,
                    viewer_like = excluded.viewer_like,
                    viewer_repost = excluded.viewer_repost,
                    fetched_at = excluded.fetched_at
                "#,
                params![
                    post.uri,
                    post.cid,
                    post.author.did,
                    post.text,
                    post.created_at,
                    post.indexed_at,
                    post.like_count,
                    post.repost_count,
                    post.reply_count,
                    embed_json,
                    repost_reason_json,
                    reply_context_json,
                    post.viewer_like,
                    post.viewer_repost,
                    now,
                ],
            )?;

            // Store author profile
            Self::store_minimal_profile_tx(&post.author, now, &tx)?;

            // Store repost_reason author if present
            if let Some(reason) = &post.repost_reason {
                Self::store_minimal_profile_tx(&reason.by, now, &tx)?;
            }

            // Store reply_context authors if present
            if let Some(ctx) = &post.reply_context {
                Self::store_minimal_profile_tx(&ctx.parent_author, now, &tx)?;
                Self::store_minimal_profile_tx(&ctx.root_author, now, &tx)?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Get post by URI
    #[allow(dead_code)]
    pub fn get(&self, uri: &str) -> Result<Post, CacheError> {
        let conn = self.db.conn();

        let mut stmt = conn.prepare(
            r#"
            SELECT
                p.uri, p.cid, p.author_did, p.text, p.created_at, p.indexed_at,
                p.like_count, p.repost_count, p.reply_count,
                p.embed_json, p.repost_reason_json, p.reply_context_json,
                p.viewer_like, p.viewer_repost,
                pr.handle, pr.display_name, pr.avatar
            FROM posts p
            LEFT JOIN profiles pr ON p.author_did = pr.did
            WHERE p.uri = ?
            "#,
        )?;

        let post = stmt
            .query_row([uri], Self::row_to_post)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => CacheError::NotFound,
                other => CacheError::Database(other),
            })?;

        Ok(post)
    }

    /// Get multiple posts by URIs (preserves order)
    #[allow(dead_code)]
    pub fn get_batch(&self, uris: &[String]) -> Result<Vec<Post>, CacheError> {
        if uris.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.db.conn();

        // Build query with placeholders
        let placeholders: Vec<_> = (1..=uris.len()).map(|i| format!("?{}", i)).collect();
        let query = format!(
            r#"
            SELECT
                p.uri, p.cid, p.author_did, p.text, p.created_at, p.indexed_at,
                p.like_count, p.repost_count, p.reply_count,
                p.embed_json, p.repost_reason_json, p.reply_context_json,
                p.viewer_like, p.viewer_repost,
                pr.handle, pr.display_name, pr.avatar
            FROM posts p
            LEFT JOIN profiles pr ON p.author_did = pr.did
            WHERE p.uri IN ({})
            "#,
            placeholders.join(", ")
        );

        let mut stmt = conn.prepare(&query)?;

        // Convert URIs to params
        let params: Vec<&dyn rusqlite::ToSql> =
            uris.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

        let mut rows = stmt.query(params.as_slice())?;
        let mut posts_map = std::collections::HashMap::new();

        while let Some(row) = rows.next()? {
            let post = Self::row_to_post(row)?;
            posts_map.insert(post.uri.clone(), post);
        }

        // Return in original order
        Ok(uris
            .iter()
            .filter_map(|uri| posts_map.remove(uri))
            .collect())
    }

    /// Update viewer state only (after like/repost actions)
    #[allow(dead_code)]
    pub fn update_viewer_state(
        &self,
        uri: &str,
        viewer_like: Option<&str>,
        viewer_repost: Option<&str>,
    ) -> Result<(), CacheError> {
        let conn = self.db.conn();

        conn.execute(
            r#"
            UPDATE posts
            SET viewer_like = ?, viewer_repost = ?
            WHERE uri = ?
            "#,
            params![viewer_like, viewer_repost, uri],
        )?;

        Ok(())
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

    /// Store a minimal profile (from post author)
    fn store_minimal_profile(
        profile: &Profile,
        now: i64,
        conn: &rusqlite::Connection,
    ) -> Result<(), CacheError> {
        conn.execute(
            r#"
            INSERT INTO profiles (did, handle, display_name, avatar, fetched_at, is_full)
            VALUES (?1, ?2, ?3, ?4, ?5, 0)
            ON CONFLICT(did) DO UPDATE SET
                handle = excluded.handle,
                display_name = COALESCE(excluded.display_name, profiles.display_name),
                avatar = COALESCE(excluded.avatar, profiles.avatar),
                fetched_at = excluded.fetched_at
            WHERE profiles.is_full = 0
            "#,
            params![
                profile.did,
                profile.handle,
                profile.display_name,
                profile.avatar,
                now
            ],
        )?;
        Ok(())
    }

    /// Store a minimal profile in a transaction
    fn store_minimal_profile_tx(
        profile: &Profile,
        now: i64,
        tx: &rusqlite::Transaction,
    ) -> Result<(), CacheError> {
        tx.execute(
            r#"
            INSERT INTO profiles (did, handle, display_name, avatar, fetched_at, is_full)
            VALUES (?1, ?2, ?3, ?4, ?5, 0)
            ON CONFLICT(did) DO UPDATE SET
                handle = excluded.handle,
                display_name = COALESCE(excluded.display_name, profiles.display_name),
                avatar = COALESCE(excluded.avatar, profiles.avatar),
                fetched_at = excluded.fetched_at
            WHERE profiles.is_full = 0
            "#,
            params![
                profile.did,
                profile.handle,
                profile.display_name,
                profile.avatar,
                now
            ],
        )?;
        Ok(())
    }
}
