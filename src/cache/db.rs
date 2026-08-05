// SPDX-License-Identifier: MPL-2.0

use crate::cache::CacheError;
use crate::cache::schema::{MIGRATION_3, SCHEMA, SCHEMA_VERSION};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Nothing in here is authoritative, it all refetches, so paying an fsync per
/// commit buys nothing and a home feed refresh commits six times on the main
/// thread. WAL plus NORMAL keeps that off the sync path. The busy timeout
/// covers the connection being shared across threads.
const PRAGMAS: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
";

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

        Ok(Self {
            conn: Arc::new(Mutex::new(Self::open_resilient(&path)?)),
            user_did: user_did.to_string(),
        })
    }

    /// Open the file, and if it cannot be opened or migrated, throw it away and
    /// build a fresh one. A cache we refuse to touch costs the user every read
    /// for the whole session; a rebuilt one costs a single refresh.
    fn open_resilient(path: &Path) -> Result<Connection, CacheError> {
        match Self::prepare(path) {
            Ok(conn) => Ok(conn),
            Err(e) => {
                eprintln!("Cache at {} unusable, rebuilding: {}", path.display(), e);
                Self::discard(path)?;
                Self::prepare(path)
            }
        }
    }

    /// One connection, pragmas applied, schema current.
    fn prepare(path: &Path) -> Result<Connection, CacheError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(PRAGMAS)?;
        Self::check_integrity(&conn)?;
        Self::migrate(&conn)?;
        Ok(conn)
    }

    /// `Connection::open` accepts any file at all, so a truncated or scribbled
    /// cache only fails later, one swallowed read at a time.
    fn check_integrity(conn: &Connection) -> Result<(), CacheError> {
        let result: String = conn.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
        if result == "ok" {
            Ok(())
        } else {
            Err(corrupt(&result))
        }
    }

    /// Remove the database and its WAL sidecars so a retry starts clean.
    fn discard(path: &Path) -> Result<(), CacheError> {
        for suffix in ["", "-wal", "-shm"] {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            let file = PathBuf::from(name);
            if let Err(e) = std::fs::remove_file(&file)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                return Err(CacheError::Path(format!(
                    "failed to remove {}: {}",
                    file.display(),
                    e
                )));
            }
        }
        Ok(())
    }

    /// Bring the schema up to `SCHEMA_VERSION`. A fresh file is at version 0
    /// and takes the whole schema; older files walk the steps they missed.
    fn migrate(conn: &Connection) -> Result<(), CacheError> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if version == SCHEMA_VERSION {
            return Ok(());
        }
        if version > SCHEMA_VERSION {
            // Written by a newer build. Rebuilding costs one refresh; reading a
            // schema we do not know costs correctness.
            return Err(corrupt(&format!(
                "schema is version {}, this build understands {}",
                version, SCHEMA_VERSION
            )));
        }

        let tx = conn.unchecked_transaction()?;
        if version < 1 {
            tx.execute_batch(SCHEMA)?;
        }
        // Versions 1 and 2 differ only in tables version 3 drops, so both land
        // here with nothing left to add.
        if version < 3 {
            tx.execute_batch(MIGRATION_3)?;
        }
        tx.execute_batch(&format!("PRAGMA user_version = {}", SCHEMA_VERSION))?;
        tx.commit()?;
        Ok(())
    }

    /// Get XDG data directory for cache
    /// Where this account's cache lives. Public so Clear Cache can delete
    /// it after the connection is dropped.
    pub fn cache_path(user_did: &str) -> Result<PathBuf, CacheError> {
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

        // Feed items expire after 24 hours; the ordering is stale by then
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

/// A cache we cannot make sense of is corrupt as far as callers care, and it
/// takes the same rebuild path as a truncated file.
fn corrupt(detail: &str) -> CacheError {
    CacheError::Database(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
        Some(detail.to_string()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A unique cache path under the system temp dir, removed on drop.
    struct TempDb(PathBuf);

    impl TempDb {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("hangar-cache-{}-{}", std::process::id(), n));
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self(dir.join("cache.db"))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            if let Some(dir) = self.0.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    fn version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version")
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .expect("sqlite_master")
            > 0
    }

    #[test]
    fn fresh_cache_is_wal_and_unsynced() {
        let db = TempDb::new();
        let conn = CacheDb::open_resilient(db.path()).expect("open");

        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal_mode");
        assert_eq!(journal, "wal");

        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("synchronous");
        assert_eq!(synchronous, 1, "NORMAL");

        let busy: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("busy_timeout");
        assert_eq!(busy, 5000);
    }

    #[test]
    fn fresh_cache_is_stamped_and_reopens_clean() {
        let db = TempDb::new();
        let conn = CacheDb::open_resilient(db.path()).expect("open");
        assert_eq!(version(&conn), SCHEMA_VERSION);
        conn.execute(
            "INSERT INTO feed_state (feed_key, has_more) VALUES ('home', 1)",
            [],
        )
        .expect("insert");
        drop(conn);

        // Reopening is a no-op migration, so the row survives.
        let conn = CacheDb::open_resilient(db.path()).expect("reopen");
        assert_eq!(version(&conn), SCHEMA_VERSION);
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM feed_state", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 1);
    }

    #[test]
    fn version_two_cache_upgrades_in_place() {
        let db = TempDb::new();
        {
            let conn = Connection::open(db.path()).expect("open");
            conn.execute_batch(SCHEMA).expect("schema");
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS notifications (uri TEXT PRIMARY KEY);
                 CREATE TABLE IF NOT EXISTS images (url TEXT PRIMARY KEY);
                 PRAGMA user_version = 2;",
            )
            .expect("legacy tables");
            conn.execute(
                "INSERT INTO feed_state (feed_key, has_more) VALUES ('home', 1)",
                [],
            )
            .expect("insert");
        }

        let conn = CacheDb::open_resilient(db.path()).expect("open");
        assert_eq!(version(&conn), SCHEMA_VERSION);
        assert!(!table_exists(&conn, "notifications"));
        assert!(!table_exists(&conn, "images"));

        // The upgrade is in place, so cached rows are not thrown away.
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM feed_state", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 1);
    }

    #[test]
    fn corrupt_file_is_rebuilt() {
        let db = TempDb::new();
        std::fs::write(db.path(), b"SQLite format 3\0this is not a database")
            .expect("write garbage");

        let conn = CacheDb::open_resilient(db.path()).expect("rebuild");
        assert_eq!(version(&conn), SCHEMA_VERSION);
        assert!(table_exists(&conn, "posts"));
    }

    #[test]
    fn cache_from_a_newer_build_is_rebuilt() {
        let db = TempDb::new();
        {
            let conn = CacheDb::open_resilient(db.path()).expect("open");
            conn.execute_batch("PRAGMA user_version = 99")
                .expect("bump");
            conn.execute(
                "INSERT INTO feed_state (feed_key, has_more) VALUES ('home', 1)",
                [],
            )
            .expect("insert");
        }

        let conn = CacheDb::open_resilient(db.path()).expect("rebuild");
        assert_eq!(version(&conn), SCHEMA_VERSION);
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM feed_state", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 0, "the unreadable cache was thrown away");
    }
}
