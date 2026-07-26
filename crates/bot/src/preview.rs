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

/// Port of Go's `preview.TrimDescription`. Not a wire-format constant (not
/// listed in 00-OVERVIEW's compatibility table), so this is a faithful but
/// not byte-exact port: entity decoding and tag stripping are done with
/// `ammonia`/manual passes rather than the exact Go libraries.
///
/// `limit == 0` disables the preview entirely, matching the Go default.
pub fn trim_description(desc: &str, limit: u32) -> String {
    if limit == 0 {
        return String::new();
    }

    let unescaped = decode_entities(desc);
    // Go's regex `<br(| /)>` matches exactly "<br>" or "<br />" (no bare
    // "<br/>"), normalizing both to "<br>" before turning it into a newline.
    let normalized = unescaped.replace("<br />", "<br>");
    let with_newlines = normalized.replace("<br>", "\n");
    let collapsed = collapse_newlines(&with_newlines);

    let stripped = strip_tags(&collapsed);
    let trimmed = stripped.trim_matches('\n');
    trimmed.chars().take(limit as usize).collect()
}

/// Remove `<tag ...>` spans, keeping their surrounding text untouched (no
/// re-escaping), matching the plain-text output of Go's strip-tags library.
/// Quoted attribute values may contain `>`; track quote state so those don't
/// prematurely close the tag.
fn strip_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '<' {
            out.push(ch);
            continue;
        }
        let mut quote: Option<char> = None;
        for next in chars.by_ref() {
            match quote {
                Some(q) if next == q => quote = None,
                Some(_) => {}
                None => match next {
                    '\'' | '"' => quote = Some(next),
                    '>' => break,
                    _ => {}
                },
            }
        }
    }
    out
}

fn collapse_newlines(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_newline = false;
    for ch in input.chars() {
        if ch == '\n' {
            if !prev_newline {
                out.push(ch);
            }
            prev_newline = true;
        } else {
            out.push(ch);
            prev_newline = false;
        }
    }
    out
}

fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        match tail.find(';').filter(|&i| i <= 10) {
            Some(semi) => {
                let entity = &tail[1..semi];
                match decode_entity(entity) {
                    Some(ch) => out.push(ch),
                    None => out.push_str(&tail[..=semi]),
                }
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = entity.strip_prefix('#') {
                dec.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_zero_disables_preview() {
        assert_eq!(trim_description("hello", 0), "");
    }

    #[test]
    fn strips_tags_and_converts_br_to_newline() {
        let desc = "<p>Hello<br>World</p><br />Again";
        assert_eq!(trim_description(desc, 100), "Hello\nWorld\nAgain");
    }

    #[test]
    fn collapses_blank_lines_and_trims_edges() {
        let desc = "<br><br>Body<br><br><br>";
        assert_eq!(trim_description(desc, 100), "Body");
    }

    #[test]
    fn decodes_common_entities() {
        // Matches Go's pipeline order (unescape, then strip tags): a decoded
        // "&lt;tag&gt;" becomes a literal "<tag>" and is then stripped as if
        // it were real markup, same as the Go original.
        assert_eq!(trim_description("A &amp; B &lt;tag&gt;", 100), "A & B ");
    }

    #[test]
    fn truncates_by_char_count() {
        assert_eq!(trim_description("Hello World", 5), "Hello");
    }
}
