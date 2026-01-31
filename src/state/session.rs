// SPDX-License-Identifier: MPL-2.0

use crate::atproto::Session;
use crate::config::APP_ID;
use secret_service::{EncryptionType, SecretService};
use thiserror::Error;

const SECRET_LABEL: &str = "Hangar Bluesky Session";

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("secret service unavailable: {0}")]
    SecretService(String),
    #[error("session not found")]
    NotFound,
    #[error("invalid session data: {0}")]
    InvalidData(String),
}

/// Persists session credentials via libsecret.
pub struct SessionManager;

impl SessionManager {
    pub async fn store(session: &Session) -> Result<(), SessionError> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| SessionError::SecretService(e.to_string()))?;

        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| SessionError::SecretService(e.to_string()))?;

        if collection.is_locked().await.unwrap_or(true) {
            collection
                .unlock()
                .await
                .map_err(|e| SessionError::SecretService(e.to_string()))?;
        }

        let session_json =
            serde_json::to_string(session).map_err(|e| SessionError::InvalidData(e.to_string()))?;

        let attributes = vec![("application", APP_ID), ("did", &session.did)];

        collection
            .create_item(
                SECRET_LABEL,
                attributes.into_iter().collect(),
                session_json.as_bytes(),
                true, // replace existing
                "text/plain",
            )
            .await
            .map_err(|e| SessionError::SecretService(e.to_string()))?;

        Ok(())
    }

    pub async fn load() -> Result<Session, SessionError> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| SessionError::SecretService(e.to_string()))?;

        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| SessionError::SecretService(e.to_string()))?;

        if collection.is_locked().await.unwrap_or(true) {
            collection
                .unlock()
                .await
                .map_err(|e| SessionError::SecretService(e.to_string()))?;
        }

        let attributes = vec![("application", APP_ID)];
        let items = collection
            .search_items(attributes.into_iter().collect())
            .await
            .map_err(|e| SessionError::SecretService(e.to_string()))?;

        let item = items.first().ok_or(SessionError::NotFound)?;

        let secret = item
            .get_secret()
            .await
            .map_err(|e| SessionError::SecretService(e.to_string()))?;

        let session: Session = serde_json::from_slice(&secret)
            .map_err(|e| SessionError::InvalidData(e.to_string()))?;

        Ok(session)
    }

    #[allow(dead_code)]
    pub async fn clear() -> Result<(), SessionError> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| SessionError::SecretService(e.to_string()))?;

        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| SessionError::SecretService(e.to_string()))?;

        if collection.is_locked().await.unwrap_or(true) {
            collection
                .unlock()
                .await
                .map_err(|e| SessionError::SecretService(e.to_string()))?;
        }

        let attributes = vec![("application", APP_ID)];
        let items = collection
            .search_items(attributes.into_iter().collect())
            .await
            .map_err(|e| SessionError::SecretService(e.to_string()))?;

        for item in items {
            item.delete()
                .await
                .map_err(|e| SessionError::SecretService(e.to_string()))?;
        }

        Ok(())
    }
}
