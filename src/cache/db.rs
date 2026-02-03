// SPDX-License-Identifier: MPL-2.0

use crate::cache::CacheError;
use crate::cache::schema::SCHEMA;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Handle to the cache database for a specific user
#[derive(Clone)]
pub struct CacheDb {
    conn: Arc<Mutex<Connection>>,
    #[allow(dead_code)]
    user_did: String,
}

impl CacheDb {
    /// Open or create cache database for user
    /// Path: ~/.local/share/hangar/{user_did}/cache.db
    pub fn open(user_did: &str) -> Result<Self, CacheError> {
        let path = Self::cache_path(user_did)?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CacheError::Path(format!("failed to create cache dir: {}", e)))?;
        }

        let conn = Connection::open(&path)?;

        // Run migrations
        Self::migrate(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            user_did: user_did.to_string(),
        })
    }

    /// Run schema migrations
    fn migrate(conn: &Connection) -> Result<(), CacheError> {
        // Execute the schema (all CREATE IF NOT EXISTS)
        conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    /// Get XDG data directory for cache
    fn cache_path(user_did: &str) -> Result<PathBuf, CacheError> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| CacheError::Path("could not find data directory".to_string()))?;

        // Sanitize DID for filesystem (replace : with _)
        let safe_did = user_did.replace(':', "_");

        Ok(data_dir.join("hangar").join(safe_did).join("cache.db"))
    }

    /// Access connection for operations
    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("cache lock poisoned")
    }

    /// Get current unix timestamp
    pub fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Cleanup old entries with sensible defaults:
    /// - Feed items: 24 hours (feed order changes constantly)
    /// - Orphan posts: 7 days (posts not in any feed)
    /// - Profiles: kept if any post references them
    pub fn cleanup_stale(&self) -> Result<(), CacheError> {
        let conn = self.conn();
        let now = Self::now();

        // Feed items older than 24 hours - the feed order is stale
        let feed_cutoff = now - (24 * 60 * 60);
        conn.execute("DELETE FROM feed_items WHERE fetched_at < ?", [feed_cutoff])?;

        // Feed state older than 24 hours
        conn.execute(
            "DELETE FROM feed_state WHERE last_refresh_at < ?",
            [feed_cutoff],
        )?;

        // Orphan posts (not in any feed) older than 7 days
        let post_cutoff = now - (7 * 24 * 60 * 60);
        conn.execute(
            r#"
            DELETE FROM posts
            WHERE fetched_at < ?
            AND uri NOT IN (SELECT post_uri FROM feed_items)
            "#,
            [post_cutoff],
        )?;

        // Orphan profiles (no posts reference them) older than 7 days
        conn.execute(
            r#"
            DELETE FROM profiles
            WHERE fetched_at < ?
            AND did NOT IN (SELECT author_did FROM posts)
            "#,
            [post_cutoff],
        )?;

        Ok(())
    }
}
