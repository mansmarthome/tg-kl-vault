//! Telegraph HTTP API client. Feature-gated behind `client`.
//!
//! Only the endpoints needed by flowerss-bot are implemented:
//! `createAccount` and `createPage`. Responses are decoded from Telegraph's
//! `{"ok": true, "result": ...}` / `{"ok": false, "error": ...}` envelope.

use serde::Deserialize;

use crate::{node::Node, Error};

const API_BASE: &str = "https://api.telegra.ph";

pub struct Client {
    access_token: String,
    http: reqwest::Client,
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

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    ok: bool,
    result: Option<T>,
    error: Option<String>,
}

impl Client {
    pub fn new(access_token: impl Into<String>) -> Self {
        Self { access_token: access_token.into(), http: reqwest::Client::new() }
    }

    pub fn with_http(access_token: impl Into<String>, http: reqwest::Client) -> Self {
        Self { access_token: access_token.into(), http }
    }

    pub async fn create_account(
        http: &reqwest::Client,
        short_name: &str,
        author_name: Option<&str>,
        author_url: Option<&str>,
    ) -> Result<Account, Error> {
        let mut form = vec![("short_name", short_name.to_owned())];
        if let Some(author_name) = author_name {
            form.push(("author_name", author_name.to_owned()));
        }
        if let Some(author_url) = author_url {
            form.push(("author_url", author_url.to_owned()));
        }

        let envelope = http
            .post(format!("{API_BASE}/createAccount"))
            .form(&form)
            .send()
            .await?
            .error_for_status()?
            .json::<Envelope<Account>>()
            .await?;
        unwrap_envelope(envelope)
    }

    pub async fn create_page(&self, page: &NewPage<'_>) -> Result<Page, Error> {
        let content = serde_json::to_string(page.content).map_err(|_| Error::TooLarge)?;
        let mut form = vec![
            ("access_token", self.access_token.clone()),
            ("title", page.title.to_owned()),
            ("content", content),
            ("return_content", page.return_content.to_string()),
        ];
        if let Some(author_name) = page.author_name {
            form.push(("author_name", author_name.to_owned()));
        }
        if let Some(author_url) = page.author_url {
            form.push(("author_url", author_url.to_owned()));
        }

        let envelope = self
            .http
            .post(format!("{API_BASE}/createPage"))
            .form(&form)
            .send()
            .await?
            .error_for_status()?
            .json::<Envelope<Page>>()
            .await?;
        unwrap_envelope(envelope)
    }
}

fn unwrap_envelope<T>(envelope: Envelope<T>) -> Result<T, Error> {
    if envelope.ok {
        return envelope.result.ok_or_else(|| Error::Api("missing result".to_owned()));
    }
    let error = envelope.error.unwrap_or_else(|| "unknown error".to_owned());
    if let Some(seconds) = parse_flood_wait(&error) {
        return Err(Error::FloodWait(seconds));
    }
    Err(Error::Api(error))
}

fn parse_flood_wait(error: &str) -> Option<u64> {
    error.strip_prefix("FLOOD_WAIT_")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flood_wait_error() {
        assert_eq!(parse_flood_wait("FLOOD_WAIT_42"), Some(42));
        assert_eq!(parse_flood_wait("CONTENT_TEXT_REQUIRED"), None);
    }

    #[test]
    fn unwraps_api_error() {
        let err = unwrap_envelope::<Page>(Envelope {
            ok: false,
            result: None,
            error: Some("CONTENT_TEXT_REQUIRED".to_owned()),
        })
        .unwrap_err();
        assert!(matches!(err, Error::Api(e) if e == "CONTENT_TEXT_REQUIRED"));
    }
}
