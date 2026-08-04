// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::collapsible_if)]

use crate::atproto::facets;
use crate::atproto::types::{
    AuthMethod, ChatMessage, ChatReaction, ComposeData, Conversation, Embed, ExternalEmbed,
    ImageEmbed, LinkCardData, Notification, Post, PostgateConfig, Profile, QuoteEmbed,
    ReplyContext, RepostReason, SavedFeed, Session, ThreadgateConfig, ThreadgateRule, VideoEmbed,
};
use crate::config::DEFAULT_PDS;
use atrium_api::agent::atp_agent::AtpAgent;
use atrium_api::agent::atp_agent::store::MemorySessionStore;
use atrium_api::com::atproto::repo::{create_record, delete_record};
use atrium_api::types::Unknown;
use atrium_api::types::string::RecordKey;
use atrium_xrpc_client::reqwest::ReqwestClient;
use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// The stored session cannot be revived and only a fresh sign-in recovers.
    /// Distinct from `Auth` so the UI can say so instead of showing a bare
    /// login dialog with no explanation.
    #[error("session expired, sign in again")]
    ReauthRequired,
}

use crate::state::oauth::HangarOAuthSession;
use atrium_api::agent::Agent as OAuthAgent;

type CredentialAgent = AtpAgent<MemorySessionStore, ReqwestClient>;
type OAuthAgentType = OAuthAgent<HangarOAuthSession>;

/// Reply reference for post creation (root + parent URIs).
#[derive(Clone)]
pub struct ReplyRef {
    pub root_uri: String,
    pub root_cid: String,
    pub parent_uri: String,
    pub parent_cid: String,
}

/// Unread tallies behind the sidebar badges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnreadCounts {
    pub mentions: u32,
    pub activity: u32,
    pub chat: u32,
}

impl UnreadCounts {
    /// Tally one page of notifications and one of conversations.
    ///
    /// Notifications come as `(reason, is_read)`; the mentions/activity split
    /// mirrors the filter in `get_notifications`. Conversations come as
    /// `(unread_count, muted)`, and muted ones stay off the badge.
    pub(crate) fn tally<'a>(
        notifications: impl IntoIterator<Item = (&'a str, bool)>,
        convos: impl IntoIterator<Item = (i64, bool)>,
    ) -> Self {
        let mut counts = Self::default();
        for (reason, is_read) in notifications {
            if is_read {
                continue;
            }
            if matches!(reason, "mention" | "reply" | "quote") {
                counts.mentions += 1;
            } else {
                counts.activity += 1;
            }
        }
        for (unread, muted) in convos {
            if !muted {
                counts.chat = counts.chat.saturating_add(unread.max(0) as u32);
            }
        }
        counts
    }
}

/// Wraps atrium so the rest of the app only sees our own types.
/// Supports both credential-based (app password) and OAuth authentication.
/// Only one of `credential_agent` or `oauth_agent` is set at a time.
pub struct HangarClient {
    credential_agent: RwLock<Option<CredentialAgent>>,
    oauth_agent: RwLock<Option<OAuthAgentType>>,
    service_url: String,
    /// Set once the server rejects our credentials after atrium has already
    /// tried to refresh them. Latched, because a dead session fails every
    /// in-flight request at once.
    session_expired: AtomicBool,
}

/// Dispatch an expression through whichever agent is active.
/// Both agent types expose the same `.api.app.bsky.*` surface.
/// The macro duplicates `$body` for each variant so the compiler
/// monomorphizes the API calls for each concrete agent type.
macro_rules! with_agent {
    ($self:expr, $agent:ident => $body:expr) => {{
        // Try credential agent first (more common), then OAuth
        let cred_guard = $self.credential_agent.read().unwrap();
        if let Some($agent) = cred_guard.as_ref() {
            $body
        } else {
            drop(cred_guard);
            let oauth_guard = $self.oauth_agent.read().unwrap();
            let $agent = oauth_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;
            $body
        }
    }};
}

/// Like `with_agent!` but also provides the authenticated DID.
/// Use for methods that create/delete records (which need `repo: did`).
macro_rules! with_agent_and_did {
    ($self:expr, $agent:ident, $did:ident => $body:expr) => {{
        let cred_guard = $self.credential_agent.read().unwrap();
        if let Some($agent) = cred_guard.as_ref() {
            let $did = $agent
                .get_session()
                .await
                .ok_or(ClientError::NotAuthenticated)?
                .data
                .did
                .clone();
            $body
        } else {
            drop(cred_guard);
            let oauth_guard = $self.oauth_agent.read().unwrap();
            let $agent = oauth_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;
            let $did = $agent.did().await.ok_or(ClientError::NotAuthenticated)?;
            $body
        }
    }};
}

impl HangarClient {
    pub fn new() -> Self {
        Self {
            credential_agent: RwLock::new(None),
            oauth_agent: RwLock::new(None),
            service_url: DEFAULT_PDS.to_string(),
            session_expired: AtomicBool::new(false),
        }
    }

    #[allow(dead_code)]
    pub fn with_service(service_url: &str) -> Self {
        Self {
            credential_agent: RwLock::new(None),
            oauth_agent: RwLock::new(None),
            service_url: service_url.to_string(),
            session_expired: AtomicBool::new(false),
        }
    }

    /// Turn an XRPC failure into our own error, noticing a session the server
    /// will not accept any more.
    ///
    /// Both agents refresh, retry once, then discard the refresh error
    /// (atrium-oauth `oauth_session/inner.rs`, atrium-api `atp_agent/inner.rs`).
    /// A rejection reaching us is the second one, so latch it.
    fn xrpc_error<E: std::fmt::Debug + std::fmt::Display>(
        &self,
        e: atrium_api::xrpc::Error<E>,
    ) -> ClientError {
        use atrium_api::xrpc::error::{Error, XrpcError, XrpcErrorKind};

        let rejected = match &e {
            Error::Authentication(_) => true,
            Error::XrpcResponse(XrpcError { status, error }) => {
                *status == atrium_api::xrpc::http::StatusCode::UNAUTHORIZED
                    || matches!(
                        error,
                        Some(XrpcErrorKind::Undefined(body))
                            if matches!(
                                body.error.as_deref(),
                                Some("ExpiredToken" | "InvalidToken" | "AuthenticationRequired")
                            )
                    )
            }
            _ => false,
        };

        if rejected {
            self.session_expired.store(true, Ordering::Relaxed);
            return ClientError::ReauthRequired;
        }
        ClientError::Network(e.to_string())
    }

    /// Consume the expired-session flag. True at most once per dead session.
    pub fn take_session_expired(&self) -> bool {
        self.session_expired.swap(false, Ordering::Relaxed)
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
            auth: AuthMethod::AppPassword {
                access_jwt: result.data.access_jwt.clone(),
                refresh_jwt: result.data.refresh_jwt.clone(),
            },
        };

        // Clear any existing OAuth agent
        *self.oauth_agent.write().unwrap() = None;
        *self.credential_agent.write().unwrap() = Some(agent);
        self.session_expired.store(false, Ordering::Relaxed);

