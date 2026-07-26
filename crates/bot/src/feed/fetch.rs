use std::time::Duration;

use bytes::BytesMut;
use futures::StreamExt;
use reqwest::{header, redirect::Policy, Client, StatusCode};

use crate::config::Config;

#[derive(Debug, Clone)]
pub struct Fetcher {
    client: Client,
    user_agent: String,
    max_body_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Unchanged,
    Modified(FetchedFeed),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedFeed {
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Fetcher {
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(Policy::limited(5))
            .user_agent(config.user_agent.clone());

        if !config.socks5.trim().is_empty() {
            // Go config stores SOCKS5 as host:port. Reqwest expects a URL.
            builder = builder.proxy(reqwest::Proxy::all(format!("socks5h://{}", config.socks5))?);
        }

        Ok(Self { client: builder.build()?, user_agent: config.user_agent.clone(), max_body_bytes: 10 * 1024 * 1024 })
    }

    pub fn with_client(client: Client, user_agent: impl Into<String>) -> Self {
        Self { client, user_agent: user_agent.into(), max_body_bytes: 10 * 1024 * 1024 }
    }

    pub async fn fetch(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> anyhow::Result<FetchOutcome> {
        let mut req = self.client.get(url).header(header::USER_AGENT, &self.user_agent);
        if let Some(etag) = etag.filter(|s| !s.is_empty()) {
            req = req.header(header::IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = last_modified.filter(|s| !s.is_empty()) {
            req = req.header(header::IF_MODIFIED_SINCE, last_modified);
        }

        let resp = req.send().await?;
        if resp.status() == StatusCode::NOT_MODIFIED {
            return Ok(FetchOutcome::Unchanged);
        }
        let resp = resp.error_for_status()?;
        let etag = header_to_string(resp.headers().get(header::ETAG));
        let last_modified = header_to_string(resp.headers().get(header::LAST_MODIFIED));

        let mut body = BytesMut::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if body.len() + chunk.len() > self.max_body_bytes {
                anyhow::bail!("feed body exceeds {} bytes", self.max_body_bytes);
            }
            body.extend_from_slice(&chunk);
        }

        Ok(FetchOutcome::Modified(FetchedFeed { body: body.to_vec(), etag, last_modified }))
    }
}

fn header_to_string(value: Option<&header::HeaderValue>) -> Option<String> {
    value.and_then(|v| v.to_str().ok()).map(ToOwned::to_owned)
}
