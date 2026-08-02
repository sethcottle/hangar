// SPDX-License-Identifier: MPL-2.0

//! Persistent SessionStore for OAuth sessions.
//!
//! Stores OAuth session data (DPoP keys + token set) as JSON in the
//! app's config directory so sessions survive app restarts.

use atrium_api::types::string::Did;
use atrium_common::store::Store;
use atrium_oauth::Scope;
use atrium_oauth::store::session::{Session, SessionStore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// One row of `oauth-sessions.json`: atrium's session plus the two
/// authorization-time facts atrium derives `client_id` from.
///
/// A loopback OAuth client has no registered `client_id`; atrium rebuilds one
/// from the redirect URI and the scope list on *every* token request, refreshes
/// included. A refresh that does not reproduce both byte for byte presents a
/// `client_id` the refresh token was never issued to and the authorization
/// server rejects it. atrium's own `Session` records neither, so we do.
///
/// The extra keys sit beside atrium's own rather than nesting them, so a file
/// written by a build that predates them still loads (the fields default to
/// `None`) and a file written here still loads in such a build.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredSession {
    #[serde(flatten)]
    session: Session,
    /// Absent on rows written before Hangar recorded this. Those sessions
    /// cannot be refreshed and need one final re-authorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redirect_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scopes: Option<Vec<Scope>>,
}