        Ok(session)
    }

    /// Forget whichever session is active.
    ///
    /// The client outlives the window, and so does the 30-second poll, so
    /// without this the previous account's agent keeps making requests. An
    /// OAuth agent that refreshes writes the whole session file back over
    /// whatever the next sign-in has written.
    pub fn sign_out(&self) {
        *self.credential_agent.write().unwrap() = None;
        *self.oauth_agent.write().unwrap() = None;
        // Nothing is signed in any more, so there is no expiry left to report.
        self.session_expired.store(false, Ordering::Relaxed);
    }

    /// Set an OAuth session as the active agent.
    pub async fn set_oauth_session(&self, oauth_session: HangarOAuthSession) -> Session {
        use atrium_api::agent::SessionManager;

        let did = oauth_session.did().await;
        let did_str = did.map(|d| d.to_string()).unwrap_or_default();
        let agent = OAuthAgent::new(oauth_session);

        // A new set of credentials starts clean, whatever the last ones did.
        self.session_expired.store(false, Ordering::Relaxed);

        // Clear any existing credential agent
        *self.credential_agent.write().unwrap() = None;
        *self.oauth_agent.write().unwrap() = Some(agent);

        Session {
            did: did_str,
            handle: String::new(), // Will be populated by profile fetch
            auth: AuthMethod::OAuth,
        }
    }

    /// Resume a session from stored data.
    /// For app passwords, restores from JWTs.
    /// For OAuth, restores from the persistent FileSessionStore.
    pub async fn resume_session(&self, session: &Session) -> Result<(), ClientError> {
        if matches!(session.auth, AuthMethod::OAuth) {
            return self.resume_oauth_session(session).await;
        }

        let (access_jwt, refresh_jwt) = match &session.auth {
            AuthMethod::AppPassword {
                access_jwt,
                refresh_jwt,
            } => (access_jwt.clone(), refresh_jwt.clone()),
            AuthMethod::OAuth => unreachable!("handled above"),
        };

        let client = ReqwestClient::new(&self.service_url);
        let agent = AtpAgent::new(client, MemorySessionStore::default());

        let atrium_session = atrium_api::com::atproto::server::create_session::Output::from(
            atrium_api::com::atproto::server::create_session::OutputData {
                access_jwt,
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
                refresh_jwt,
                status: None,
            },
        );

        agent
            .resume_session(atrium_session)
            .await
            .map_err(|e| ClientError::Auth(e.to_string()))?;

        // Clear any existing OAuth agent
        *self.oauth_agent.write().unwrap() = None;
        *self.credential_agent.write().unwrap() = Some(agent);

        Ok(())
    }

    /// Restore an OAuth session from the persistent store.
    async fn resume_oauth_session(&self, session: &Session) -> Result<(), ClientError> {
        use crate::state::oauth::{OAuthError, OAuthManager};
        use crate::state::session_store::FileSessionStore;
        use atrium_common::store::Store;

        let did: atrium_api::types::string::Did = session
            .did
            .parse()
            .map_err(|e| ClientError::Auth(format!("invalid DID: {e}")))?;

        let store = FileSessionStore::new();

        // OAuthClient::restore reads the stored session without refreshing, so
        // whatever token is on disk is what the first request goes out with.
        // If the server has stopped honouring it the retry path will only
        // refresh once our recorded expiry has passed, which can leave the
        // session permanently broken across restarts. Drop the cached expiry
        // so a rejected token can actually be replaced.
        if let Err(e) = store.invalidate_cached_expiry(&did) {
            eprintln!("hangar: could not reset stored token expiry: {e}");
        }

        let oauth_client = match OAuthManager::build_restore_client(store.clone(), &did) {
            Ok(client) => client,
            Err(OAuthError::MissingClientBinding) => {
                // The callback port is gone, so the client_id cannot be
                // rebuilt. Drop the row.
                if let Err(e) = store.del(&did).await {
                    eprintln!("hangar: could not discard the unusable OAuth session: {e}");
                }
                // Return the error without latching it. The caller is
                // already waiting; latching as well would raise it twice.
                return Err(ClientError::ReauthRequired);
            }
            Err(e) => {
                return Err(ClientError::Auth(format!(
                    "failed to build OAuth client: {e}"
                )));
            }
        };

        let oauth_session = OAuthManager::restore_session(&oauth_client, &did)
            .await
            .map_err(|e| ClientError::Auth(format!("failed to restore OAuth session: {e}")))?;

        // Set the restored session as the active agent
        self.set_oauth_session(oauth_session).await;
        Ok(())
    }

    #[allow(dead_code, clippy::await_holding_lock)]
    pub async fn is_authenticated(&self) -> bool {
        {
            let cred = self.credential_agent.read().unwrap();
            if let Some(agent) = cred.as_ref() {
                if agent.get_session().await.is_some() {
                    return true;
                }
            }
        }
        self.oauth_agent.read().unwrap().is_some()
    }

    #[allow(dead_code, clippy::await_holding_lock)]
    pub async fn session(&self) -> Option<Session> {
        {
            let cred = self.credential_agent.read().unwrap();
            if let Some(agent) = cred.as_ref() {
                let atrium_session = agent.get_session().await?;
                return Some(Session {
                    did: atrium_session.data.did.to_string(),
                    handle: atrium_session.data.handle.to_string(),
                    auth: AuthMethod::AppPassword {
                        access_jwt: atrium_session.data.access_jwt.clone(),
                        refresh_jwt: atrium_session.data.refresh_jwt.clone(),
                    },
                });
            }
        }
        {
            let oauth = self.oauth_agent.read().unwrap();
            let agent = oauth.as_ref()?;
            let did = agent.did().await?;
            Some(Session {
                did: did.to_string(),
                handle: String::new(),
                auth: AuthMethod::OAuth,
            })
        }
    }

    #[allow(dead_code)]
    pub async fn clear_session(&self) {
        *self.credential_agent.write().unwrap() = None;
        *self.oauth_agent.write().unwrap() = None;
    }

    #[allow(clippy::await_holding_lock)]
    pub async fn get_timeline(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<Post>, Option<String>), ClientError> {
        with_agent!(self, agent => {

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
            .map_err(|e| self.xrpc_error(e))?;

        let posts: Vec<Post> = output
            .data
            .feed
            .into_iter()
            .map(|feed_view| self.convert_feed_view_post(feed_view))
            .collect();

        Ok((posts, output.data.cursor))
        })
    }

    fn convert_feed_view_post(
        &self,
        feed_view: atrium_api::app::bsky::feed::defs::FeedViewPost,
    ) -> Post {
        let post_view = feed_view.data.post;
        let author = post_view.data.author;

        let (text, created_at) = self.extract_post_record(&post_view.data.record);

        // Extract rich embed (images, external links, videos, quotes)
        let embed = self.extract_embed(&post_view.data.embed);

        // Extract repost reason (who reposted this into the feed)
        let repost_reason = self.extract_repost_reason(&feed_view.data.reason);

        // Extract reply context (who this is replying to)
        let reply_context = self.extract_reply_context(&feed_view.data.reply);

        // Extract viewer state (like/repost URIs, bookmark flag)
        let (viewer_like, viewer_repost, viewer_bookmarked) = post_view
            .data
            .viewer
            .as_ref()
            .map(|v| {
                (
                    v.data.like.clone(),
                    v.data.repost.clone(),
                    v.data.bookmarked,
                )
            })
            .unwrap_or((None, None, None));

        Post {
            uri: post_view.data.uri,
            cid: post_view.data.cid.as_ref().to_string(),
            author: Profile::minimal(
                author.data.did.to_string(),
                author.data.handle.to_string(),
                author.data.display_name.clone(),
                author.data.avatar.clone(),
            ),
            text,
            created_at,
            reply_count: post_view.data.reply_count.map(|c| c as u32),
            repost_count: post_view.data.repost_count.map(|c| c as u32),
            like_count: post_view.data.like_count.map(|c| c as u32),
            indexed_at: post_view.data.indexed_at.as_str().to_string(),
            embed,
            viewer_like,
            viewer_repost,
            viewer_bookmarked,
            repost_reason,
            reply_context,
        }
    }

    /// Extract all embed types from a post view
    fn extract_embed(
        &self,
        embed: &Option<
            atrium_api::types::Union<atrium_api::app::bsky::feed::defs::PostViewEmbedRefs>,
        >,
    ) -> Option<Embed> {
        use atrium_api::app::bsky::feed::defs::PostViewEmbedRefs;
        use atrium_api::types::Union;

        let embed_ref = match embed.as_ref()? {
            Union::Refs(embed_ref) => embed_ref,
            // A lexicon atrium does not know. Read what we can off raw IPLD,
            // log the rest.
            Union::Unknown(unknown) => return Self::extract_unknown_embed(unknown, "post"),
        };

        match embed_ref {
            PostViewEmbedRefs::AppBskyEmbedImagesView(images_view) => {
                let images: Vec<ImageEmbed> = images_view
                    .data
                    .images
                    .iter()
                    .map(|img| ImageEmbed {
                        thumb: img.thumb.as_str().to_string(),
                        fullsize: img.fullsize.as_str().to_string(),
                        alt: img.alt.clone(),
                        aspect_ratio: img
                            .aspect_ratio
                            .as_ref()
                            .map(|ar| (ar.data.width.get() as u32, ar.data.height.get() as u32)),
                    })
                    .collect();
                Some(Embed::Images(images))
            }
            PostViewEmbedRefs::AppBskyEmbedExternalView(external_view) => {
                let ext = &external_view.data.external;
                Some(Embed::External(ExternalEmbed {
                    uri: ext.data.uri.clone(),
                    title: ext.data.title.clone(),
                    description: ext.data.description.clone(),
                    thumb: ext.data.thumb.clone(),
                }))
            }
            PostViewEmbedRefs::AppBskyEmbedVideoView(video_view) => {
                Some(Embed::Video(VideoEmbed {
                    playlist: video_view.data.playlist.clone(),
                    thumbnail: video_view.data.thumbnail.clone(),
                    alt: video_view.data.alt.clone(),
                    aspect_ratio: video_view
                        .data
                        .aspect_ratio
                        .as_ref()
                        .map(|ar| (ar.data.width.get() as u32, ar.data.height.get() as u32)),
                }))
            }
            PostViewEmbedRefs::AppBskyEmbedRecordView(record_view) => {
                self.extract_quote_embed(&record_view.data.record)
            }
            PostViewEmbedRefs::AppBskyEmbedRecordWithMediaView(rwm_view) => {
                Self::combine_record_with_media(
                    self.extract_quote_from_record(&rwm_view.data.record.data.record),
                    Self::extract_media_embed(&rwm_view.data.media),
                )
            }
        }
    }

    /// Put a record-with-media view back together from whichever halves
    /// resolved.
    ///
    /// Never `and_then` the two together. A quote can be blocked, deleted or
    /// detached, and a media half can be an unknown lexicon. Requiring both
    /// returned `embed: None`, and `PostRow` hides the embed container for
    /// `None`, so the post rendered with a blank body.
    fn combine_record_with_media(quote: Option<QuoteEmbed>, media: Option<Embed>) -> Option<Embed> {
        match (quote, media) {
            (Some(quote), Some(media)) => Some(Embed::QuoteWithMedia {
                quote,
                media: Box::new(media),
            }),
            (Some(quote), None) => Some(Embed::Quote(quote)),
            (None, Some(media)) => Some(media),
            (None, None) => None,
        }
    }

    /// Best effort at an embed whose `$type` is not in this build's lexicons.
    ///
    /// `app.bsky.embed.gallery` supersedes `app.bsky.embed.images`, and
    /// atrium-api 0.25 (the newest published) has no module for it, so it
    /// arrives as `Union::Unknown`. Dropping it was ~0.8% of embedded posts in
    /// a live sample, each rendering as a blank body. Its items are
    /// `{thumbnail, fullsize, alt, aspectRatio}`, which is [`ImageEmbed`].
    ///
    /// Anything else is named on stderr.
    fn extract_unknown_embed(
        unknown: &atrium_api::types::UnknownData,
        context: &str,
    ) -> Option<Embed> {
        if unknown.r#type != "app.bsky.embed.gallery#view" {
            eprintln!(
                "hangar: unsupported {context} embed type {:?}; showing the post without it",
                unknown.r#type
            );
            return None;
        }

        #[derive(serde::Deserialize)]
        struct GalleryView {
            #[serde(default)]
            items: Vec<GalleryItem>,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct GalleryItem {
            thumbnail: String,
            fullsize: String,
            #[serde(default)]
            alt: String,
            #[serde(default)]
            aspect_ratio: Option<GalleryAspect>,
        }
        #[derive(serde::Deserialize)]
        struct GalleryAspect {
            width: u32,
            height: u32,
        }

        let view: GalleryView = serde_json::to_value(&unknown.data)
            .and_then(serde_json::from_value)
            .map_err(|e| eprintln!("hangar: could not read a gallery embed: {e}"))
            .ok()?;

        if view.items.is_empty() {
            return None;
        }

        Some(Embed::Images(
            view.items
                .into_iter()
                .map(|item| ImageEmbed {
                    thumb: item.thumbnail,
                    fullsize: item.fullsize,
                    alt: item.alt,
                    aspect_ratio: item
                        .aspect_ratio
                        .filter(|a| a.width > 0 && a.height > 0)
                        .map(|a| (a.width, a.height)),
                })
                .collect(),
        ))
    }

    /// Extract quote embed from record view
    fn extract_quote_embed(
        &self,
        record: &atrium_api::types::Union<atrium_api::app::bsky::embed::record::ViewRecordRefs>,
    ) -> Option<Embed> {
        let quote = self.extract_quote_from_record(record)?;
        Some(Embed::Quote(quote))
    }

    /// Extract QuoteEmbed from record union
    fn extract_quote_from_record(
        &self,
        record: &atrium_api::types::Union<atrium_api::app::bsky::embed::record::ViewRecordRefs>,
    ) -> Option<QuoteEmbed> {
        use atrium_api::app::bsky::embed::record::ViewRecordRefs;
        use atrium_api::types::Union;

        match record {
            Union::Refs(ViewRecordRefs::ViewRecord(view_record)) => {
                let data = &view_record.data;
                let (text, _) = self.extract_post_record(&data.value);

                // Extract nested embeds if present
                let nested_embed = data
                    .embeds
                    .as_ref()
                    .and_then(|embeds| embeds.first())
                    .and_then(|e| self.extract_record_embed(e));

                Some(QuoteEmbed {
                    uri: data.uri.clone(),
                    cid: data.cid.as_ref().to_string(),
                    author: Profile::minimal(
                        data.author.data.did.to_string(),
                        data.author.data.handle.to_string(),
                        data.author.data.display_name.clone(),
                        data.author.data.avatar.clone(),
                    ),
                    text,
                    indexed_at: data.indexed_at.as_str().to_string(),
                    embed: nested_embed.map(Box::new),
                })
            }
            // ViewNotFound, ViewBlocked, ViewDetached all map to None
            _ => None,
        }
    }

    /// Extract embed from record view embeds (for nested embeds in quotes)
    fn extract_record_embed(
        &self,
        embed: &atrium_api::types::Union<
            atrium_api::app::bsky::embed::record::ViewRecordEmbedsItem,
        >,
    ) -> Option<Embed> {
        use atrium_api::app::bsky::embed::record::ViewRecordEmbedsItem;
        use atrium_api::types::Union;

        match embed {
            Union::Refs(ViewRecordEmbedsItem::AppBskyEmbedImagesView(images_view)) => {
                let images: Vec<ImageEmbed> = images_view
                    .data
                    .images
                    .iter()
                    .map(|img| ImageEmbed {
                        thumb: img.thumb.as_str().to_string(),
                        fullsize: img.fullsize.as_str().to_string(),
                        alt: img.alt.clone(),
                        aspect_ratio: img
                            .aspect_ratio
                            .as_ref()
                            .map(|ar| (ar.data.width.get() as u32, ar.data.height.get() as u32)),
                    })
                    .collect();
                Some(Embed::Images(images))
            }
            Union::Refs(ViewRecordEmbedsItem::AppBskyEmbedExternalView(external_view)) => {
                let ext = &external_view.data.external;
                Some(Embed::External(ExternalEmbed {
                    uri: ext.data.uri.clone(),
                    title: ext.data.title.clone(),
                    description: ext.data.description.clone(),
                    thumb: ext.data.thumb.clone(),
                }))
            }
            Union::Refs(ViewRecordEmbedsItem::AppBskyEmbedVideoView(video_view)) => {
                Some(Embed::Video(VideoEmbed {
                    playlist: video_view.data.playlist.clone(),
                    thumbnail: video_view.data.thumbnail.clone(),
                    alt: video_view.data.alt.clone(),
                    aspect_ratio: video_view
                        .data
                        .aspect_ratio
                        .as_ref()
                        .map(|ar| (ar.data.width.get() as u32, ar.data.height.get() as u32)),
                }))
            }
            Union::Refs(ViewRecordEmbedsItem::AppBskyEmbedRecordView(record_view)) => {
                self.extract_quote_embed(&record_view.data.record)
            }
            Union::Refs(ViewRecordEmbedsItem::AppBskyEmbedRecordWithMediaView(rwm_view)) => {
                Self::combine_record_with_media(
                    self.extract_quote_from_record(&rwm_view.data.record.data.record),
                    Self::extract_media_embed(&rwm_view.data.media),
                )
            }
            Union::Unknown(unknown) => Self::extract_unknown_embed(unknown, "quoted post"),
        }
    }

    /// Extract media embed from record-with-media view
    fn extract_media_embed(
        media: &atrium_api::types::Union<
            atrium_api::app::bsky::embed::record_with_media::ViewMediaRefs,
        >,
    ) -> Option<Embed> {
        use atrium_api::app::bsky::embed::record_with_media::ViewMediaRefs;
        use atrium_api::types::Union;

        match media {
            Union::Refs(ViewMediaRefs::AppBskyEmbedImagesView(images_view)) => {
                let images: Vec<ImageEmbed> = images_view
                    .data
                    .images
                    .iter()
                    .map(|img| ImageEmbed {
                        thumb: img.thumb.as_str().to_string(),
                        fullsize: img.fullsize.as_str().to_string(),
                        alt: img.alt.clone(),
                        aspect_ratio: img
                            .aspect_ratio
                            .as_ref()
                            .map(|ar| (ar.data.width.get() as u32, ar.data.height.get() as u32)),
                    })
                    .collect();
                Some(Embed::Images(images))
            }
            Union::Refs(ViewMediaRefs::AppBskyEmbedVideoView(video_view)) => {
                Some(Embed::Video(VideoEmbed {
                    playlist: video_view.data.playlist.clone(),
                    thumbnail: video_view.data.thumbnail.clone(),
                    alt: video_view.data.alt.clone(),
                    aspect_ratio: video_view
                        .data
                        .aspect_ratio
                        .as_ref()
                        .map(|ar| (ar.data.width.get() as u32, ar.data.height.get() as u32)),
                }))
            }
            // The third variant, and the one that was missing. "Quote a post,
            // reply with a GIF" is the commonest record-with-media shape, since
            // GIFs are `external` views. Dropping it gave the empty post body,
            // and took ordinary link cards on a quote with it.
            Union::Refs(ViewMediaRefs::AppBskyEmbedExternalView(external_view)) => {
                let ext = &external_view.data.external;
                Some(Embed::External(ExternalEmbed {
                    uri: ext.data.uri.clone(),
                    title: ext.data.title.clone(),
                    description: ext.data.description.clone(),
                    thumb: ext.data.thumb.clone(),
                }))
            }
            Union::Unknown(unknown) => Self::extract_unknown_embed(unknown, "attached media"),
        }
    }

    /// Extract repost reason (who reposted this into the feed)
    fn extract_repost_reason(
        &self,
        reason: &Option<
            atrium_api::types::Union<atrium_api::app::bsky::feed::defs::FeedViewPostReasonRefs>,
        >,
    ) -> Option<RepostReason> {
        use atrium_api::app::bsky::feed::defs::FeedViewPostReasonRefs;
        use atrium_api::types::Union;

        let Union::Refs(FeedViewPostReasonRefs::ReasonRepost(repost)) = reason.as_ref()? else {
            return None;
        };

        Some(RepostReason {
            by: Profile::minimal(
                repost.data.by.data.did.to_string(),
                repost.data.by.data.handle.to_string(),
                repost.data.by.data.display_name.clone(),
                repost.data.by.data.avatar.clone(),
            ),
            indexed_at: repost.data.indexed_at.as_str().to_string(),
        })
    }

    /// Extract reply context (parent and root authors)
    fn extract_reply_context(
        &self,
        reply: &Option<atrium_api::app::bsky::feed::defs::ReplyRef>,
    ) -> Option<ReplyContext> {
        use atrium_api::app::bsky::feed::defs::ReplyRefParentRefs;
        use atrium_api::app::bsky::feed::defs::ReplyRefRootRefs;
        use atrium_api::types::Union;

        let reply = reply.as_ref()?;

        // Extract parent author
        let parent_author = match &reply.data.parent {
            Union::Refs(ReplyRefParentRefs::PostView(pv)) => Profile::minimal(
                pv.data.author.data.did.to_string(),
                pv.data.author.data.handle.to_string(),
                pv.data.author.data.display_name.clone(),
                pv.data.author.data.avatar.clone(),
            ),
            Union::Refs(ReplyRefParentRefs::NotFoundPost(_)) => return None,
            Union::Refs(ReplyRefParentRefs::BlockedPost(_)) => return None,
            _ => return None,
        };

        // Extract root author
        let root_author = match &reply.data.root {
            Union::Refs(ReplyRefRootRefs::PostView(pv)) => Profile::minimal(
                pv.data.author.data.did.to_string(),
                pv.data.author.data.handle.to_string(),
                pv.data.author.data.display_name.clone(),
                pv.data.author.data.avatar.clone(),
            ),
            Union::Refs(ReplyRefRootRefs::NotFoundPost(_)) => return None,
            Union::Refs(ReplyRefRootRefs::BlockedPost(_)) => return None,
            _ => return None,
        };

        Some(ReplyContext {
            parent_author,
            root_author,
        })
    }

    #[allow(clippy::await_holding_lock)]
    pub async fn get_profile(&self, actor: &str) -> Result<Profile, ClientError> {
        with_agent!(self, agent => {

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
            .map_err(|e| self.xrpc_error(e))?;

        // Extract viewer state
        let viewer_following = output
            .data
            .viewer
            .as_ref()
            .and_then(|v| v.data.following.clone());
        let viewer_followed_by = output
            .data
            .viewer
            .as_ref()
            .and_then(|v| v.data.followed_by.clone());

        Ok(Profile {
            did: output.data.did.to_string(),
            handle: output.data.handle.to_string(),
            display_name: output.data.display_name.clone(),
            avatar: output.data.avatar.clone(),
            banner: output.data.banner.clone(),
            description: output.data.description.clone(),
            followers_count: output.data.followers_count.map(|c| c as u32),
            following_count: output.data.follows_count.map(|c| c as u32),
            posts_count: output.data.posts_count.map(|c| c as u32),
            viewer_following,
            viewer_followed_by,
        })
        })
    }

    /// Fetch multiple profiles in a single batch request (up to 25 at a time)
    #[allow(clippy::await_holding_lock)]
    pub async fn get_profiles(&self, actors: &[String]) -> Result<Vec<Profile>, ClientError> {
        if actors.is_empty() {
            return Ok(vec![]);
        }

        with_agent!(self, agent => {

        // ATProto limits to 25 profiles per request
        let actors: Vec<_> = actors
            .iter()
            .take(25)
            .map(|a| {
                a.parse()
                    .map_err(|e| ClientError::InvalidResponse(format!("invalid actor: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let params = atrium_api::app::bsky::actor::get_profiles::ParametersData { actors };

        let output = agent
            .api
            .app
            .bsky
            .actor
            .get_profiles(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let profiles = output
            .data
            .profiles
            .into_iter()
            .map(|p| {
                let viewer_following = p.viewer.as_ref().and_then(|v| v.data.following.clone());
                let viewer_followed_by = p.viewer.as_ref().and_then(|v| v.data.followed_by.clone());

                Profile {
                    did: p.did.to_string(),
                    handle: p.handle.to_string(),
                    display_name: p.display_name.clone(),
                    avatar: p.avatar.clone(),
                    banner: p.banner.clone(),
                    description: p.description.clone(),
                    followers_count: p.followers_count.map(|c| c as u32),
                    following_count: p.follows_count.map(|c| c as u32),
                    posts_count: p.posts_count.map(|c| c as u32),
                    viewer_following,
                    viewer_followed_by,
                }
            })
            .collect();

        Ok(profiles)
        })
    }

    /// Like a post and return the URI of the created like record
    #[allow(clippy::await_holding_lock)]
    pub async fn like(&self, uri: &str, cid: &str) -> Result<String, ClientError> {
        with_agent_and_did!(self, agent, did => {
        // DID available as `did`

        let record_json = serde_json::json!({
            "$type": "app.bsky.feed.like",
            "subject": { "uri": uri, "cid": cid },
            "createdAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });
        let record: Unknown = serde_json::from_value(record_json)
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        let collection = atrium_api::types::string::Nsid::new("app.bsky.feed.like".to_string())
            .map_err(|_| ClientError::InvalidResponse("invalid collection".into()))?;

        let input = create_record::InputData {
            collection,
            record,
            repo: did.clone().into(),
            rkey: None,
            swap_commit: None,
            validate: None,
        };

        let output = agent
            .api
            .com
            .atproto
            .repo
            .create_record(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(output.data.uri.to_string())
        })
    }

    /// Repost a post and return the URI of the created repost record
    #[allow(clippy::await_holding_lock)]
    pub async fn repost(&self, uri: &str, cid: &str) -> Result<String, ClientError> {
        with_agent_and_did!(self, agent, did => {
        // DID available as `did`

        let record_json = serde_json::json!({
            "$type": "app.bsky.feed.repost",
            "subject": { "uri": uri, "cid": cid },
            "createdAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });
        let record: Unknown = serde_json::from_value(record_json)
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        let collection = atrium_api::types::string::Nsid::new("app.bsky.feed.repost".to_string())
            .map_err(|_| ClientError::InvalidResponse("invalid collection".into()))?;

        let input = create_record::InputData {
            collection,
            record,
            repo: did.clone().into(),
            rkey: None,
            swap_commit: None,
            validate: None,
        };

        let output = agent
            .api
            .com
            .atproto
            .repo
            .create_record(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(output.data.uri.to_string())
        })
    }

    /// Unlike a post by deleting the like record
    /// `like_uri` is the AT-URI of the like record (from viewer_like)
    #[allow(clippy::await_holding_lock)]
    pub async fn unlike(&self, like_uri: &str) -> Result<(), ClientError> {
        self.delete_record(like_uri, "app.bsky.feed.like").await
    }

    /// Delete a repost by deleting the repost record
    /// `repost_uri` is the AT-URI of the repost record (from viewer_repost)
    #[allow(clippy::await_holding_lock)]
    pub async fn delete_repost(&self, repost_uri: &str) -> Result<(), ClientError> {
        self.delete_record(repost_uri, "app.bsky.feed.repost").await
    }

    /// Delete one of the user's own posts
    /// `post_uri` is the AT-URI of the post record
    #[allow(clippy::await_holding_lock)]
    pub async fn delete_post(&self, post_uri: &str) -> Result<(), ClientError> {
        self.delete_record(post_uri, "app.bsky.feed.post").await
    }

    /// Generic delete record helper
    #[allow(clippy::await_holding_lock)]
    async fn delete_record(&self, record_uri: &str, collection: &str) -> Result<(), ClientError> {
        with_agent!(self, agent => {

        let (repo, rkey) = parse_record_uri(record_uri, collection)?;

        let collection = atrium_api::types::string::Nsid::new(collection.to_string())
            .map_err(|_| ClientError::InvalidResponse("invalid collection".into()))?;

        let input = delete_record::InputData {
            collection,
            repo: repo
                .parse()
                .map_err(|_| ClientError::InvalidResponse("invalid repo DID".into()))?,
            rkey: rkey
                .parse::<RecordKey>()
                .map_err(|_| ClientError::InvalidResponse("invalid record key".into()))?,
            swap_commit: None,
            swap_record: None,
        };

        agent
            .api
            .com
            .atproto
            .repo
            .delete_record(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;
        Ok(())
        })
    }

    /// Resolve an AT Protocol handle to a DID.
    #[allow(clippy::await_holding_lock)]
    pub async fn resolve_handle(&self, handle: &str) -> Result<String, ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::com::atproto::identity::resolve_handle::ParametersData {
            handle: handle
                .parse()
                .map_err(|_| ClientError::InvalidResponse("invalid handle".into()))?,
        };

        let output = agent
            .api
            .com
            .atproto
            .identity
            .resolve_handle(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(output.data.did.to_string())
        })
    }

    /// Parse text for facets and resolve any mention handles to DIDs.
    /// Must be called before acquiring the agent lock for create_record.
    async fn resolve_facets(&self, text: &str) -> (Vec<facets::RawFacet>, HashMap<String, String>) {
        let raw_facets = facets::parse_facets(text);
        let mut resolved_dids = HashMap::new();

        for raw in &raw_facets {
            if let facets::RawFacet::Mention { handle, .. } = raw {
                if let Ok(did) = self.resolve_handle(handle).await {
                    resolved_dids.insert(handle.clone(), did);
                }
                // A failed resolution is skipped and produces no facet
            }
        }

        (raw_facets, resolved_dids)
    }

    #[allow(clippy::await_holding_lock)]
    pub async fn create_post(&self, text: &str) -> Result<(), ClientError> {
        self.create_post_internal(text, None).await
    }

    #[allow(clippy::await_holding_lock)]
    pub async fn create_reply(
        &self,
        text: &str,
        parent_uri: &str,
        parent_cid: &str,
    ) -> Result<(), ClientError> {
        // For a reply, we need the root of the thread.
        // If replying to a top-level post, root = parent.
        // If replying to a reply, we'd need to fetch the thread to get the root.
        // For now, we treat parent as root (works for direct replies to top-level posts).
        let reply = ReplyRef {
            root_uri: parent_uri.to_string(),
            root_cid: parent_cid.to_string(),
            parent_uri: parent_uri.to_string(),
            parent_cid: parent_cid.to_string(),
        };
        self.create_post_internal(text, Some(reply)).await
    }

    /// Create a quote post (post with an embedded reference to another post)
    pub async fn create_quote_post(
        &self,
        text: &str,
        quoted_uri: &str,
        quoted_cid: &str,
    ) -> Result<(), ClientError> {
        let data = ComposeData {
            text: text.to_string(),
            ..Default::default()
        };
        self.create_post_with_data(&data, None, Some((quoted_uri, quoted_cid)))
            .await?;
        Ok(())
    }

    /// Upload a blob (image/video) to the PDS and return the blob ref as JSON.
    #[allow(clippy::await_holding_lock)]
    pub async fn upload_blob(
        &self,
        data: Vec<u8>,
        mime_type: &str,
    ) -> Result<serde_json::Value, ClientError> {
        with_agent!(self, agent => {

        let output = agent
            .api
            .com
            .atproto
            .repo
            .upload_blob(data)
            .await
            .map_err(|e| self.xrpc_error(e))?;

        // Serialize the BlobRef to JSON; atrium's BlobRef implements Serialize.
        // The output contains the blob reference we need for embeds.
        let blob_json = serde_json::to_value(&output.data.blob)
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        // Ensure the mime_type in the blob ref matches what we uploaded
        // (atrium may set it from Content-Type header, but let's be explicit)
        let mut blob = blob_json;
        if let Some(obj) = blob.as_object_mut() {
            obj.insert("mimeType".to_string(), serde_json::json!(mime_type));
        }

        Ok(blob)
        })
    }

    /// Create a post with full compose data (images, language, CW, threadgate, etc.)
    /// Returns `(uri, cid)` of the created post.
    #[allow(clippy::await_holding_lock)]
    pub async fn create_post_with_data(
        &self,
        data: &ComposeData,
        reply: Option<ReplyRef>,
        quote: Option<(&str, &str)>,
    ) -> Result<(String, String), ClientError> {
        // Resolve facets before acquiring the agent lock
        let (raw_facets, resolved_dids) = self.resolve_facets(&data.text).await;

        // Upload image blobs (if any) before acquiring the agent lock for create_record
        let mut image_blobs = Vec::new();
        for img in &data.images {
            let blob_ref = self.upload_blob(img.data.clone(), &img.mime_type).await?;
            image_blobs.push((blob_ref, img.alt_text.clone(), img.width, img.height));
        }

        // Upload link card thumbnail (if present)
        let link_card_thumb_blob = if let Some(ref card) = data.link_card {
            if let Some((ref thumb_data, ref thumb_mime)) = card.thumb {
                Some(self.upload_blob(thumb_data.clone(), thumb_mime).await?)
            } else {
                None
            }
        } else {
            None
        };

        with_agent_and_did!(self, agent, did => {
        // DID available as `did`

        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        let mut record_json = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": data.text,
            "createdAt": now
        });

        // Reply reference
        if let Some(r) = &reply {
            record_json["reply"] = serde_json::json!({
                "root": { "uri": r.root_uri, "cid": r.root_cid },
                "parent": { "uri": r.parent_uri, "cid": r.parent_cid }
            });
        }

        // Facets
        let facets_json = facets::build_facets_json(&raw_facets, &resolved_dids);
        if let serde_json::Value::Array(ref arr) = facets_json {
            if !arr.is_empty() {
                record_json["facets"] = facets_json;
            }
        }

        // Language tags
        if !data.langs.is_empty() {
            record_json["langs"] = serde_json::json!(data.langs);
        }

        // Content warning (self-labels)
        if let Some(ref cw) = data.content_warning {
            record_json["labels"] = serde_json::json!({
                "$type": "com.atproto.label.defs#selfLabels",
                "values": [{ "val": cw }]
            });
        }

        // Build embed based on what's attached.
        // Priority: images > link card (images win when both present).
        // Quote embed can be combined with media via recordWithMedia.
        let media_embed = if !image_blobs.is_empty() {
            // Image embed
            let images: Vec<serde_json::Value> = image_blobs
                .iter()
                .map(|(blob, alt, w, h)| {
                    serde_json::json!({
                        "alt": alt,
                        "image": blob,
                        "aspectRatio": { "width": w, "height": h }
                    })
                })
                .collect();
            Some(serde_json::json!({
                "$type": "app.bsky.embed.images",
                "images": images
            }))
        } else if let Some(ref card) = data.link_card {
            // External link card embed
            let mut external = serde_json::json!({
                "uri": card.url,
                "title": card.title,
                "description": card.description
            });
            if let Some(ref thumb_blob) = link_card_thumb_blob {
                external["thumb"] = thumb_blob.clone();
            }
            Some(serde_json::json!({
                "$type": "app.bsky.embed.external",
                "external": external
            }))
        } else {
            None
        };

        if let Some((quoted_uri, quoted_cid)) = quote {
            let quote_record = serde_json::json!({
                "uri": quoted_uri,
                "cid": quoted_cid
            });
            if let Some(media) = media_embed {
                // Quote + media → recordWithMedia
                record_json["embed"] = serde_json::json!({
                    "$type": "app.bsky.embed.recordWithMedia",
                    "record": {
                        "$type": "app.bsky.embed.record",
                        "record": quote_record
                    },
                    "media": media
                });
            } else {
                // Quote only
                record_json["embed"] = serde_json::json!({
                    "$type": "app.bsky.embed.record",
                    "record": quote_record
                });
            }
        } else if let Some(media) = media_embed {
            record_json["embed"] = media;
        }

        let record: Unknown = serde_json::from_value(record_json)
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        let collection = atrium_api::types::string::Nsid::new("app.bsky.feed.post".to_string())
            .map_err(|_| ClientError::InvalidResponse("invalid collection".into()))?;

        let input = create_record::InputData {
            collection,
            record,
            repo: did.clone().into(),
            rkey: None,
            swap_commit: None,
            validate: None,
        };

        let output = agent
            .api
            .com
            .atproto
            .repo
            .create_record(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let post_uri = output.data.uri.to_string();
        let post_cid = output.data.cid.as_ref().to_string();

        // Gate methods acquire their own read locks (RwLock allows concurrent reads)

        // Create threadgate record if configured
        if let Some(ref tg) = data.threadgate {
            self.create_threadgate(&post_uri, tg, &now).await?;
        }

        // Create postgate record if configured
        if let Some(ref pg) = data.postgate {
            if pg.disable_quoting {
                self.create_postgate(&post_uri, pg, &now).await?;
            }
        }

        Ok((post_uri, post_cid))
        })
    }

    /// Create a threadgate record controlling who can reply to a post.
    #[allow(clippy::await_holding_lock)]
    async fn create_threadgate(
        &self,
        post_uri: &str,
        config: &ThreadgateConfig,
        created_at: &str,
    ) -> Result<(), ClientError> {
        with_agent_and_did!(self, agent, did => {
        // DID available as `did`

        let allow_rules: Vec<serde_json::Value> = config
            .allow_rules
            .iter()
            .map(|r| match r {
                ThreadgateRule::MentionRule => {
                    serde_json::json!({"$type": "app.bsky.feed.threadgate#mentionRule"})
                }
                ThreadgateRule::FollowingRule => {
                    serde_json::json!({"$type": "app.bsky.feed.threadgate#followingRule"})
                }
                ThreadgateRule::FollowersRule => {
                    serde_json::json!({"$type": "app.bsky.feed.threadgate#followerRule"})
                }
            })
            .collect();

        let record_json = serde_json::json!({
            "$type": "app.bsky.feed.threadgate",
            "post": post_uri,
            "allow": allow_rules,
            "createdAt": created_at
        });

        let record: Unknown = serde_json::from_value(record_json)
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        // Threadgate rkey must match the post's rkey
        let rkey = post_uri
            .rsplit('/')
            .next()
            .map(|s| s.parse::<RecordKey>())
            .transpose()
            .map_err(|_| ClientError::InvalidResponse("invalid record key".into()))?;

        let collection =
            atrium_api::types::string::Nsid::new("app.bsky.feed.threadgate".to_string())
                .map_err(|_| ClientError::InvalidResponse("invalid collection".into()))?;

        let input = create_record::InputData {
            collection,
            record,
            repo: did.clone().into(),
            rkey,
            swap_commit: None,
            validate: None,
        };

        agent
            .api
            .com
            .atproto
            .repo
            .create_record(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;
        Ok(())
        })
    }

    /// Create a postgate record controlling quoting of a post.
    #[allow(clippy::await_holding_lock)]
    async fn create_postgate(
        &self,
        post_uri: &str,
        _config: &PostgateConfig,
        created_at: &str,
    ) -> Result<(), ClientError> {
        with_agent_and_did!(self, agent, did => {
        // DID available as `did`

        let record_json = serde_json::json!({
            "$type": "app.bsky.feed.postgate",
            "post": post_uri,
            "embeddingRules": [{"$type": "app.bsky.feed.postgate#disableRule"}],
            "createdAt": created_at
        });

        let record: Unknown = serde_json::from_value(record_json)
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        let rkey = post_uri
            .rsplit('/')
            .next()
            .map(|s| s.parse::<RecordKey>())
            .transpose()
            .map_err(|_| ClientError::InvalidResponse("invalid record key".into()))?;

        let collection = atrium_api::types::string::Nsid::new("app.bsky.feed.postgate".to_string())
            .map_err(|_| ClientError::InvalidResponse("invalid collection".into()))?;

        let input = create_record::InputData {
            collection,
            record,
            repo: did.clone().into(),
            rkey,
            swap_commit: None,
            validate: None,
        };

        agent
            .api
            .com
            .atproto
            .repo
            .create_record(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;
        Ok(())
        })
    }

    /// Create a thread (multiple posts, each replying to the previous).
    /// Returns `Vec<(uri, cid)>` for all created posts.
    pub async fn create_thread(
        &self,
        posts: &[ComposeData],
        reply_to: Option<ReplyRef>,
    ) -> Result<Vec<(String, String)>, ClientError> {
        if posts.is_empty() {
            return Ok(Vec::new());
        }

        let mut results: Vec<(String, String)> = Vec::new();
        let mut root_uri = String::new();
        let mut root_cid = String::new();

        for (i, post_data) in posts.iter().enumerate() {
            let reply = if i == 0 {
                reply_to.clone()
            } else {
                let (parent_uri, parent_cid) = &results[i - 1];
                Some(ReplyRef {
                    root_uri: root_uri.clone(),
                    root_cid: root_cid.clone(),
                    parent_uri: parent_uri.clone(),
                    parent_cid: parent_cid.clone(),
                })
            };

            let (uri, cid) = self.create_post_with_data(post_data, reply, None).await?;

            if i == 0 {
                root_uri = uri.clone();
                root_cid = cid.clone();
            }

            results.push((uri, cid));
        }

        Ok(results)
    }

    #[allow(clippy::await_holding_lock)]
    async fn create_post_internal(
        &self,
        text: &str,
        reply: Option<ReplyRef>,
    ) -> Result<(), ClientError> {
        let data = ComposeData {
            text: text.to_string(),
            ..Default::default()
        };
        self.create_post_with_data(&data, reply, None).await?;
        Ok(())
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

    /// Get the user's saved/pinned feeds from preferences
    #[allow(clippy::await_holding_lock)]
    pub async fn get_saved_feeds(&self) -> Result<Vec<SavedFeed>, ClientError> {
        with_agent!(self, agent => {

        let output = agent
            .api
            .app
            .bsky
            .actor
            .get_preferences(
                atrium_api::app::bsky::actor::get_preferences::ParametersData {}.into(),
            )
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let mut feeds = vec![SavedFeed::home()];

        // Parse preferences to find saved feeds
        for pref in output.data.preferences.iter() {
            use atrium_api::app::bsky::actor::defs::PreferencesItem;
            use atrium_api::types::Union;

            if let Union::Refs(PreferencesItem::SavedFeedsPrefV2(saved_feeds_pref)) = pref {
                for item in &saved_feeds_pref.data.items {
                    // Only include pinned feeds (shown in feed selector)
                    if item.data.pinned {
                        let feed_type = item.data.r#type.clone();
                        let uri = item.data.value.clone();

                        // Skip timeline type as we already have "Following"
                        if feed_type == "timeline" {
                            continue;
                        }

                        // We'll need to fetch the display name separately
                        // For now, use the rkey from URI as a fallback name
                        let display_name = uri.split('/').next_back().unwrap_or("Feed").to_string();

                        feeds.push(SavedFeed {
                            feed_type,
                            uri,
                            display_name,
                            description: None,
                            pinned: true,
                        });
                    }
                }
            }
        }

        // Now fetch display names for the feed generators
        let feed_uris: Vec<String> = feeds
            .iter()
            .filter(|f| !f.is_home())
            .map(|f| f.uri.clone())
            .collect();

        if !feed_uris.is_empty() {
            if let Ok(generators) = self.get_feed_generators_internal(&feed_uris).await {
                for (uri, name, description) in generators {
                    if let Some(feed) = feeds.iter_mut().find(|f| f.uri == uri) {
                        feed.display_name = name;
                        feed.description = description;
                    }
                }
            }
        }

        Ok(feeds)
        })
    }

    /// Internal helper to get feed generator metadata (uri, display_name, description)
    #[allow(clippy::await_holding_lock, dead_code)]
    async fn get_feed_generators_internal(
        &self,
        uris: &[String],
    ) -> Result<Vec<(String, String, Option<String>)>, ClientError> {
        with_agent!(self, agent => {
        let params = atrium_api::app::bsky::feed::get_feed_generators::ParametersData {
            feeds: uris.iter().map(|s| s.parse().unwrap()).collect(),
        };

        let output = agent
            .api
            .app
            .bsky
            .feed
            .get_feed_generators(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(output
            .data
            .feeds
            .into_iter()
            .map(|f| (f.data.uri, f.data.display_name, f.data.description))
            .collect())
        })
    }

    /// Fetch a custom feed by its AT-URI
    #[allow(clippy::await_holding_lock)]
    pub async fn get_feed(
        &self,
        feed_uri: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Post>, Option<String>), ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::feed::get_feed::ParametersData {
            feed: feed_uri
                .parse()
                .map_err(|e| ClientError::InvalidResponse(format!("invalid feed URI: {e}")))?,
            cursor: cursor.map(String::from),
            limit: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .feed
            .get_feed(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let posts: Vec<Post> = output
            .data
            .feed
            .into_iter()
            .map(|feed_view| self.convert_feed_view_post(feed_view))
            .collect();

        Ok((posts, output.data.cursor))
        })
    }

    /// Get a post thread (the main post and its replies)
    #[allow(clippy::await_holding_lock)]
    pub async fn get_thread(&self, post_uri: &str) -> Result<Vec<Post>, ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::feed::get_post_thread::ParametersData {
            uri: post_uri
                .parse()
                .map_err(|e| ClientError::InvalidResponse(format!("invalid URI: {e}")))?,
            depth: Some(atrium_api::types::LimitedU16::try_from(6_u16).unwrap()),
            parent_height: Some(atrium_api::types::LimitedU16::try_from(80_u16).unwrap()),
        };

        let output = agent
            .api
            .app
            .bsky
            .feed
            .get_post_thread(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        // Extract posts from thread view
        let mut posts = Vec::new();
        self.extract_thread_posts(&output.data.thread, &mut posts);
        Ok(posts)
        })
    }

    /// Recursively extract posts from a thread view
    fn extract_thread_posts(
        &self,
        thread: &atrium_api::types::Union<
            atrium_api::app::bsky::feed::get_post_thread::OutputThreadRefs,
        >,
        posts: &mut Vec<Post>,
    ) {
        use atrium_api::app::bsky::feed::get_post_thread::OutputThreadRefs;
        use atrium_api::types::Union;

        match thread {
            Union::Refs(OutputThreadRefs::AppBskyFeedDefsThreadViewPost(thread_view)) => {
                // Add parent posts first (recursively)
                if let Some(parent) = &thread_view.data.parent {
                    self.extract_parent_posts(parent, posts);
                }

                // Add the main post
                let post = self.convert_post_view(&thread_view.data.post);
                posts.push(post);

                // Add replies
                if let Some(replies) = &thread_view.data.replies {
                    for reply in replies {
                        self.extract_reply_posts(reply, posts);
                    }
                }
            }
            Union::Refs(OutputThreadRefs::AppBskyFeedDefsNotFoundPost(_)) => {}
            Union::Refs(OutputThreadRefs::AppBskyFeedDefsBlockedPost(_)) => {}
            _ => {}
        }
    }

    /// Extract parent posts from thread (going up the chain)
    fn extract_parent_posts(
        &self,
        parent: &atrium_api::types::Union<
            atrium_api::app::bsky::feed::defs::ThreadViewPostParentRefs,
        >,
        posts: &mut Vec<Post>,
    ) {
        use atrium_api::app::bsky::feed::defs::ThreadViewPostParentRefs;
        use atrium_api::types::Union;

        match parent {
            Union::Refs(ThreadViewPostParentRefs::ThreadViewPost(thread_view)) => {
                // Recurse to get older parents first
                if let Some(grandparent) = &thread_view.data.parent {
                    self.extract_parent_posts(grandparent, posts);
                }
                // Then add this parent
                let post = self.convert_post_view(&thread_view.data.post);
                posts.push(post);
            }
            Union::Refs(ThreadViewPostParentRefs::NotFoundPost(_)) => {}
            Union::Refs(ThreadViewPostParentRefs::BlockedPost(_)) => {}
            _ => {}
        }
    }

    /// Extract reply posts from thread
    fn extract_reply_posts(
        &self,
        reply: &atrium_api::types::Union<
            atrium_api::app::bsky::feed::defs::ThreadViewPostRepliesItem,
        >,
        posts: &mut Vec<Post>,
    ) {
        use atrium_api::app::bsky::feed::defs::ThreadViewPostRepliesItem;
        use atrium_api::types::Union;

        match reply {
            Union::Refs(ThreadViewPostRepliesItem::ThreadViewPost(thread_view)) => {
                let post = self.convert_post_view(&thread_view.data.post);
                posts.push(post);

                // Recursively add nested replies
                if let Some(replies) = &thread_view.data.replies {
                    for nested_reply in replies {
                        self.extract_reply_posts(nested_reply, posts);
                    }
                }
            }
            Union::Refs(ThreadViewPostRepliesItem::NotFoundPost(_)) => {}
            Union::Refs(ThreadViewPostRepliesItem::BlockedPost(_)) => {}
            _ => {}
        }
    }

    /// Convert a PostView to our Post type (used for thread extraction)
    fn convert_post_view(&self, post_view: &atrium_api::app::bsky::feed::defs::PostView) -> Post {
        let author = &post_view.data.author;
        let (text, created_at) = self.extract_post_record(&post_view.data.record);
        let embed = self.extract_embed(&post_view.data.embed);

        let (viewer_like, viewer_repost, viewer_bookmarked) = post_view
            .data
            .viewer
            .as_ref()
            .map(|v| {
                (
                    v.data.like.clone(),
                    v.data.repost.clone(),
                    v.data.bookmarked,
                )
            })
            .unwrap_or((None, None, None));

        Post {
            uri: post_view.data.uri.clone(),
            cid: post_view.data.cid.as_ref().to_string(),
            author: Profile::minimal(
                author.data.did.to_string(),
                author.data.handle.to_string(),
                author.data.display_name.clone(),
                author.data.avatar.clone(),
            ),
            text,
            created_at,
            reply_count: post_view.data.reply_count.map(|c| c as u32),
            repost_count: post_view.data.repost_count.map(|c| c as u32),
            like_count: post_view.data.like_count.map(|c| c as u32),
            indexed_at: post_view.data.indexed_at.as_str().to_string(),
            embed,
            viewer_like,
            viewer_repost,
            viewer_bookmarked,
            repost_reason: None,
            reply_context: None,
        }
    }

    /// Get an author's feed (posts by a specific user)
    #[allow(clippy::await_holding_lock)]
    pub async fn get_author_feed(
        &self,
        actor: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Post>, Option<String>), ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::feed::get_author_feed::ParametersData {
            actor: actor
                .parse()
                .map_err(|e| ClientError::InvalidResponse(format!("invalid actor: {e}")))?,
            cursor: cursor.map(String::from),
            filter: None,
            include_pins: None,
            limit: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .feed
            .get_author_feed(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let posts: Vec<Post> = output
            .data
            .feed
            .into_iter()
            .map(|feed_view| self.convert_feed_view_post(feed_view))
            .collect();

        Ok((posts, output.data.cursor))
        })
    }

    /// Get posts liked by a specific user
    #[allow(clippy::await_holding_lock)]
    pub async fn get_actor_likes(
        &self,
        actor: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Post>, Option<String>), ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::feed::get_actor_likes::ParametersData {
            actor: actor
                .parse()
                .map_err(|e| ClientError::InvalidResponse(format!("invalid actor: {e}")))?,
            cursor: cursor.map(String::from),
            limit: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .feed
            .get_actor_likes(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let posts: Vec<Post> = output
            .data
            .feed
            .into_iter()
            .map(|feed_view| self.convert_feed_view_post(feed_view))
            .collect();

        Ok((posts, output.data.cursor))
        })
    }

    /// Get notifications (mentions, replies, quotes, likes, reposts, follows)
    /// If `mentions_only` is true, filters to just mentions, replies, and quotes
    #[allow(clippy::await_holding_lock)]
    pub async fn get_notifications(
        &self,
        cursor: Option<&str>,
        mentions_only: bool,
    ) -> Result<(Vec<Notification>, Option<String>), ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::notification::list_notifications::ParametersData {
            cursor: cursor.map(String::from),
            limit: None,
            priority: None,
            reasons: None,
            seen_at: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .notification
            .list_notifications(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let notifications: Vec<Notification> = output
            .data
            .notifications
            .into_iter()
            .filter_map(|notif| {
                let reason = notif.data.reason.clone();

                // If mentions_only, filter to just mentions/replies/quotes
                if mentions_only && reason != "mention" && reason != "reply" && reason != "quote" {
                    return None;
                }

                let author = Profile::minimal(
                    notif.data.author.data.did.to_string(),
                    notif.data.author.data.handle.to_string(),
                    notif.data.author.data.display_name.clone(),
                    notif.data.author.data.avatar.clone(),
                );

                // Extract post data if this is a post-based notification
                let post = self.extract_notification_post(&notif);

                Some(Notification {
                    uri: notif.data.uri.clone(),
                    cid: notif.data.cid.as_ref().to_string(),
                    author,
                    reason,
                    indexed_at: notif.data.indexed_at.as_str().to_string(),
                    is_read: notif.data.is_read,
                    post,
                })
            })
            .collect();

        Ok((notifications, output.data.cursor))
        })
    }

    /// Extract post data from a notification record
    fn extract_notification_post(
        &self,
        notif: &atrium_api::app::bsky::notification::list_notifications::Notification,
    ) -> Option<Post> {
        use atrium_api::types::Unknown;

        // The record contains the post data for mentions/replies/quotes
        let reason = &notif.data.reason;
        if reason != "mention" && reason != "reply" && reason != "quote" {
            return None;
        }

        // Extract text and created_at from record
        let (text, created_at) = match &notif.data.record {
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
        };

        let author = Profile::minimal(
            notif.data.author.data.did.to_string(),
            notif.data.author.data.handle.to_string(),
            notif.data.author.data.display_name.clone(),
            notif.data.author.data.avatar.clone(),
        );

        Some(Post {
            uri: notif.data.uri.clone(),
            cid: notif.data.cid.as_ref().to_string(),
            author,
            text,
            created_at,
            indexed_at: notif.data.indexed_at.as_str().to_string(),
            like_count: None,
            repost_count: None,
            reply_count: None,
            embed: None,
            viewer_like: None,
            viewer_repost: None,
            viewer_bookmarked: None,
            repost_reason: None,
            reply_context: None,
        })
    }

    /// Get list of direct message conversations
    #[allow(clippy::await_holding_lock)]
    pub async fn get_conversations(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<Conversation>, Option<String>), ClientError> {
        use atrium_api::agent::bluesky::{AtprotoServiceType, BSKY_CHAT_DID};

        with_agent!(self, agent => {

        // Chat API requires proxying through the chat service
        let chat_did = BSKY_CHAT_DID
            .parse()
            .map_err(|e| ClientError::Network(format!("invalid chat DID: {e}")))?;
        let chat_api = agent.api_with_proxy(chat_did, AtprotoServiceType::BskyChat);

        let params = atrium_api::chat::bsky::convo::list_convos::ParametersData {
            cursor: cursor.map(String::from),
            limit: None,
            read_state: None,
            status: None,
        };

        let output = chat_api
            .chat
            .bsky
            .convo
            .list_convos(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let conversations: Vec<Conversation> = output
            .data
            .convos
            .into_iter()
            .map(|convo| self.convert_convo_view(convo))
            .collect();

        Ok((conversations, output.data.cursor))
        })
    }

    /// Tally unread notifications and chat messages for the sidebar badges.
    ///
    /// One page of each, notifications capped at 100. A very busy account can
    /// undercount, but the badge display caps at 99+ anyway.
    #[allow(clippy::await_holding_lock)]
    pub async fn get_unread_counts(&self) -> Result<UnreadCounts, ClientError> {
        use atrium_api::agent::bluesky::{AtprotoServiceType, BSKY_CHAT_DID};

        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::notification::list_notifications::ParametersData {
            cursor: None,
            limit: 100u8.try_into().ok(),
            priority: None,
            reasons: None,
            seen_at: None,
        };

        let notif_output = agent
            .api
            .app
            .bsky
            .notification
            .list_notifications(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let chat_did = BSKY_CHAT_DID
            .parse()
            .map_err(|e| ClientError::Network(format!("invalid chat DID: {e}")))?;
        let chat_api = agent.api_with_proxy(chat_did, AtprotoServiceType::BskyChat);

        let convo_params = atrium_api::chat::bsky::convo::list_convos::ParametersData {
            cursor: None,
            limit: None,
            read_state: None,
            status: None,
        };

        let convo_output = chat_api
            .chat
            .bsky
            .convo
            .list_convos(convo_params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(UnreadCounts::tally(
            notif_output
                .data
                .notifications
                .iter()
                .map(|n| (n.data.reason.as_str(), n.data.is_read)),
            convo_output
                .data
                .convos
                .iter()
                .map(|c| (c.data.unread_count, c.data.muted)),
        ))
        })
    }

    /// Tell the server every notification has been seen.
    ///
    /// `seenAt` is account-wide. The server cannot mark only mentions seen,
    /// so the Mentions and Activity badges clear together.
    #[allow(clippy::await_holding_lock)]
    pub async fn update_notifications_seen(&self) -> Result<(), ClientError> {
        with_agent!(self, agent => {

        let input = atrium_api::app::bsky::notification::update_seen::InputData {
            seen_at: atrium_api::types::string::Datetime::now(),
        };

        agent
            .api
            .app
            .bsky
            .notification
            .update_seen(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(())
        })
    }

    /// Mark a conversation read up to its latest message.
    ///
    /// The chat badge follows the server's word on the next poll.
    #[allow(clippy::await_holding_lock)]
    pub async fn mark_convo_read(&self, convo_id: &str) -> Result<(), ClientError> {
        use atrium_api::agent::bluesky::{AtprotoServiceType, BSKY_CHAT_DID};

        with_agent!(self, agent => {

        let chat_did = BSKY_CHAT_DID
            .parse()
            .map_err(|e| ClientError::Network(format!("invalid chat DID: {e}")))?;
        let chat_api = agent.api_with_proxy(chat_did, AtprotoServiceType::BskyChat);

        let input = atrium_api::chat::bsky::convo::update_read::InputData {
            convo_id: convo_id.to_string(),
            message_id: None,
        };

        chat_api
            .chat
            .bsky
            .convo
            .update_read(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(())
        })
    }

    /// Get messages for a specific conversation
    #[allow(clippy::await_holding_lock)]
    pub async fn get_messages(
        &self,
        convo_id: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<ChatMessage>, Option<String>), ClientError> {
        use atrium_api::agent::bluesky::{AtprotoServiceType, BSKY_CHAT_DID};

        with_agent!(self, agent => {

        // Chat API requires proxying through the chat service
        let chat_did = BSKY_CHAT_DID
            .parse()
            .map_err(|e| ClientError::Network(format!("invalid chat DID: {e}")))?;
        let chat_api = agent.api_with_proxy(chat_did, AtprotoServiceType::BskyChat);

        let params = atrium_api::chat::bsky::convo::get_messages::ParametersData {
            convo_id: convo_id.to_string(),
            cursor: cursor.map(String::from),
            limit: None,
        };

        let output = chat_api
            .chat
            .bsky
            .convo
            .get_messages(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        use atrium_api::chat::bsky::convo::get_messages::OutputMessagesItem;
        use atrium_api::types::Union;

        let messages: Vec<ChatMessage> = output
            .data
            .messages
            .into_iter()
            .filter_map(|msg| match msg {
                Union::Refs(OutputMessagesItem::ChatBskyConvoDefsMessageView(view)) => {
                    Some(self.chat_message_from_view(&view))
                }
                // Skip deleted messages
                Union::Refs(OutputMessagesItem::ChatBskyConvoDefsDeletedMessageView(_)) => None,
                _ => None,
            })
            .collect();

        Ok((messages, output.data.cursor))
        })
    }

    /// Convert atrium ConvoView to our Conversation type
    fn convert_convo_view(
        &self,
        convo: atrium_api::chat::bsky::convo::defs::ConvoView,
    ) -> Conversation {
        use atrium_api::chat::bsky::convo::defs::ConvoViewLastMessageRefs;
        use atrium_api::types::Union;

        let members: Vec<Profile> = convo
            .data
            .members
            .iter()
            .map(|m| {
                Profile::minimal(
                    m.data.did.to_string(),
                    m.data.handle.to_string(),
                    m.data.display_name.clone(),
                    m.data.avatar.clone(),
                )
            })
            .collect();

        let last_message = convo.data.last_message.as_ref().and_then(|msg| match msg {
            Union::Refs(ConvoViewLastMessageRefs::MessageView(view)) => {
                Some(self.chat_message_from_view(view))
            }
            _ => None,
        });

        Conversation {
            id: convo.data.id,
            members,
            last_message,
            unread_count: convo.data.unread_count,
            muted: convo.data.muted,
        }
    }

    /// Convert an atrium MessageView to our ChatMessage type
    fn chat_message_from_view(
        &self,
        view: &atrium_api::chat::bsky::convo::defs::MessageView,
    ) -> ChatMessage {
        use atrium_api::chat::bsky::convo::defs::MessageViewEmbedRefs;
        use atrium_api::types::Union;

        // The chat lexicon's only embed is a record view, a post shared
        // into the conversation.
        let embed = view.data.embed.as_ref().and_then(|e| match e {
            Union::Refs(MessageViewEmbedRefs::AppBskyEmbedRecordView(record_view)) => {
                self.extract_quote_embed(&record_view.data.record)
            }
            Union::Unknown(_) => None,
        });

        let reactions = view
            .data
            .reactions
            .as_ref()
            .map(|reactions| {
                reactions
                    .iter()
                    .map(|r| ChatReaction {
                        value: r.data.value.clone(),
                        sender_did: r.data.sender.data.did.to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        ChatMessage {
            id: view.data.id.clone(),
            text: view.data.text.clone(),
            sender_did: view.data.sender.data.did.to_string(),
            sent_at: view.data.sent_at.as_str().to_string(),
            embed,
            reactions,
        }
    }

    /// Send a plain text message in a conversation.
    ///
    /// The server answers with the message it stored, so the open view can
    /// append the real thing rather than a local copy.
    #[allow(clippy::await_holding_lock)]
    pub async fn send_chat_message(
        &self,
        convo_id: &str,
        text: &str,
    ) -> Result<ChatMessage, ClientError> {
        use atrium_api::agent::bluesky::{AtprotoServiceType, BSKY_CHAT_DID};

        with_agent!(self, agent => {

        let chat_did = BSKY_CHAT_DID
            .parse()
            .map_err(|e| ClientError::Network(format!("invalid chat DID: {e}")))?;
        let chat_api = agent.api_with_proxy(chat_did, AtprotoServiceType::BskyChat);

        let message = atrium_api::chat::bsky::convo::defs::MessageInputData {
            embed: None,
            facets: None,
            text: text.to_string(),
        };

        let input = atrium_api::chat::bsky::convo::send_message::InputData {
            convo_id: convo_id.to_string(),
            message: message.into(),
        };

        let output = chat_api
            .chat
            .bsky
            .convo
            .send_message(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(self.chat_message_from_view(&output))
        })
    }

    /// The conversation with `did`, if the server allows one.
    ///
    /// Ok(None) means messaging is unavailable: DMs off, a block, or an
    /// account restriction. The server does not say which. When chat is
    /// allowed but no conversation exists yet, get_convo_for_members
    /// mints an empty one.
    #[allow(clippy::await_holding_lock)]
    pub async fn start_conversation(&self, did: &str) -> Result<Option<Conversation>, ClientError> {
        use atrium_api::agent::bluesky::{AtprotoServiceType, BSKY_CHAT_DID};

        with_agent!(self, agent => {

        let chat_did = BSKY_CHAT_DID
            .parse()
            .map_err(|e| ClientError::Network(format!("invalid chat DID: {e}")))?;
        let chat_api = agent.api_with_proxy(chat_did, AtprotoServiceType::BskyChat);

        let member: atrium_api::types::string::Did = did
            .parse()
            .map_err(|e| ClientError::InvalidResponse(format!("invalid did: {e}")))?;

        let params = atrium_api::chat::bsky::convo::get_convo_availability::ParametersData {
            members: vec![member.clone()],
        };
        let availability = chat_api
            .chat
            .bsky
            .convo
            .get_convo_availability(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        if !availability.data.can_chat {
            return Ok(None);
        }
        if let Some(convo) = availability.data.convo.clone() {
            return Ok(Some(self.convert_convo_view(convo)));
        }

        let params = atrium_api::chat::bsky::convo::get_convo_for_members::ParametersData {
            members: vec![member],
        };
        let output = chat_api
            .chat
            .bsky
            .convo
            .get_convo_for_members(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(Some(self.convert_convo_view(output.data.convo.clone())))
        })
    }

    /// Add an emoji reaction to a message. The server answers with the
    /// updated message, so the open view can refresh the row from truth.
    #[allow(clippy::await_holding_lock)]
    pub async fn add_chat_reaction(
        &self,
        convo_id: &str,
        message_id: &str,
        value: &str,
    ) -> Result<ChatMessage, ClientError> {
        use atrium_api::agent::bluesky::{AtprotoServiceType, BSKY_CHAT_DID};

        with_agent!(self, agent => {

        let chat_did = BSKY_CHAT_DID
            .parse()
            .map_err(|e| ClientError::Network(format!("invalid chat DID: {e}")))?;
        let chat_api = agent.api_with_proxy(chat_did, AtprotoServiceType::BskyChat);

        let input = atrium_api::chat::bsky::convo::add_reaction::InputData {
            convo_id: convo_id.to_string(),
            message_id: message_id.to_string(),
            value: value.to_string(),
        };

        let output = chat_api
            .chat
            .bsky
            .convo
            .add_reaction(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(self.chat_message_from_view(&output.data.message))
        })
    }

    /// Take a reaction back off a message. Same answer shape as adding.
    #[allow(clippy::await_holding_lock)]
    pub async fn remove_chat_reaction(
        &self,
        convo_id: &str,
        message_id: &str,
        value: &str,
    ) -> Result<ChatMessage, ClientError> {
        use atrium_api::agent::bluesky::{AtprotoServiceType, BSKY_CHAT_DID};

        with_agent!(self, agent => {

        let chat_did = BSKY_CHAT_DID
            .parse()
            .map_err(|e| ClientError::Network(format!("invalid chat DID: {e}")))?;
        let chat_api = agent.api_with_proxy(chat_did, AtprotoServiceType::BskyChat);

        let input = atrium_api::chat::bsky::convo::remove_reaction::InputData {
            convo_id: convo_id.to_string(),
            message_id: message_id.to_string(),
            value: value.to_string(),
        };

        let output = chat_api
            .chat
            .bsky
            .convo
            .remove_reaction(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(self.chat_message_from_view(&output.data.message))
        })
    }

    /// Delete a message from this account's view of the conversation. The
    /// other side keeps their copy; there is no delete-for-everyone.
    #[allow(clippy::await_holding_lock)]
    pub async fn delete_chat_message(
        &self,
        convo_id: &str,
        message_id: &str,
    ) -> Result<(), ClientError> {
        use atrium_api::agent::bluesky::{AtprotoServiceType, BSKY_CHAT_DID};

        with_agent!(self, agent => {

        let chat_did = BSKY_CHAT_DID
            .parse()
            .map_err(|e| ClientError::Network(format!("invalid chat DID: {e}")))?;
        let chat_api = agent.api_with_proxy(chat_did, AtprotoServiceType::BskyChat);

        let input = atrium_api::chat::bsky::convo::delete_message_for_self::InputData {
            convo_id: convo_id.to_string(),
            message_id: message_id.to_string(),
        };

        chat_api
            .chat
            .bsky
            .convo
            .delete_message_for_self(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(())
        })
    }

    /// Search posts by query string
    #[allow(clippy::await_holding_lock)]
    pub async fn search_posts(
        &self,
        query: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Post>, Option<String>), ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::feed::search_posts::ParametersData {
            q: query.to_string(),
            author: None,
            cursor: cursor.map(String::from),
            domain: None,
            lang: None,
            limit: None,
            mentions: None,
            since: None,
            sort: None,
            tag: None,
            until: None,
            url: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .feed
            .search_posts(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let posts: Vec<Post> = output
            .data
            .posts
            .into_iter()
            .map(|post_view| self.convert_post_view(&post_view))
            .collect();

        Ok((posts, output.data.cursor))
        })
    }

    /// Fast typeahead search for actors (used by mention autocomplete).
    /// Returns a lightweight list of matching profiles.
    #[allow(clippy::await_holding_lock)]
    pub async fn search_actors_typeahead(
        &self,
        query: &str,
        limit: u8,
    ) -> Result<Vec<Profile>, ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::actor::search_actors_typeahead::ParametersData {
            q: Some(query.to_string()),
            limit: atrium_api::types::LimitedNonZeroU8::try_from(limit).ok(),
            term: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .actor
            .search_actors_typeahead(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let actors: Vec<Profile> = output
            .data
            .actors
            .into_iter()
            .map(|actor| Profile {
                did: actor.data.did.to_string(),
                handle: actor.data.handle.to_string(),
                display_name: actor.data.display_name.clone(),
                avatar: actor.data.avatar.clone(),
                banner: None,
                description: None,
                followers_count: None,
                following_count: None,
                posts_count: None,
                viewer_following: actor
                    .data
                    .viewer
                    .as_ref()
                    .and_then(|v| v.data.following.clone()),
                viewer_followed_by: actor
                    .data
                    .viewer
                    .as_ref()
                    .and_then(|v| v.data.followed_by.clone()),
            })
            .collect();

        Ok(actors)
        })
    }

    /// Search actors (users) by query string, with cursor pagination
    #[allow(clippy::await_holding_lock)]
    pub async fn search_actors(
        &self,
        query: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Profile>, Option<String>), ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::actor::search_actors::ParametersData {
            q: Some(query.to_string()),
            cursor: cursor.map(String::from),
            limit: None,
            term: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .actor
            .search_actors(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let actors: Vec<Profile> = output
            .data
            .actors
            .iter()
            .map(Self::profile_from_view)
            .collect();

        Ok((actors, output.data.cursor))
        })
    }

    /// Convert a wire ProfileView into our Profile. The view carries no
    /// counts; pages that need them fetch the full profile.
    fn profile_from_view(view: &atrium_api::app::bsky::actor::defs::ProfileView) -> Profile {
        Profile {
            did: view.data.did.to_string(),
            handle: view.data.handle.to_string(),
            display_name: view.data.display_name.clone(),
            avatar: view.data.avatar.clone(),
            banner: None,
            description: view.data.description.clone(),
            followers_count: None,
            following_count: None,
            posts_count: None,
            viewer_following: view
                .data
                .viewer
                .as_ref()
                .and_then(|v| v.data.following.clone()),
            viewer_followed_by: view
                .data
                .viewer
                .as_ref()
                .and_then(|v| v.data.followed_by.clone()),
        }
    }

    /// Fetch one page of the accounts following `actor`
    #[allow(clippy::await_holding_lock)]
    pub async fn get_followers(
        &self,
        actor: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Profile>, Option<String>), ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::graph::get_followers::ParametersData {
            actor: actor
                .parse()
                .map_err(|e| ClientError::InvalidResponse(format!("invalid actor: {e}")))?,
            cursor: cursor.map(String::from),
            limit: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .graph
            .get_followers(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let profiles = output.data.followers.iter().map(Self::profile_from_view).collect();

        Ok((profiles, output.data.cursor))
        })
    }

    /// Fetch one page of the accounts `actor` follows
    #[allow(clippy::await_holding_lock)]
    pub async fn get_follows(
        &self,
        actor: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Profile>, Option<String>), ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::graph::get_follows::ParametersData {
            actor: actor
                .parse()
                .map_err(|e| ClientError::InvalidResponse(format!("invalid actor: {e}")))?,
            cursor: cursor.map(String::from),
            limit: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .graph
            .get_follows(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let profiles = output.data.follows.iter().map(Self::profile_from_view).collect();

        Ok((profiles, output.data.cursor))
        })
    }

    /// Follow a user. Returns the follow record's URI, which unfollow needs.
    #[allow(clippy::await_holding_lock)]
    pub async fn follow(&self, subject_did: &str) -> Result<String, ClientError> {
        with_agent_and_did!(self, agent, did => {

        let record_json = serde_json::json!({
            "$type": "app.bsky.graph.follow",
            "subject": subject_did,
            "createdAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });
        let record: Unknown = serde_json::from_value(record_json)
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        let collection = atrium_api::types::string::Nsid::new("app.bsky.graph.follow".to_string())
            .map_err(|_| ClientError::InvalidResponse("invalid collection".into()))?;

        let input = create_record::InputData {
            collection,
            record,
            repo: did.clone().into(),
            rkey: None,
            swap_commit: None,
            validate: None,
        };

        let output = agent
            .api
            .com
            .atproto
            .repo
            .create_record(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(output.data.uri.to_string())
        })
    }

    /// Unfollow by deleting the follow record
    /// `follow_uri` is the AT-URI of the follow record (from viewer_following)
    #[allow(clippy::await_holding_lock)]
    pub async fn unfollow(&self, follow_uri: &str) -> Result<(), ClientError> {
        self.delete_record(follow_uri, "app.bsky.graph.follow")
            .await
    }

    /// Save a post to the signed-in user's bookmarks. Bookmarks are private
    /// server-side state, not repo records, so there is nothing to delete by
    /// URI later; the post URI itself is the key.
    #[allow(clippy::await_holding_lock)]
    pub async fn create_bookmark(&self, uri: &str, cid: &str) -> Result<(), ClientError> {
        with_agent!(self, agent => {

        let input = atrium_api::app::bsky::bookmark::create_bookmark::InputData {
            cid: cid
                .parse()
                .map_err(|e| ClientError::InvalidResponse(format!("invalid cid: {e}")))?,
            uri: uri.to_string(),
        };

        agent
            .api
            .app
            .bsky
            .bookmark
            .create_bookmark(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(())
        })
    }

    /// Take a post out of the signed-in user's bookmarks
    #[allow(clippy::await_holding_lock)]
    pub async fn delete_bookmark(&self, uri: &str) -> Result<(), ClientError> {
        with_agent!(self, agent => {

        let input = atrium_api::app::bsky::bookmark::delete_bookmark::InputData {
            uri: uri.to_string(),
        };

        agent
            .api
            .app
            .bsky
            .bookmark
            .delete_bookmark(input.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        Ok(())
        })
    }

    /// Fetch one page of the signed-in user's saved posts
    #[allow(clippy::await_holding_lock)]
    pub async fn get_bookmarks(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<Post>, Option<String>), ClientError> {
        with_agent!(self, agent => {

        let params = atrium_api::app::bsky::bookmark::get_bookmarks::ParametersData {
            cursor: cursor.map(String::from),
            limit: None,
        };

        let output = agent
            .api
            .app
            .bsky
            .bookmark
            .get_bookmarks(params.into())
            .await
            .map_err(|e| self.xrpc_error(e))?;

        let posts: Vec<Post> = output
            .data
            .bookmarks
            .iter()
            .filter_map(|bookmark| self.convert_bookmark_view(bookmark))
            .collect();

        Ok((posts, output.data.cursor))
        })
    }

    /// The post behind a bookmark, if there still is one. A bookmark can
    /// outlive its post or point at an author who has since blocked the
    /// viewer; those items carry no post to show and are skipped.
    fn convert_bookmark_view(
        &self,
        bookmark: &atrium_api::app::bsky::bookmark::defs::BookmarkView,
    ) -> Option<Post> {
        use atrium_api::app::bsky::bookmark::defs::BookmarkViewItemRefs;
        use atrium_api::types::Union;

        match &bookmark.data.item {
            Union::Refs(BookmarkViewItemRefs::AppBskyFeedDefsPostView(view)) => {
                Some(self.convert_post_view(view))
            }
            _ => None,
        }
    }
}

impl Default for HangarClient {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Open Graph metadata fetching (for link card previews) ───

static OG_TITLE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"<meta\s+(?:property|name)="og:title"\s+content="([^"]*)"#).unwrap()
});

static OG_DESC_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"<meta\s+(?:property|name)="og:description"\s+content="([^"]*)"#).unwrap()
});

static OG_IMAGE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"<meta\s+(?:property|name)="og:image"\s+content="([^"]*)"#).unwrap()
});

// Also match reversed attribute order (content before property)
static OG_TITLE_RE2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"<meta\s+content="([^"]*)"\s+(?:property|name)="og:title""#).unwrap()
});

static OG_DESC_RE2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"<meta\s+content="([^"]*)"\s+(?:property|name)="og:description""#).unwrap()
});

static OG_IMAGE_RE2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"<meta\s+content="([^"]*)"\s+(?:property|name)="og:image""#).unwrap()
});

static HTML_TITLE_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"<title[^>]*>([^<]*)</title>").unwrap());

/// Fetch Open Graph metadata from a URL for link card previews.
/// This is a plain HTTP request; does not require authentication.
pub async fn fetch_link_card_meta(url: &str) -> Result<LinkCardData, ClientError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent("Hangar/1.0 (Bluesky Desktop Client)")
        .build()
        .map_err(|e| ClientError::Network(e.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| ClientError::Network(e.to_string()))?;

    let final_url = response.url().to_string();
    let html = response
        .text()
        .await
        .map_err(|e| ClientError::Network(e.to_string()))?;

    // Extract OG metadata with regex (both attribute orderings)
    let title = OG_TITLE_RE
        .captures(&html)
        .or_else(|| OG_TITLE_RE2.captures(&html))
        .and_then(|c| c.get(1))
        .map(|m| html_decode(m.as_str()))
        .or_else(|| {
            HTML_TITLE_RE
                .captures(&html)
                .and_then(|c| c.get(1))
                .map(|m| html_decode(m.as_str()))
        })
        .unwrap_or_default();

    let description = OG_DESC_RE
        .captures(&html)
        .or_else(|| OG_DESC_RE2.captures(&html))
        .and_then(|c| c.get(1))
        .map(|m| html_decode(m.as_str()))
        .unwrap_or_default();

    let og_image_url = OG_IMAGE_RE
        .captures(&html)
        .or_else(|| OG_IMAGE_RE2.captures(&html))
        .and_then(|c| c.get(1))
        .map(|m| html_decode(m.as_str()));

    // Fetch thumbnail if og:image is present
    let thumb = if let Some(ref img_url) = og_image_url {
        match client.get(img_url).send().await {
            Ok(resp) => {
                let content_type = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("image/jpeg")
                    .to_string();
                let mime = if content_type.contains("png") {
                    "image/png".to_string()
                } else if content_type.contains("webp") {
                    "image/webp".to_string()
                } else if content_type.contains("gif") {
                    "image/gif".to_string()
                } else {
                    "image/jpeg".to_string()
                };
                match resp.bytes().await {
                    Ok(bytes) => Some((bytes.to_vec(), mime)),
                    Err(_) => None,
                }
            }
            Err(_) => None,
        }
    } else {
        None
    };

    Ok(LinkCardData {
        url: final_url,
        title,
        description,
        thumb,
    })
}

/// Split an AT-URI like `at://did:plc:xxx/app.bsky.feed.post/rkey` into its
/// repo DID and rkey. Refuses a URI from any other collection, so a like URI
/// can never reach the post delete path.
fn parse_record_uri<'a>(
    record_uri: &'a str,
    collection: &str,
) -> Result<(&'a str, &'a str), ClientError> {
    let parts: Vec<&str> = record_uri.split('/').collect();
    if parts.len() < 5 {
        return Err(ClientError::InvalidResponse("invalid record URI".into()));
    }
    if parts[3] != collection {
        return Err(ClientError::InvalidResponse(format!(
            "expected a {collection} URI, got {}",
            parts[3]
        )));
    }
    Ok((parts[2], parts[4]))
}

/// Basic HTML entity decoding for OG metadata values.
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use atrium_api::app::bsky::embed::record_with_media::ViewMediaRefs;
    use atrium_api::types::{Union, UnknownData};

    fn quote() -> QuoteEmbed {
        QuoteEmbed {
            uri: "at://did:plc:test/app.bsky.feed.post/quoted".into(),
            cid: "cid".into(),
            author: Profile::minimal(
                "did:plc:test".into(),
                "someone.bsky.social".into(),
                None,
                None,
            ),
            text: "the quoted text".into(),
            indexed_at: "2026-01-01T00:00:00Z".into(),
            embed: None,
        }
    }

    fn image() -> Embed {
        Embed::Images(vec![ImageEmbed {
            thumb: "t".into(),
            fullsize: "f".into(),
            alt: String::new(),
            aspect_ratio: None,
        }])
    }

    fn bookmark_view(
        item: serde_json::Value,
    ) -> atrium_api::app::bsky::bookmark::defs::BookmarkView {
        serde_json::from_value(serde_json::json!({
            "subject": {
                "uri": "at://did:plc:test/app.bsky.feed.post/saved",
                "cid": "bafyreidfayvfuwqa7qlnopdjiqrxzs6blmoeu4rujcjtnci5beludirz2a"
            },
            "item": item
        }))
        .expect("a bookmark view deserializes")
    }

    /// A saved post comes through whole; a bookmark whose post is gone or
    /// whose author blocks the viewer has nothing to show and is skipped.
    #[test]
    fn a_bookmark_yields_its_post_and_a_dead_one_yields_nothing() {
        let client = HangarClient::new();

        let live = bookmark_view(serde_json::json!({
            "$type": "app.bsky.feed.defs#postView",
            "uri": "at://did:plc:test/app.bsky.feed.post/saved",
            "cid": "bafyreidfayvfuwqa7qlnopdjiqrxzs6blmoeu4rujcjtnci5beludirz2a",
            "author": { "did": "did:plc:test", "handle": "someone.bsky.social" },
            "record": {
                "$type": "app.bsky.feed.post",
                "text": "worth keeping",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "indexedAt": "2026-01-01T00:00:00Z",
            "viewer": { "bookmarked": true }
        }));
        let post = client
            .convert_bookmark_view(&live)
            .expect("a live post comes through");
        assert_eq!(post.uri, "at://did:plc:test/app.bsky.feed.post/saved");
        assert_eq!(post.text, "worth keeping");
        assert_eq!(
            post.viewer_bookmarked,
            Some(true),
            "the menu decides Save vs Remove from this flag"
        );

        let gone = bookmark_view(serde_json::json!({
            "$type": "app.bsky.feed.defs#notFoundPost",
            "uri": "at://did:plc:test/app.bsky.feed.post/gone",
            "notFound": true
        }));
        assert!(client.convert_bookmark_view(&gone).is_none());

        let blocked = bookmark_view(serde_json::json!({
            "$type": "app.bsky.feed.defs#blockedPost",
            "uri": "at://did:plc:test/app.bsky.feed.post/blocked",
            "blocked": true,
            "author": { "did": "did:plc:test" }
        }));
        assert!(client.convert_bookmark_view(&blocked).is_none());
    }

    /// Cached posts were serialized before `viewer_bookmarked` existed; a row
    /// missing the key must still come back rather than voiding the cache.
    #[test]
    fn a_post_serialized_before_bookmarks_still_deserializes() {
        let old = serde_json::json!({
            "uri": "at://did:plc:test/app.bsky.feed.post/old",
            "cid": "cid",
            "author": {
                "did": "did:plc:test",
                "handle": "someone.bsky.social",
                "display_name": null,
                "avatar": null,
                "banner": null,
                "description": null,
                "followers_count": null,
                "following_count": null,
                "posts_count": null,
                "viewer_following": null,
                "viewer_followed_by": null
            },
            "text": "from the old cache",
            "created_at": "2026-01-01T00:00:00Z",
            "indexed_at": "2026-01-01T00:00:00Z",
            "like_count": null,
            "repost_count": null,
            "reply_count": null,
            "embed": null,
            "viewer_like": null,
            "viewer_repost": null,
            "repost_reason": null,
            "reply_context": null
        });
        let post: Post = serde_json::from_value(old).expect("an old cached post still loads");
        assert_eq!(post.viewer_bookmarked, None);
    }

    /// A GIF attached to a quote is an `external` view, which was the one
    /// variant of this union with no arm: it fell into `_ => None` and the post
    /// rendered with a blank body.
    #[test]
    fn an_external_attached_to_a_quote_still_reaches_the_ui() {
        let media: Union<ViewMediaRefs> = serde_json::from_str(
            r#"{
                "$type": "app.bsky.embed.external#view",
                "external": {
                    "uri": "https://static.klipy.com/ii/hash/76/03/slug.gif?hh=278&ww=498&mp4=abc",
                    "title": "Team",
                    "description": "ALT: Team",
                    "thumb": "https://cdn.bsky.app/img/feed_thumbnail/plain/did/cid"
                }
            }"#,
        )
        .expect("an external view is a media ref");

        match HangarClient::extract_media_embed(&media) {
            Some(Embed::External(ext)) => {
                assert!(ext.uri.contains("static.klipy.com"));
                assert_eq!(ext.title, "Team");
            }
            other => panic!("expected an external embed, got {other:?}"),
        }
    }

    /// Half a record-with-media is still worth showing.
    #[test]
    fn a_record_with_media_survives_losing_either_half() {
        // A quote whose media this build cannot read: show the quote.
        assert!(matches!(
            HangarClient::combine_record_with_media(Some(quote()), None),
            Some(Embed::Quote(_))
        ));
        // Media whose quoted post was deleted, blocked or detached: show the
        // media. `extract_quote_from_record` returns None for all three, and
        // that used to take the media down with it.
        assert!(matches!(
            HangarClient::combine_record_with_media(None, Some(image())),
            Some(Embed::Images(_))
        ));
        assert!(matches!(
            HangarClient::combine_record_with_media(Some(quote()), Some(image())),
            Some(Embed::QuoteWithMedia { .. })
        ));
        assert!(HangarClient::combine_record_with_media(None, None).is_none());
    }

    /// `app.bsky.embed.gallery` is newer than any published atrium-api, so it
    /// arrives as raw IPLD. Dropping it silently made roughly one embedded post
    /// in a hundred render blank.
    #[test]
    fn a_gallery_embed_is_read_off_the_raw_ipld() {
        let unknown: UnknownData = serde_json::from_str(
            r#"{
                "$type": "app.bsky.embed.gallery#view",
                "items": [
                    {
                        "$type": "app.bsky.embed.gallery#viewImage",
                        "thumbnail": "https://cdn.bsky.app/thumb/1",
                        "fullsize": "https://cdn.bsky.app/full/1",
                        "alt": "a description",
                        "aspectRatio": { "height": 3999, "width": 3000 }
                    },
                    {
                        "$type": "app.bsky.embed.gallery#viewImage",
                        "thumbnail": "https://cdn.bsky.app/thumb/2",
                        "fullsize": "https://cdn.bsky.app/full/2"
                    }
                ]
            }"#,
        )
        .expect("gallery view");

        let Some(Embed::Images(images)) = HangarClient::extract_unknown_embed(&unknown, "post")
        else {
            panic!("a gallery is a set of images");
        };
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].fullsize, "https://cdn.bsky.app/full/1");
        assert_eq!(images[0].alt, "a description");
        assert_eq!(images[0].aspect_ratio, Some((3000, 3999)));
        // The optional fields really are optional.
        assert_eq!(images[1].alt, "");
        assert_eq!(images[1].aspect_ratio, None);
    }

    /// Anything else is still `None`, but it is a `None` that says so on
    /// stderr instead of leaving a hole in the post and no way to find out why.
    #[test]
    fn an_unrecognised_embed_type_is_declined_rather_than_guessed_at() {
        let unknown: UnknownData =
            serde_json::from_str(r#"{"$type":"app.bsky.embed.somethingNew#view","x":1}"#)
                .expect("unknown view");
        assert!(HangarClient::extract_unknown_embed(&unknown, "post").is_none());

        // A gallery with no items is nothing to render either.
        let empty: UnknownData =
            serde_json::from_str(r#"{"$type":"app.bsky.embed.gallery#view","items":[]}"#)
                .expect("empty gallery");
        assert!(HangarClient::extract_unknown_embed(&empty, "post").is_none());
    }

    /// The AT-URI parser feeding every delete.
    #[test]
    fn a_record_uri_parses_into_repo_and_rkey_or_is_refused() {
        let (repo, rkey) = parse_record_uri(
            "at://did:plc:abc123/app.bsky.feed.post/3kxyz",
            "app.bsky.feed.post",
        )
        .expect("a well-formed post URI parses");
        assert_eq!(repo, "did:plc:abc123");
        assert_eq!(rkey, "3kxyz");

        // A like URI handed to the post delete path would delete whatever
        // post happens to share the rkey, so the collection has to match.
        assert!(
            parse_record_uri(
                "at://did:plc:abc123/app.bsky.feed.like/3kxyz",
                "app.bsky.feed.post",
            )
            .is_err()
        );

        // Too short to carry an rkey.
        assert!(parse_record_uri("at://did:plc:abc123", "app.bsky.feed.post").is_err());
        assert!(parse_record_uri("", "app.bsky.feed.post").is_err());

        // The other collections the app deletes from still parse.
        for coll in ["app.bsky.feed.like", "app.bsky.feed.repost"] {
            let uri = format!("at://did:plc:a/{coll}/rkey");
            assert!(parse_record_uri(&uri, coll).is_ok());
        }
    }

    /// The mentions/activity split behind the sidebar badges.
    #[test]
    fn unread_notifications_split_between_mentions_and_activity() {
        let counts = UnreadCounts::tally(
            [
                ("mention", false),
                ("reply", false),
                ("quote", false),
                ("like", false),
                ("repost", false),
                ("follow", false),
                ("like-via-repost", false),
                // Read ones count nowhere.
                ("mention", true),
                ("like", true),
            ],
            [],
        );
        assert_eq!(counts.mentions, 3, "mention, reply, and quote");
        assert_eq!(counts.activity, 4, "everything else unread");
        assert_eq!(counts.chat, 0);
    }

    /// Muted conversations must not light the chat badge.
    #[test]
    fn chat_badge_sums_unread_over_unmuted_conversations_only() {
        let counts = UnreadCounts::tally(
            [],
            [
                (2, false),
                (5, false),
                (7, true),
                (0, false),
                // The server should never send a negative count; ignore one
                // rather than wrapping the badge around.
                (-3, false),
            ],
        );
        assert_eq!(counts.chat, 7);
        assert_eq!(counts.mentions, 0);
        assert_eq!(counts.activity, 0);
    }

    /// The wire-to-app mapping for one chat message: id, text, sender,
    /// and timestamp come through untouched.
    #[test]
    fn chat_messages_map_straight_off_the_wire() {
        use atrium_api::chat::bsky::convo::defs::{MessageViewData, MessageViewSenderData};

        let view: atrium_api::chat::bsky::convo::defs::MessageView = MessageViewData {
            embed: None,
            facets: None,
            id: "3kmsgid".into(),
            reactions: None,
            rev: "22".into(),
            sender: MessageViewSenderData {
                did: "did:plc:sender".parse().unwrap(),
            }
            .into(),
            sent_at: "2026-01-01T00:00:00Z".parse().unwrap(),
            text: "hi there & <hello>".into(),
        }
        .into();

        let client = HangarClient::new();
        let message = client.chat_message_from_view(&view);
        assert_eq!(message.id, "3kmsgid");
        assert_eq!(message.text, "hi there & <hello>");
        assert_eq!(message.sender_did, "did:plc:sender");
        assert_eq!(message.sent_at, "2026-01-01T00:00:00Z");
        assert!(message.embed.is_none());
    }

    /// A post shared into a conversation comes through as a quote embed;
    /// its own media rides along for the card to hint at.
    #[test]
    fn a_shared_post_rides_the_message_as_a_quote() {
        let view: atrium_api::chat::bsky::convo::defs::MessageView =
            serde_json::from_value(serde_json::json!({
                "id": "3kmsgid",
                "rev": "22",
                "text": "look at this",
                "sender": { "did": "did:plc:sender" },
                "sentAt": "2026-01-01T00:00:00Z",
                "embed": {
                    "$type": "app.bsky.embed.record#view",
                    "record": {
                        "$type": "app.bsky.embed.record#viewRecord",
                        "uri": "at://did:plc:author/app.bsky.feed.post/shared",
                        "cid": "bafyreidfayvfuwqa7qlnopdjiqrxzs6blmoeu4rujcjtnci5beludirz2a",
                        "author": { "did": "did:plc:author", "handle": "author.bsky.social" },
                        "value": {
                            "$type": "app.bsky.feed.post",
                            "text": "the shared post",
                            "createdAt": "2026-01-01T00:00:00Z"
                        },
                        "indexedAt": "2026-01-01T00:00:00Z"
                    }
                }
            }))
            .expect("a valid message view");

        let client = HangarClient::new();
        let message = client.chat_message_from_view(&view);
        let Some(Embed::Quote(quote)) = message.embed else {
            panic!("the shared post is a quote embed");
        };
        assert_eq!(quote.uri, "at://did:plc:author/app.bsky.feed.post/shared");
        assert_eq!(quote.text, "the shared post");
        assert_eq!(quote.author.handle, "author.bsky.social");
    }
}
