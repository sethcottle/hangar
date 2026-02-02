// SPDX-License-Identifier: MPL-2.0

use crate::atproto::Profile;
use crate::cache::{CacheDb, CacheError};
use rusqlite::params;

/// Cache operations for profiles
pub struct ProfileCache<'a> {
    db: &'a CacheDb,
}

impl<'a> ProfileCache<'a> {
    pub fn new(db: &'a CacheDb) -> Self {
        Self { db }
    }

    /// Store a full profile (upserts, overwrites minimal)
    pub fn store_full(&self, profile: &Profile) -> Result<(), CacheError> {
        let conn = self.db.conn();
        let now = CacheDb::now();

        conn.execute(
            r#"
            INSERT INTO profiles (
                did, handle, display_name, avatar, banner, description,
                followers_count, following_count, posts_count,
                viewer_following, viewer_followed_by,
                fetched_at, is_full
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1)
            ON CONFLICT(did) DO UPDATE SET
                handle = excluded.handle,
                display_name = excluded.display_name,
                avatar = excluded.avatar,
                banner = excluded.banner,
                description = excluded.description,
                followers_count = excluded.followers_count,
                following_count = excluded.following_count,
                posts_count = excluded.posts_count,
                viewer_following = excluded.viewer_following,
                viewer_followed_by = excluded.viewer_followed_by,
                fetched_at = excluded.fetched_at,
                is_full = 1
            "#,
            params![
                profile.did,
                profile.handle,
                profile.display_name,
                profile.avatar,
                profile.banner,
                profile.description,
                profile.followers_count,
                profile.following_count,
                profile.posts_count,
                profile.viewer_following,
                profile.viewer_followed_by,
                now,
            ],
        )?;

        Ok(())
    }

    /// Store a minimal profile (only updates if not already full)
    #[allow(dead_code)]
    pub fn store_minimal(&self, profile: &Profile) -> Result<(), CacheError> {
        let conn = self.db.conn();
        let now = CacheDb::now();

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
                now,
            ],
        )?;

        Ok(())
    }

    /// Get profile by DID
    pub fn get(&self, did: &str) -> Result<Profile, CacheError> {
        let conn = self.db.conn();

        let mut stmt = conn.prepare(
            r#"
            SELECT
                did, handle, display_name, avatar, banner, description,
                followers_count, following_count, posts_count,
                viewer_following, viewer_followed_by, is_full
            FROM profiles
            WHERE did = ?
            "#,
        )?;

        let profile = stmt
            .query_row([did], |row| {
                Ok(Profile {
                    did: row.get(0)?,
                    handle: row.get(1)?,
                    display_name: row.get(2)?,
                    avatar: row.get(3)?,
                    banner: row.get(4)?,
                    description: row.get(5)?,
                    followers_count: row.get(6)?,
                    following_count: row.get(7)?,
                    posts_count: row.get(8)?,
                    viewer_following: row.get(9)?,
                    viewer_followed_by: row.get(10)?,
                })
            })
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => CacheError::NotFound,
                other => CacheError::Database(other),
            })?;

        Ok(profile)
    }

    /// Check if we have a fresh full profile
    #[allow(dead_code)]
    pub fn has_fresh_full(&self, did: &str, max_age_secs: i64) -> Result<bool, CacheError> {
        let conn = self.db.conn();
        let cutoff = CacheDb::now() - max_age_secs;

        let mut stmt = conn.prepare(
            r#"
            SELECT 1 FROM profiles
            WHERE did = ? AND is_full = 1 AND fetched_at > ?
            "#,
        )?;

        let exists = stmt.exists(params![did, cutoff])?;
        Ok(exists)
    }
}
