use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use tokio::sync::Mutex;
use url::Url;

#[derive(Debug, Clone)]
pub struct PublishRequest<'a> {
    pub title: &'a str,
    pub author_name: Option<&'a str>,
    pub author_url: Option<&'a str>,
    pub html: &'a str,
    pub base_url: Option<&'a str>,
}

#[allow(async_fn_in_trait)]
pub trait PreviewPublisher: Send + Sync {
    async fn publish(&self, req: &PublishRequest<'_>) -> anyhow::Result<Option<String>>;
}

#[derive(Debug, Clone, Default)]
pub struct NoopPublisher;

impl PreviewPublisher for NoopPublisher {
    async fn publish(&self, _req: &PublishRequest<'_>) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

pub struct TelegraphPublisher {
    clients: Vec<telegraph::api::Client>,
    next: AtomicUsize,
    cooldowns: Mutex<Vec<Option<Instant>>>,
}

impl TelegraphPublisher {
    pub fn new(tokens: &[String]) -> Self {
        Self {
            clients: tokens.iter().map(telegraph::api::Client::new).collect(),
            next: AtomicUsize::new(0),
            cooldowns: Mutex::new(vec![None; tokens.len()]),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    async fn pick_client(&self) -> Option<usize> {
        if self.clients.is_empty() {
            return None;
        }
        let now = Instant::now();
        let cooldowns = self.cooldowns.lock().await;
        for _ in 0..self.clients.len() {
            let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.clients.len();
            if cooldowns[idx].is_none_or(|until| until <= now) {
                return Some(idx);
            }
        }
        None
    }

    async fn set_cooldown(&self, idx: usize, seconds: u64) {
        let mut cooldowns = self.cooldowns.lock().await;
        cooldowns[idx] = Some(Instant::now() + Duration::from_secs(seconds));
    }
}

impl PreviewPublisher for TelegraphPublisher {
    async fn publish(&self, req: &PublishRequest<'_>) -> anyhow::Result<Option<String>> {
        if self.clients.is_empty() || req.html.trim().is_empty() {
            return Ok(None);
        }

        let base_url = req.base_url.and_then(|raw| Url::parse(raw).ok());
        let converted = telegraph::html_to_nodes(
            req.html,
            &telegraph::ConvertOptions { base_url, ..telegraph::ConvertOptions::default() },
        );
        if converted.nodes.is_empty() {
            return Ok(None);
        }

        for _ in 0..self.clients.len() {
            let Some(idx) = self.pick_client().await else {
                return Ok(None);
            };
            let page = telegraph::api::NewPage {
                title: req.title,
                author_name: req.author_name,
                author_url: req.author_url,
                content: &converted.nodes,
                return_content: false,
            };
            match self.clients[idx].create_page(&page).await {
                Ok(page) => return Ok(Some(page.url)),
                Err(telegraph::Error::FloodWait(seconds)) => {
                    self.set_cooldown(idx, seconds).await;
                    continue;
                }
                Err(err) => return Err(err.into()),
            }
        }

        Ok(None)
    }
}