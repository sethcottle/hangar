// SPDX-License-Identifier: MPL-2.0

//! Image caching with SQLite-backed persistent storage and LRU memory cache.
//!
//! This module provides efficient image caching with:
//! - In-memory LRU cache for fast access to recently used images
//! - SQLite blob storage for persistence across app restarts
//! - Automatic cleanup of old/large caches
//! - Size limits to prevent unbounded growth

use crate::cache::{CacheDb, CacheError};
use rusqlite::params;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Maximum number of images to keep in memory (LRU eviction)
const MEMORY_CACHE_CAPACITY: usize = 200;

/// Maximum total size of disk cache in bytes (100MB)
const MAX_DISK_CACHE_BYTES: i64 = 100 * 1024 * 1024;

/// Maximum age for cached images before cleanup (30 days)
const MAX_IMAGE_AGE_SECS: i64 = 30 * 24 * 60 * 60;

/// Cached image data with metadata
#[derive(Clone)]
pub struct CachedImage {
    pub data: Vec<u8>,
    #[allow(dead_code)]
    pub content_type: Option<String>,
}

/// Simple LRU cache using a HashMap + access order tracking
struct LruCache {
    map: HashMap<String, CachedImage>,
    order: Vec<String>, // Most recently used at the end
    capacity: usize,
}

impl LruCache {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
            order: Vec::with_capacity(capacity),
            capacity,
        }
    }

    fn get(&mut self, key: &str) -> Option<CachedImage> {
        if let Some(img) = self.map.get(key).cloned() {
            // Move to end (most recently used)
            self.order.retain(|k| k != key);
            self.order.push(key.to_string());
            Some(img)
        } else {
            None
        }
    }

    fn insert(&mut self, key: String, value: CachedImage) {
        // Evict oldest if at capacity
        while self.map.len() >= self.capacity && !self.order.is_empty() {
            let oldest = self.order.remove(0);
            self.map.remove(&oldest);
        }

        self.map.insert(key.clone(), value);
        self.order.retain(|k| k != &key);
        self.order.push(key);
    }

    #[allow(dead_code)]
    fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }
}

/// Image cache with memory LRU + SQLite disk backing
pub struct ImageCache {
    memory: Arc<Mutex<LruCache>>,
}

impl ImageCache {
    /// Create a new image cache
    pub fn new() -> Self {
        Self {
            memory: Arc::new(Mutex::new(LruCache::new(MEMORY_CACHE_CAPACITY))),
        }
    }

    /// Get an image from cache (memory first, then disk)
    pub fn get(&self, db: &CacheDb, url: &str) -> Option<CachedImage> {
        // Check memory cache first
        {
            let mut memory = self.memory.lock().unwrap();
            if let Some(img) = memory.get(url) {
                return Some(img);
            }
        }

        // Try disk cache
        if let Ok(img) = self.get_from_disk(db, url) {
            // Promote to memory cache
            let mut memory = self.memory.lock().unwrap();
            memory.insert(url.to_string(), img.clone());
            return Some(img);
        }

        None
    }

    /// Check if an image is cached (memory or disk)
    #[allow(dead_code)]
    pub fn contains(&self, db: &CacheDb, url: &str) -> bool {
        // Check memory first
        {
            let memory = self.memory.lock().unwrap();
            if memory.contains(url) {
                return true;
            }
        }

        // Check disk
        self.exists_on_disk(db, url)
    }

    /// Store an image in both memory and disk cache
    pub fn store(
        &self,
        db: &CacheDb,
        url: &str,
        data: Vec<u8>,
        content_type: Option<String>,
    ) -> Result<(), CacheError> {
        let img = CachedImage {
            data: data.clone(),
            content_type: content_type.clone(),
        };

        // Store in memory
        {
            let mut memory = self.memory.lock().unwrap();
            memory.insert(url.to_string(), img);
        }

        // Store on disk
        self.store_to_disk(db, url, &data, content_type.as_deref())?;

        Ok(())
    }