/// File-backed session store that persists OAuth sessions to disk.
///
/// Sessions are stored as a JSON map of DID → Session in the app's
/// config directory. An in-memory cache avoids repeated disk reads.
#[derive(Clone)]
pub struct FileSessionStore {
    inner: std::sync::Arc<Mutex<HashMap<Did, StoredSession>>>,
    path: PathBuf,
    /// Set only on the handle driving a live authorization. atrium writes the
    /// row itself once it exchanges the code, so the binding has to ride along
    /// on the store until that write happens.
    pending_redirect_uri: Option<String>,
    pending_scopes: Option<Vec<Scope>>,
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
            pending_redirect_uri: None,
            pending_scopes: None,
        }
    }

    /// Mark this handle as belonging to an in-flight authorization, so the row
    /// atrium writes when it exchanges the code records what `client_id` was
    /// built from.
    pub fn with_client_binding(mut self, redirect_uri: &str, scopes: &[Scope]) -> Self {
        self.pending_redirect_uri = Some(redirect_uri.to_string());
        self.pending_scopes = Some(scopes.to_vec());
        self
    }

    /// The redirect URI and scopes this DID's session was authorized under.
    ///
    /// `Ok(None)` means the session cannot be refreshed at all: either there is
    /// no row, or it was written before Hangar recorded the binding and the
    /// original `client_id` is gone for good. Callers must treat that as
    /// "sign in again", not as a transient failure.
    pub fn client_binding(&self, did: &Did) -> Result<Option<(String, Vec<Scope>)>, StoreError> {
        let guard = self.inner.lock().map_err(|_| StoreError::Lock)?;
        let Some(row) = guard.get(did) else {
            return Ok(None);
        };
        // Both or neither: they are written together, and half a binding
        // rebuilds a `client_id` as wrong as no binding at all.
        Ok(row.redirect_uri.clone().zip(row.scopes.clone()))
    }

    fn store_path() -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(crate::config::APP_ID);
        let _ = std::fs::create_dir_all(&config_dir);
        config_dir.join("oauth-sessions.json")
    }

    fn load_from_disk(path: &PathBuf) -> Result<HashMap<Did, StoredSession>, StoreError> {
        match std::fs::read_to_string(path) {
            Ok(data) => {
                let sessions: HashMap<String, StoredSession> = serde_json::from_str(&data)?;
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

    /// Backdate a stored session's expiry so the next refresh actually runs.
    ///
    /// atrium-oauth refreshes lazily: on a DPoP `invalid_token` response it
    /// calls into the session registry, which returns the cached session
    /// untouched if `expires_at` is still in the future and only performs a
    /// real refresh once the clock has passed it.
    ///
    /// That leaves no way out when the server rejects a token that has not
    /// expired by our clock -- revoked, rotated, or bound to a DPoP key the
    /// server no longer honours. The refresh is skipped, the retry replays the
    /// same dead token, and restarting does not help because the session is
    /// restored from this file without a refresh either. The app is stuck with
    /// every authenticated call failing until the recorded expiry passes.
    ///
    /// Treating our cached expiry as untrustworthy costs nothing when the
    /// token is good: no request fails, so no refresh is ever attempted. It
    /// only matters when a call does come back rejected, and then it lets the
    /// refresh do real work instead of short-circuiting. Orthogonal to the
    /// stored client binding: this decides whether a refresh is *attempted*,
    /// the binding decides whether it is *accepted*.
    pub fn invalidate_cached_expiry(&self, did: &Did) -> Result<(), StoreError> {
        let mut guard = self.inner.lock().map_err(|_| StoreError::Lock)?;
        let Some(row) = guard.get_mut(did) else {
            return Ok(());
        };
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        row.session.token_set.expires_at = Some(atrium_api::types::string::Datetime::new(
            past.fixed_offset(),
        ));
        self.save_to_disk(&guard)
    }

    /// Attach the client binding to a session on its way to disk.
    ///
    /// atrium writes back through `set` after every successful refresh, from a
    /// store handle that knows nothing about the original authorization. Taking
    /// the existing row's binding when this handle has none keeps that write
    /// from erasing it and breaking the *next* refresh.
    fn bind(&self, session: Session, existing: Option<&StoredSession>) -> StoredSession {
        StoredSession {
            session,
            redirect_uri: self
                .pending_redirect_uri
                .clone()
                .or_else(|| existing.and_then(|row| row.redirect_uri.clone())),
            scopes: self
                .pending_scopes
                .clone()
                .or_else(|| existing.and_then(|row| row.scopes.clone())),
        }
    }

    fn save_to_disk(&self, sessions: &HashMap<Did, StoredSession>) -> Result<(), StoreError> {
        // Convert Did keys to String for JSON serialization
        let string_map: HashMap<String, &StoredSession> =
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
        Ok(guard.get(key).map(|row| row.session.clone()))
    }

    async fn set(&self, key: Did, value: Session) -> Result<(), Self::Error> {
        let mut guard = self.inner.lock().map_err(|_| StoreError::Lock)?;
        let row = self.bind(value, guard.get(&key));
        guard.insert(key, row);
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

#[cfg(test)]
mod tests {
    use super::*;
    use atrium_oauth::KnownScope;

    /// A row as written by a build that predates the client binding. The key
    /// material is synthetic; only its shape matters.
    const LEGACY_ROW: &str = r#"{
        "dpop_key": {
            "kty": "EC",
            "crv": "P-256",
            "x": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "y": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "d": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        },
        "token_set": {
            "iss": "https://bsky.social",
            "sub": "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa",
            "aud": "https://pds.example.com",
            "scope": "atproto transition:generic",
            "refresh_token": "refresh",
            "access_token": "access",
            "token_type": "DPoP",
            "expires_at": "2026-01-01T00:00:00.000Z"
        }
    }"#;

    fn store_with_pending(binding: Option<(&str, Vec<Scope>)>) -> FileSessionStore {
        let store = FileSessionStore {
            inner: std::sync::Arc::new(Mutex::new(HashMap::new())),
            path: PathBuf::from("/nonexistent/oauth-sessions.json"),
            pending_redirect_uri: None,
            pending_scopes: None,
        };
        match binding {
            Some((uri, scopes)) => store.with_client_binding(uri, &scopes),
            None => store,
        }
    }

    /// The flattened layout has to stay readable both ways, or shipping this
    /// change would strand every session written before it.
    #[test]
    fn legacy_row_loads_without_a_binding() {
        let row: StoredSession = serde_json::from_str(LEGACY_ROW).expect("legacy row parses");
        assert!(row.redirect_uri.is_none());
        assert!(row.scopes.is_none());
        assert_eq!(row.session.token_set.access_token, "access");
    }

    #[test]
    fn bound_row_round_trips_and_stays_readable_as_a_bare_session() {
        let legacy: StoredSession = serde_json::from_str(LEGACY_ROW).unwrap();
        let bound = store_with_pending(Some((
            "http://127.0.0.1:41234/callback",
            vec![Scope::Known(KnownScope::Atproto)],
        )))
        .bind(legacy.session.clone(), None);

        let json = serde_json::to_string(&bound).unwrap();
        let reparsed: StoredSession = serde_json::from_str(&json).unwrap();
        assert_eq!(
            reparsed.redirect_uri.as_deref(),
            Some("http://127.0.0.1:41234/callback")
        );
        assert_eq!(
            reparsed.scopes,
            Some(vec![Scope::Known(KnownScope::Atproto)])
        );

        // A build without these fields must still load what we wrote.
        let bare: Session = serde_json::from_str(&json).expect("still a valid Session");
        assert_eq!(bare.token_set.access_token, "access");
    }

    /// The regression that would silently reintroduce the original bug: atrium
    /// writes back through `set` after a successful refresh with no pending
    /// binding, and dropping it there breaks the refresh after next.
    #[test]
    fn refresh_write_keeps_the_existing_binding() {
        let legacy: StoredSession = serde_json::from_str(LEGACY_ROW).unwrap();
        let existing = store_with_pending(Some((
            "http://127.0.0.1:41234/callback",
            vec![Scope::Known(KnownScope::Atproto)],
        )))
        .bind(legacy.session.clone(), None);

        let refreshed = store_with_pending(None).bind(legacy.session, Some(&existing));
        assert_eq!(refreshed.redirect_uri, existing.redirect_uri);
        assert_eq!(refreshed.scopes, existing.scopes);
    }
}
