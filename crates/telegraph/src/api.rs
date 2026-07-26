//! Telegraph HTTP API client. Feature-gated behind `client`.
//!
//! TODO(agent): implement. Only two endpoints are needed:
//!   - POST https://api.telegra.ph/createAccount
//!   - POST https://api.telegra.ph/createPage
//!
//! Responses are `{"ok": true, "result": {...}}` or `{"ok": false, "error": "..."}`.
//! Map `FLOOD_WAIT_<n>` errors to `Error::FloodWait(n)` — the bot's token pool
//! depends on distinguishing those.
//!
//! Accept an injected `reqwest::Client` via `with_http` so the caller controls
//! timeouts and the SOCKS5 proxy; do not construct one internally.

use crate::{node::Node, Error};

pub struct Client {
    _access_token: String,
    _http: reqwest::Client,
}

pub struct NewPage<'a> {
    pub title: &'a str,
    pub author_name: Option<&'a str>,
    pub author_url: Option<&'a str>,
    pub content: &'a [Node],
    pub return_content: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct Account {
    pub access_token: String,
    pub short_name: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct Page {
    pub path: String,
    pub url: String,
    pub title: String,
}

impl Client {
    pub fn with_http(access_token: impl Into<String>, http: reqwest::Client) -> Self {
        Self { _access_token: access_token.into(), _http: http }
    }

    pub async fn create_page(&self, _page: &NewPage<'_>) -> Result<Page, Error> {
        todo!("see 01-telegraph-crate.md")
    }
}