    /// Get image from disk cache
    fn get_from_disk(&self, db: &CacheDb, url: &str) -> Result<CachedImage, CacheError> {
        let conn = db.conn();
        let now = CacheDb::now();

        // Update last_accessed_at when reading
        let mut stmt = conn.prepare(
            r#"
            UPDATE images SET last_accessed_at = ?1 WHERE url = ?2
            RETURNING data, content_type
            "#,
        )?;

        let img = stmt
            .query_row(params![now, url], |row| {
                let data: Vec<u8> = row.get(0)?;
                let content_type: Option<String> = row.get(1)?;
                Ok(CachedImage { data, content_type })
            })
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => CacheError::NotFound,
                other => CacheError::Database(other),
            })?;

        Ok(img)
    }

    /// Check if image exists on disk
    #[allow(dead_code)]
    fn exists_on_disk(&self, db: &CacheDb, url: &str) -> bool {
        let conn = db.conn();
        conn.query_row("SELECT 1 FROM images WHERE url = ?", [url], |_| Ok(()))
            .is_ok()
    }

    /// Store image to disk cache
    fn store_to_disk(
        &self,
        db: &CacheDb,
        url: &str,
        data: &[u8],
        content_type: Option<&str>,
    ) -> Result<(), CacheError> {
        let conn = db.conn();
        let now = CacheDb::now();
        let size = data.len() as i64;

        conn.execute(
            r#"
            INSERT INTO images (url, data, content_type, size, fetched_at, last_accessed_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?5)
            ON CONFLICT(url) DO UPDATE SET
                data = excluded.data,
                content_type = excluded.content_type,
                size = excluded.size,
                fetched_at = excluded.fetched_at,
                last_accessed_at = excluded.last_accessed_at
            "#,
            params![url, data, content_type, size, now],
        )?;

        Ok(())
    }

    /// Clean up old and excess images from disk cache
    pub fn cleanup(&self, db: &CacheDb) -> Result<CleanupStats, CacheError> {
        let conn = db.conn();
        let now = CacheDb::now();
        let age_cutoff = now - MAX_IMAGE_AGE_SECS;

        // Delete images older than max age
        let old_deleted = conn.execute(
            "DELETE FROM images WHERE last_accessed_at < ?",
            [age_cutoff],
        )?;

        // Check total size and delete oldest if over limit
        let total_size: i64 = conn
            .query_row("SELECT COALESCE(SUM(size), 0) FROM images", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        let mut size_deleted = 0;
        if total_size > MAX_DISK_CACHE_BYTES {
            // Delete oldest images until under limit
            let excess = total_size - MAX_DISK_CACHE_BYTES;
            size_deleted = conn.execute(
                r#"
                DELETE FROM images WHERE url IN (
                    SELECT url FROM images
                    ORDER BY last_accessed_at ASC
                    LIMIT (
                        SELECT COUNT(*) FROM images
                        WHERE (
                            SELECT SUM(size) FROM images i2
                            WHERE i2.last_accessed_at <= images.last_accessed_at
                        ) <= ?
                    )
                )
                "#,
                [excess],
            )?;
        }

        Ok(CleanupStats {
            old_deleted,
            size_deleted,
            total_size_after: total_size - (size_deleted as i64 * 10000), // Approximate
        })
    }

    /// Get cache statistics
    pub fn stats(&self, db: &CacheDb) -> Result<CacheStats, CacheError> {
        let conn = db.conn();

        let (count, total_size): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM images",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        let memory_count = {
            let memory = self.memory.lock().unwrap();
            memory.map.len()
        };

        Ok(CacheStats {
            disk_count: count as usize,
            disk_size_bytes: total_size as usize,
            memory_count,
        })
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about cache cleanup
#[derive(Debug)]
pub struct CleanupStats {
    pub old_deleted: usize,
    pub size_deleted: usize,
    pub total_size_after: i64,
}

/// Cache statistics
#[derive(Debug)]
#[allow(dead_code)]
pub struct CacheStats {
    pub disk_count: usize,
    pub disk_size_bytes: usize,
    pub memory_count: usize,
}
