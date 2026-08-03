// SPDX-License-Identifier: MPL-2.0

use crate::state::session_store::FileSessionStore;
use atrium_identity::did::{CommonDidResolver, CommonDidResolverConfig, DEFAULT_PLC_DIRECTORY_URL};
use atrium_identity::handle::{AppViewHandleResolver, AppViewHandleResolverConfig};
use atrium_oauth::store::state::MemoryStateStore;
use atrium_oauth::{
    AtprotoLocalhostClientMetadata, AuthorizeOptions, CallbackParams, DefaultHttpClient,
    KnownScope, OAuthClient, OAuthClientConfig, OAuthResolverConfig, OAuthSession, Scope,
};
use std::sync::Arc;
use thiserror::Error;

/// Concrete types for our OAuth client.
type DidResolver = CommonDidResolver<DefaultHttpClient>;
type HandleResolver = AppViewHandleResolver<DefaultHttpClient>;
type HangarOAuthClient =
    OAuthClient<MemoryStateStore, FileSessionStore, DidResolver, HandleResolver>;

/// The OAuthSession type produced by our client.
pub type HangarOAuthSession =
    OAuthSession<DefaultHttpClient, DidResolver, HandleResolver, FileSessionStore>;

const BSKY_PUBLIC_API: &str = "https://public.api.bsky.app";

#[derive(Error, Debug)]
pub enum OAuthError {
    #[error("OAuth error: {0}")]
    OAuth(String),
    #[error("callback server error: {0}")]
    CallbackServer(String),
    /// The stored session predates Hangar recording what its `client_id` was
    /// derived from, so no client we can build is able to refresh it. Only a
    /// fresh sign-in recovers.
    #[error("stored session was not recorded with a redirect URI")]
    MissingClientBinding,
}

/// Manages the AT Protocol OAuth flow for desktop apps.
///
/// The OAuthClient is created per-auth-attempt because the redirect_uri
/// includes a dynamically assigned port from the callback server.
pub struct OAuthManager;

impl OAuthManager {
    /// The scopes we ask for at authorization.
    ///
    /// Single source of truth: a loopback `client_id` is derived from the scope
    /// list as much as from the redirect URI, so two copies would hand the
    /// authorization server two different clients for one sign-in.
    fn requested_scopes() -> Vec<Scope> {
        vec![
            Scope::Known(KnownScope::Atproto),
            Scope::Known(KnownScope::TransitionGeneric),
            Scope::Known(KnownScope::TransitionChatBsky),
        ]
    }

    /// Build an OAuthClient configured for a specific callback port.
    fn build_client(
        redirect_uri: &str,
        scopes: Vec<Scope>,
        session_store: FileSessionStore,
    ) -> Result<HangarOAuthClient, OAuthError> {
        let http_client = Arc::new(DefaultHttpClient::default());

        let config = OAuthClientConfig {
            client_metadata: AtprotoLocalhostClientMetadata {
                redirect_uris: Some(vec![redirect_uri.to_string()]),
                scopes: Some(scopes),
            },
            keys: None,
            resolver: OAuthResolverConfig {
                did_resolver: CommonDidResolver::new(CommonDidResolverConfig {
                    plc_directory_url: DEFAULT_PLC_DIRECTORY_URL.to_string(),
                    http_client: Arc::clone(&http_client),
                }),
                handle_resolver: AppViewHandleResolver::new(AppViewHandleResolverConfig {
                    service_url: BSKY_PUBLIC_API.to_string(),
                    http_client: Arc::clone(&http_client),
                }),
                authorization_server_metadata: Default::default(),
                protected_resource_metadata: Default::default(),
            },
            state_store: MemoryStateStore::default(),
            session_store,
        };

        HangarOAuthClient::new(config).map_err(|e| OAuthError::OAuth(e.to_string()))
    }

    /// Build an OAuthClient that reproduces the `client_id` this DID's session
    /// was authorized under.
    ///
    /// atrium derives a loopback `client_id` from the redirect URI and scopes
    /// and sends it with every token request, refreshes included, so restoring
    /// with a placeholder URI produces a client the authorization server will
    /// refuse to refresh for. The binding comes from the store, and its absence
    /// is a hard failure.
    pub fn build_restore_client(
        session_store: FileSessionStore,
        did: &atrium_api::types::string::Did,
    ) -> Result<HangarOAuthClient, OAuthError> {
        let (redirect_uri, scopes) = session_store
            .client_binding(did)
            .map_err(|e| OAuthError::OAuth(e.to_string()))?
            .ok_or(OAuthError::MissingClientBinding)?;

        // A binding that fails validation means the original client_id cannot
        // be rebuilt. Send it down the re-auth path rather than reporting a
        // client-construction failure.
        Self::build_client(&redirect_uri, scopes, session_store).map_err(|e| {
            eprintln!("hangar: stored OAuth client binding is unusable: {e}");
            OAuthError::MissingClientBinding
        })
    }

