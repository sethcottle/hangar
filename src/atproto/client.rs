// SPDX-License-Identifier: MPL-2.0

use crate::atproto::types::{
    ChatMessage, Conversation, Embed, ExternalEmbed, ImageEmbed, Notification, Post, Profile,
    QuoteEmbed, ReplyContext, RepostReason, SavedFeed, Session, VideoEmbed,
};
use crate::config::DEFAULT_PDS;
use atrium_api::agent::AtpAgent;
use atrium_api::agent::store::MemorySessionStore;
use atrium_api::com::atproto::repo::{create_record, delete_record};
use atrium_api::types::Unknown;
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

/// Internal struct for reply references
struct ReplyRef {
    root_uri: String,
    root_cid: String,
    parent_uri: String,
    parent_cid: String,
}

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

        // Extract rich embed (images, external links, videos, quotes)
        let embed = self.extract_embed(&post_view.data.embed);

        // Extract repost reason (who reposted this into the feed)
        let repost_reason = self.extract_repost_reason(&feed_view.data.reason);

        // Extract reply context (who this is replying to)
        let reply_context = self.extract_reply_context(&feed_view.data.reply);

        // Extract viewer state (like/repost URIs)
        let (viewer_like, viewer_repost) = post_view
            .data
            .viewer
            .as_ref()
            .map(|v| (v.data.like.clone(), v.data.repost.clone()))
            .unwrap_or((None, None));

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

        let Union::Refs(embed_ref) = embed.as_ref()? else {
            return None;
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
                let quote = self.extract_quote_from_record(&rwm_view.data.record.data.record)?;
                let media = self.extract_media_embed(&rwm_view.data.media)?;
                Some(Embed::QuoteWithMedia {
                    quote,
                    media: Box::new(media),
                })
            }
        }
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
            // ViewNotFound, ViewBlocked, ViewDetached - return None
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
                let quote = self.extract_quote_from_record(&rwm_view.data.record.data.record)?;
                let media = self.extract_media_embed(&rwm_view.data.media)?;
                Some(Embed::QuoteWithMedia {
                    quote,
                    media: Box::new(media),
                })
            }
            _ => None,
        }
    }

    /// Extract media embed from record-with-media view
    fn extract_media_embed(
        &self,
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
            _ => None,
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
    }

    /// Like a post and return the URI of the created like record
    #[allow(clippy::await_holding_lock)]
    pub async fn like(&self, uri: &str, cid: &str) -> Result<String, ClientError> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;
        let session = agent
            .get_session()
            .await
            .ok_or(ClientError::NotAuthenticated)?;

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
            repo: session.data.did.clone().into(),
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
            .map_err(|e| ClientError::Network(e.to_string()))?;

        Ok(output.data.uri.to_string())
    }

    /// Repost a post and return the URI of the created repost record
    #[allow(clippy::await_holding_lock)]
    pub async fn repost(&self, uri: &str, cid: &str) -> Result<String, ClientError> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;
        let session = agent
            .get_session()
            .await
            .ok_or(ClientError::NotAuthenticated)?;

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
            repo: session.data.did.clone().into(),
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
            .map_err(|e| ClientError::Network(e.to_string()))?;

        Ok(output.data.uri.to_string())
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

    /// Generic delete record helper
    #[allow(clippy::await_holding_lock)]
    async fn delete_record(&self, record_uri: &str, collection: &str) -> Result<(), ClientError> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;

        // Parse the AT-URI to extract repo and rkey
        // Format: at://did:plc:xxx/app.bsky.feed.like/rkey
        let parts: Vec<&str> = record_uri.split('/').collect();
        if parts.len() < 5 {
            return Err(ClientError::InvalidResponse("invalid record URI".into()));
        }
        let repo = parts[2]; // did:plc:xxx
        let rkey = parts[4]; // the record key

        let collection = atrium_api::types::string::Nsid::new(collection.to_string())
            .map_err(|_| ClientError::InvalidResponse("invalid collection".into()))?;

        let input = delete_record::InputData {
            collection,
            repo: repo
                .parse()
                .map_err(|_| ClientError::InvalidResponse("invalid repo DID".into()))?,
            rkey: rkey.to_string(),
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
            .map_err(|e| ClientError::Network(e.to_string()))?;
        Ok(())
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
    #[allow(clippy::await_holding_lock)]
    pub async fn create_quote_post(
        &self,
        text: &str,
        quoted_uri: &str,
        quoted_cid: &str,
    ) -> Result<(), ClientError> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;
        let session = agent
            .get_session()
            .await
            .ok_or(ClientError::NotAuthenticated)?;

        let record_json = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": text,
            "createdAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "embed": {
                "$type": "app.bsky.embed.record",
                "record": {
                    "uri": quoted_uri,
                    "cid": quoted_cid
                }
            }
        });

        let record: Unknown = serde_json::from_value(record_json)
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        let collection = atrium_api::types::string::Nsid::new("app.bsky.feed.post".to_string())
            .map_err(|_| ClientError::InvalidResponse("invalid collection".into()))?;

        let input = create_record::InputData {
            collection,
            record,
            repo: session.data.did.clone().into(),
            rkey: None,
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
            .map_err(|e| ClientError::Network(e.to_string()))?;
        Ok(())
    }

    #[allow(clippy::await_holding_lock)]
    async fn create_post_internal(
        &self,
        text: &str,
        reply: Option<ReplyRef>,
    ) -> Result<(), ClientError> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;
        let session = agent
            .get_session()
            .await
            .ok_or(ClientError::NotAuthenticated)?;

        let mut record_json = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": text,
            "createdAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });

        if let Some(r) = reply {
            record_json["reply"] = serde_json::json!({
                "root": { "uri": r.root_uri, "cid": r.root_cid },
                "parent": { "uri": r.parent_uri, "cid": r.parent_cid }
            });
        }

        let record: Unknown = serde_json::from_value(record_json)
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        let collection = atrium_api::types::string::Nsid::new("app.bsky.feed.post".to_string())
            .map_err(|_| ClientError::InvalidResponse("invalid collection".into()))?;

        let input = create_record::InputData {
            collection,
            record,
            repo: session.data.did.clone().into(),
            rkey: None,
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
            .map_err(|e| ClientError::Network(e.to_string()))?;
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
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;

        let output = agent
            .api
            .app
            .bsky
            .actor
            .get_preferences(
                atrium_api::app::bsky::actor::get_preferences::ParametersData {}.into(),
            )
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;

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
                        let display_name = uri.split('/').last().unwrap_or("Feed").to_string();

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
            if let Ok(generators) = self.get_feed_generators_internal(&agent, &feed_uris).await {
                for (uri, name, description) in generators {
                    if let Some(feed) = feeds.iter_mut().find(|f| f.uri == uri) {
                        feed.display_name = name;
                        feed.description = description;
                    }
                }
            }
        }

        Ok(feeds)
    }

    /// Internal helper to get feed generator metadata (uri, display_name, description)
    #[allow(clippy::await_holding_lock)]
    async fn get_feed_generators_internal(
        &self,
        agent: &Agent,
        uris: &[String],
    ) -> Result<Vec<(String, String, Option<String>)>, ClientError> {
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
            .map_err(|e| ClientError::Network(e.to_string()))?;

        Ok(output
            .data
            .feeds
            .into_iter()
            .map(|f| (f.data.uri, f.data.display_name, f.data.description))
            .collect())
    }

    /// Fetch a custom feed by its AT-URI
    #[allow(clippy::await_holding_lock)]
    pub async fn get_feed(
        &self,
        feed_uri: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Post>, Option<String>), ClientError> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;

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
            .map_err(|e| ClientError::Network(e.to_string()))?;

        let posts: Vec<Post> = output
            .data
            .feed
            .into_iter()
            .map(|feed_view| self.convert_feed_view_post(feed_view))
            .collect();

        Ok((posts, output.data.cursor))
    }

    /// Get a post thread (the main post and its replies)
    #[allow(clippy::await_holding_lock)]
    pub async fn get_thread(&self, post_uri: &str) -> Result<Vec<Post>, ClientError> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;

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
            .map_err(|e| ClientError::Network(e.to_string()))?;

        // Extract posts from thread view
        let mut posts = Vec::new();
        self.extract_thread_posts(&output.data.thread, &mut posts);
        Ok(posts)
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

        let (viewer_like, viewer_repost) = post_view
            .data
            .viewer
            .as_ref()
            .map(|v| (v.data.like.clone(), v.data.repost.clone()))
            .unwrap_or((None, None));

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
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;

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
            .map_err(|e| ClientError::Network(e.to_string()))?;

        let posts: Vec<Post> = output
            .data
            .feed
            .into_iter()
            .map(|feed_view| self.convert_feed_view_post(feed_view))
            .collect();

        Ok((posts, output.data.cursor))
    }

    /// Get notifications (mentions, replies, quotes, likes, reposts, follows)
    /// If `mentions_only` is true, filters to just mentions, replies, and quotes
    #[allow(clippy::await_holding_lock)]
    pub async fn get_notifications(
        &self,
        cursor: Option<&str>,
        mentions_only: bool,
    ) -> Result<(Vec<Notification>, Option<String>), ClientError> {
        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;

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
            .map_err(|e| ClientError::Network(e.to_string()))?;

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

        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;

        // Chat API requires proxying through the chat service
        let chat_did = BSKY_CHAT_DID
            .parse()
            .map_err(|e| ClientError::Network(format!("invalid chat DID: {e}")))?;
        let chat_api = agent.api_with_proxy(chat_did, AtprotoServiceType::BskyChat);

        let params = atrium_api::chat::bsky::convo::list_convos::ParametersData {
            cursor: cursor.map(String::from),
            limit: None,
        };

        let output = chat_api
            .chat
            .bsky
            .convo
            .list_convos(params.into())
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;

        let conversations: Vec<Conversation> = output
            .data
            .convos
            .into_iter()
            .map(|convo| self.convert_convo_view(convo))
            .collect();

        Ok((conversations, output.data.cursor))
    }

    /// Get messages for a specific conversation
    #[allow(clippy::await_holding_lock)]
    pub async fn get_messages(
        &self,
        convo_id: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<ChatMessage>, Option<String>), ClientError> {
        use atrium_api::agent::bluesky::{AtprotoServiceType, BSKY_CHAT_DID};

        let agent_guard = self.agent.read().unwrap();
        let agent = agent_guard.as_ref().ok_or(ClientError::NotAuthenticated)?;

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
            .map_err(|e| ClientError::Network(e.to_string()))?;

        use atrium_api::chat::bsky::convo::get_messages::OutputMessagesItem;
        use atrium_api::types::Union;

        let messages: Vec<ChatMessage> = output
            .data
            .messages
            .into_iter()
            .filter_map(|msg| match msg {
                Union::Refs(OutputMessagesItem::ChatBskyConvoDefsMessageView(view)) => {
                    Some(ChatMessage {
                        id: view.data.id.clone(),
                        text: view.data.text.clone(),
                        sender_did: view.data.sender.data.did.to_string(),
                        sent_at: view.data.sent_at.as_str().to_string(),
                    })
                }
                // Skip deleted messages
                Union::Refs(OutputMessagesItem::ChatBskyConvoDefsDeletedMessageView(_)) => None,
                _ => None,
            })
            .collect();

        Ok((messages, output.data.cursor))
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
            Union::Refs(ConvoViewLastMessageRefs::MessageView(view)) => Some(ChatMessage {
                id: view.data.id.clone(),
                text: view.data.text.clone(),
                sender_did: view.data.sender.data.did.to_string(),
                sent_at: view.data.sent_at.as_str().to_string(),
            }),
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
}

impl Default for HangarClient {
    fn default() -> Self {
        Self::new()
    }
}
