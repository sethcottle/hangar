// SPDX-License-Identifier: MPL-2.0

//! Persistent SessionStore for OAuth sessions.
//!
//! Stores OAuth session data (DPoP keys + token set) as JSON in the
//! app's config directory so sessions survive app restarts.

use atrium_api::types::string::Did;
use atrium_common::store::Store;
use atrium_oauth::store::session::{Session, SessionStore};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// File-backed session store that persists OAuth sessions to disk.
///
/// Sessions are stored as a JSON map of DID → Session in the app's
/// config directory. An in-memory cache avoids repeated disk reads.
#[derive(Clone)]
pub struct FileSessionStore {
    inner: std::sync::Arc<Mutex<HashMap<Did, Session>>>,
    path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("lock poisoned")]
    Lock,
}

impl FileSessionStore {
    /// Create a new store, loading any existing sessions from disk.
    pub fn new() -> Self {
        let path = Self::store_path();
        let sessions = Self::load_from_disk(&path).unwrap_or_default();
        Self {
            inner: std::sync::Arc::new(Mutex::new(sessions)),
            path,
        }
    }

    fn store_path() -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("io.github.sethcottle.Hangar");
        let _ = std::fs::create_dir_all(&config_dir);
        config_dir.join("oauth-sessions.json")
    }

    fn load_from_disk(path: &PathBuf) -> Result<HashMap<Did, Session>, StoreError> {
        match std::fs::read_to_string(path) {
            Ok(data) => {
                let sessions: HashMap<String, Session> = serde_json::from_str(&data)?;
                // Convert String keys back to Did
                let mut map = HashMap::new();
                for (key, value) in sessions {
                    if let Ok(did) = key.parse::<Did>() {
                        map.insert(did, value);
                    }
                }
                Ok(map)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(StoreError::Io(e)),
        }
    }

    fn save_to_disk(&self, sessions: &HashMap<Did, Session>) -> Result<(), StoreError> {
        // Convert Did keys to String for JSON serialization
        let string_map: HashMap<String, &Session> =
            sessions.iter().map(|(k, v)| (k.to_string(), v)).collect();
        let json = serde_json::to_string_pretty(&string_map)?;

        // Write atomically via temp file
        let tmp_path = self.path.with_extension("json.tmp");
        std::fs::write(&tmp_path, json)?;
        std::fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }
}

impl Store<Did, Session> for FileSessionStore {
    type Error = StoreError;

    async fn get(&self, key: &Did) -> Result<Option<Session>, Self::Error> {
        let guard = self.inner.lock().map_err(|_| StoreError::Lock)?;
        Ok(guard.get(key).cloned())
    }

    async fn set(&self, key: Did, value: Session) -> Result<(), Self::Error> {
        let mut guard = self.inner.lock().map_err(|_| StoreError::Lock)?;
        guard.insert(key, value);
        self.save_to_disk(&guard)?;
        Ok(())
    }

    async fn del(&self, key: &Did) -> Result<(), Self::Error> {
        let mut guard = self.inner.lock().map_err(|_| StoreError::Lock)?;
        guard.remove(key);
        self.save_to_disk(&guard)?;
        Ok(())
    }

    async fn clear(&self) -> Result<(), Self::Error> {
        let mut guard = self.inner.lock().map_err(|_| StoreError::Lock)?;
        guard.clear();
        self.save_to_disk(&guard)?;
        Ok(())
    }
}

impl SessionStore for FileSessionStore {}