    /// Run the full OAuth authorization flow.
    ///
    /// 1. Binds a localhost callback server on a random port
    /// 2. Creates an OAuthClient with that port's redirect URI
    /// 3. Gets the authorization URL
    /// 4. Returns (auth_url, client, callback_rx); the caller opens the browser and waits
    pub async fn start_auth(
        handle: &str,
        session_store: FileSessionStore,
    ) -> Result<
        (
            String,
            HangarOAuthClient,
            std::sync::mpsc::Receiver<CallbackParams>,
        ),
        OAuthError,
    > {
        // Bind callback server to a random port
        let server = tiny_http::Server::http("127.0.0.1:0")
            .map_err(|e| OAuthError::CallbackServer(e.to_string()))?;
        let port = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| OAuthError::CallbackServer("failed to get server address".into()))?
            .port();

        let redirect_uri = format!("http://127.0.0.1:{port}/callback");
        let scopes = Self::requested_scopes();

        // atrium writes the session row itself when it exchanges the code.
        // The store has to carry the binding before the client is built,
        // because we never see that row again to attach it later.
        let session_store = session_store.with_client_binding(&redirect_uri, &scopes);

        // Build OAuth client with this specific redirect URI
        let client = Self::build_client(&redirect_uri, scopes.clone(), session_store)?;

        // Get authorization URL
        let options = AuthorizeOptions {
            redirect_uri: Some(redirect_uri),
            scopes,
            prompt: None,
            state: None,
        };

        let auth_url = client
            .authorize(handle, options)
            .await
            .map_err(|e| OAuthError::OAuth(e.to_string()))?;

        // Spawn callback listener on a separate OS thread
        let (tx, rx) = std::sync::mpsc::channel::<CallbackParams>();

        std::thread::spawn(move || {
            // Wait up to 5 minutes for the callback
            let timeout = std::time::Duration::from_secs(300);
            match server.recv_timeout(timeout) {
                Ok(Some(request)) => {
                    let url_str = format!("http://127.0.0.1{}", request.url());
                    if let Ok(url) = url::Url::parse(&url_str) {
                        let params: std::collections::HashMap<String, String> =
                            url.query_pairs().into_owned().collect();

                        let callback = CallbackParams {
                            code: params.get("code").cloned().unwrap_or_default(),
                            state: params.get("state").cloned(),
                            iss: params.get("iss").cloned(),
                        };

                        // Respond with a theme-aware success page
                        let response = tiny_http::Response::from_string(CALLBACK_SUCCESS_HTML)
                            .with_header(
                                tiny_http::Header::from_bytes(
                                    &b"Content-Type"[..],
                                    &b"text/html; charset=utf-8"[..],
                                )
                                .unwrap(),
                            );

                        let _ = request.respond(response);
                        let _ = tx.send(callback);
                    }
                }
                Ok(None) | Err(_) => {
                    // Timeout or error. The server drops and the caller sees the channel disconnect.
                }
            }
        });

        Ok((auth_url, client, rx))
    }

    /// Complete the OAuth flow by exchanging the authorization code for a session.
    pub async fn complete_auth(
        client: &HangarOAuthClient,
        params: CallbackParams,
    ) -> Result<HangarOAuthSession, OAuthError> {
        let (session, _app_state) = client
            .callback(params)
            .await
            .map_err(|e| OAuthError::OAuth(e.to_string()))?;

        Ok(session)
    }

    /// Restore an OAuth session from the persistent store by DID.
    pub async fn restore_session(
        client: &HangarOAuthClient,
        did: &atrium_api::types::string::Did,
    ) -> Result<HangarOAuthSession, OAuthError> {
        client
            .restore(did)
            .await
            .map_err(|e| OAuthError::OAuth(e.to_string()))
    }
}

/// Success page served by the callback server.
/// Uses prefers-color-scheme to match the user's system theme.
const CALLBACK_SUCCESS_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>Hangar — Signed In</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    font-family: system-ui, -apple-system, sans-serif;
    display: flex; justify-content: center; align-items: center;
    height: 100vh;
    background: #fafafa; color: #1a1a1a;
  }
  .card {
    text-align: center; padding: 2.5rem;
    border-radius: 12px; background: #fff;
    box-shadow: 0 2px 8px rgba(0,0,0,0.08);
    max-width: 400px;
  }
  h1 { color: #3584e4; margin-bottom: 0.5rem; font-size: 1.5rem; }
  p { color: #666; line-height: 1.5; }
  @media (prefers-color-scheme: dark) {
    body { background: #1a1a2e; color: #e0e0e0; }
    .card { background: #16213e; box-shadow: 0 4px 12px rgba(0,0,0,0.3); }
    h1 { color: #78aeed; }
    p { color: #a0a0a0; }
  }
</style>
</head>
<body>
<div class="card">
  <h1>Signed in!</h1>
  <p>You can close this tab and return to Hangar.</p>
</div>
</body>
</html>"#;
