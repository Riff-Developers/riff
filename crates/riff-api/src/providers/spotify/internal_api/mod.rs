pub(super) mod connect_state;
mod playlist_permission;

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use reqwest::header::CONTENT_TYPE;
use reqwest::Method;

use crate::error::DomainError;
use crate::token::TokenProvider;

const RESOLVE_BASE_URL: &str = "https://apresolve.spotify.com/";

const FALLBACK_SPCLIENT: &str = "spclient.wg.spotify.com";
const FALLBACK_DEALER: &str = "dealer.spotify.com";

/// How long a looked-up host is used before it's looked up again.
const RESOLVED_FOR: Duration = Duration::from_secs(60 * 60);
/// How long the fallback is used after a host couldn't be looked up.
const FALLBACK_FOR: Duration = Duration::from_secs(300);

pub struct SpotifyInternalApi {
    token_provider: Arc<dyn TokenProvider>,
    spclient: CachedHost,
    dealer: CachedHost,
}

impl SpotifyInternalApi {
    pub fn new(token_provider: Arc<dyn TokenProvider>) -> Self {
        Self {
            token_provider,
            spclient: CachedHost::new("spclient", FALLBACK_SPCLIENT),
            dealer: CachedHost::new("dealer", FALLBACK_DEALER),
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, DomainError> {
        #[cfg(debug_assertions)]
        if let Some(err) = crate::dev::simulated_failure() {
            return Err(err);
        }

        let token = self.access_token()?;
        let url = format!("https://{}{path}", self.spclient.get().await);
        let mut request = crate::http::api_client()
            .request(method, url)
            .bearer_auth(token)
            .header(CONTENT_TYPE, content_type)
            .body(body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        super::request::send(request).await
    }

    fn access_token(&self) -> Result<String, DomainError> {
        self.token_provider
            .access_token()
            .ok_or(DomainError::NoToken)
    }

    async fn dealer_host(&self) -> String {
        self.dealer.get().await
    }
}

struct CachedHost {
    kind: &'static str,
    fallback: &'static str,
    host: RwLock<Option<(String, Instant)>>,
}

impl CachedHost {
    fn new(kind: &'static str, fallback: &'static str) -> Self {
        Self {
            kind,
            fallback,
            host: RwLock::new(None),
        }
    }

    async fn get(&self) -> String {
        if let Some((host, until)) = self.host.read().unwrap().clone() {
            if Instant::now() < until {
                return host;
            }
        }
        let (host, keep_for) = match resolve(self.kind).await {
            Some(host) => (host, RESOLVED_FOR),
            None => (self.fallback.to_string(), FALLBACK_FOR),
        };
        *self.host.write().unwrap() = Some((host.clone(), Instant::now() + keep_for));
        host
    }
}

async fn resolve(kind: &str) -> Option<String> {
    let response = crate::http::api_client()
        .get(format!("{RESOLVE_BASE_URL}?type={kind}"))
        .send()
        .await
        .inspect_err(|e| warn!("spotify: could not resolve a {kind} host: {e}"))
        .ok()?;
    let listed: serde_json::Value = serde_json::from_slice(&response.bytes().await.ok()?).ok()?;
    let host = listed.get(kind)?.get(0)?.as_str()?;
    Some(host.strip_suffix(":443").unwrap_or(host).to_string())
}
