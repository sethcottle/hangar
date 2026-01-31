// SPDX-License-Identifier: MPL-2.0

use crate::atproto::types::{Post, Profile, Session};
use crate::config::DEFAULT_PDS;
use atrium_api::agent::AtpAgent;
use atrium_api::agent::store::MemorySessionStore;
use atrium_xrpc_client::reqwest::ReqwestClient;
use std::sync::RwLock;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("not authenticated")]
    NotAuthenticated,
}

type Agent = AtpAgent<MemorySessionStore, ReqwestClient>;

/// Wraps atrium so the rest of the app only sees our own types.
pub struct HangarClient {
    agent: RwLock<Option<Agent>>,
    service_url: String,
}

impl HangarClient {
    pub fn new() -> Self {
        Self {
            agent: RwLock::new(None),
            service_url: DEFAULT_PDS.to_string(),
        }
    }

    #[allow(dead_code)]
    pub fn with_service(service_url: &str) -> Self {
        Self {
            agent: RwLock::new(None),
            service_url: service_url.to_string(),
        }
    }

    pub async fn login(&self, handle: &str, password: &str) -> Result<Session, ClientError> {
        let client = ReqwestClient::new(&self.service_url);
        let agent = AtpAgent::new(client, MemorySessionStore::default());

        let result = agent
            .login(handle, password)
            .await
            .map_err(|e| ClientError::Auth(e.to_string()))?;

        let session = Session {
            did: result.data.did.to_string(),
            handle: result.data.handle.to_string(),
            access_jwt: result.data.access_jwt.clone(),
            refresh_jwt: result.data.refresh_jwt.clone(),
        };

        let mut agent_guard = self.agent.write().unwrap();
        *agent_guard = Some(agent);

        Ok(session)
    }

    #[allow(dead_code)]
    pub async fn resume_session(&self, session: &Session) -> Result<(), ClientError> {
        let client = ReqwestClient::new(&self.service_url);
        let agent = AtpAgent::new(client, MemorySessionStore::default());

        let atrium_session = atrium_api::agent::Session::from(
            atrium_api::com::atproto::server::create_session::OutputData {
                access_jwt: session.access_jwt.clone(),
                active: None,
                did: session
                    .did
                    .parse()
                    .map_err(|e| ClientError::Auth(format!("invalid DID: {e}")))?,
                did_doc: None,
                email: None,
                email_auth_factor: None,
                email_confirmed: None,
                handle: session
                    .handle
                    .parse()
                    .map_err(|e| ClientError::Auth(format!("invalid handle: {e}")))?,
                refresh_jwt: session.refresh_jwt.clone(),
                status: None,
            },
        );

        agent
            .resume_session(atrium_session)
            .await
            .map_err(|e| ClientError::Auth(e.to_string()))?;

        let mut agent_guard = self.agent.write().unwrap();
        *agent_guard = Some(agent);

        Ok(())
    }

    #[allow(dead_code, clippy::await_holding_lock)]
    pub async fn is_authenticated(&self) -> bool {
        let agent_guard = self.agent.read().unwrap();
        if let Some(agent) = agent_guard.as_ref() {
            agent.get_session().await.is_some()
        } else {
            false
        }
    }

    #[allow(dead_code, clippy::await_holding_lock)]
    pub async fn session(&self) -> Option<Session> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref()?;
        let atrium_session = agent.get_session().await?;

        Some(Session {
            did: atrium_session.data.did.to_string(),
            handle: atrium_session.data.handle.to_string(),
            access_jwt: atrium_session.data.access_jwt.clone(),
            refresh_jwt: atrium_session.data.refresh_jwt.clone(),
        })
    }

    #[allow(dead_code)]
    pub async fn clear_session(&self) {
        let mut agent_guard = self.agent.write().unwrap();
        *agent_guard = None;
    }

    #[allow(clippy::await_holding_lock)]
    pub async fn get_timeline(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<Post>, Option<String>), ClientError> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;

        let params = atrium_api::app::bsky::feed::get_timeline::ParametersData {
            algorithm: None,
            cursor: cursor.map(String::from),
            limit: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .feed
            .get_timeline(params.into())
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;

        let posts: Vec<Post> = output
            .data
            .feed
            .into_iter()
            .map(|feed_view| self.convert_feed_view_post(feed_view))
            .collect();

        Ok((posts, output.data.cursor))
    }

    fn convert_feed_view_post(
        &self,
        feed_view: atrium_api::app::bsky::feed::defs::FeedViewPost,
    ) -> Post {
        let post_view = feed_view.data.post;
        let author = post_view.data.author;

        let (text, created_at) = self.extract_post_record(&post_view.data.record);

        Post {
            uri: post_view.data.uri,
            cid: post_view.data.cid.as_ref().to_string(),
            author: Profile {
                did: author.data.did.to_string(),
                handle: author.data.handle.to_string(),
                display_name: author.data.display_name.clone(),
                avatar: author.data.avatar.clone(),
            },
            text,
            created_at,
            reply_count: post_view.data.reply_count.map(|c| c as u32),
            repost_count: post_view.data.repost_count.map(|c| c as u32),
            like_count: post_view.data.like_count.map(|c| c as u32),
            indexed_at: post_view.data.indexed_at.as_str().to_string(),
        }
    }

    #[allow(clippy::await_holding_lock)]
    pub async fn get_profile(&self, actor: &str) -> Result<Profile, ClientError> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;

        let params = atrium_api::app::bsky::actor::get_profile::ParametersData {
            actor: actor
                .parse()
                .map_err(|e| ClientError::InvalidResponse(format!("invalid actor: {e}")))?,
        };

        let output = agent
            .api
            .app
            .bsky
            .actor
            .get_profile(params.into())
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;

        Ok(Profile {
            did: output.data.did.to_string(),
            handle: output.data.handle.to_string(),
            display_name: output.data.display_name.clone(),
            avatar: output.data.avatar.clone(),
        })
    }

    fn extract_post_record(&self, record: &atrium_api::types::Unknown) -> (String, String) {
        use atrium_api::types::Unknown;

        match record {
            Unknown::Object(map) => {
                let text = map
                    .get("text")
                    .and_then(|dm| serde_json::to_value(dm).ok())
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();

                let created_at = map
                    .get("createdAt")
                    .and_then(|dm| serde_json::to_value(dm).ok())
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();

                (text, created_at)
            }
            _ => (String::new(), String::new()),
        }
    }
}

impl Default for HangarClient {
    fn default() -> Self {
        Self::new()
    }
}
