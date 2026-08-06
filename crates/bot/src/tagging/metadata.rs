//! Best-effort page metadata (title + description) for bookmarks created from
//! a bare URL pasted into chat, where the client sends no title. Without this
//! a bookmark renders as a naked URL and the AI has only the URL to go on.
//!
//! Streaming fetch hard-capped at 128 KB, bailing early at `</head>` — the same
//! shape as `feed/fetch.rs`. Non-`text/html` is skipped. Extraction is a
//! hand-written pure function (`quick-xml` isn't HTML-tolerant, and `scraper`
//! has no business in the bot crate).
//!
//! Security: this fetches an arbitrary user-supplied URL (SSRF surface:
//! link-local, loopback). The exposure already exists via `/sub` →
//! `create_source`; callers should still gate bookmark commands on
//! `allowed_users`.

use futures::StreamExt;
use reqwest::{header, Client};

use crate::preview::decode_entities;

const MAX_BYTES: usize = 128 * 1024;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PageMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
}

pub async fn fetch_metadata(
    client: &Client,
    user_agent: &str,
    url: &str,
) -> anyhow::Result<PageMetadata> {
    let resp = client
        .get(url)
        .header(header::USER_AGENT, user_agent)
        .send()
        .await?
        .error_for_status()?;

    let is_html = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.to_ascii_lowercase().contains("text/html"))
        .unwrap_or(false);
    if !is_html {
        return Ok(PageMetadata::default());
    }

    let mut body: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        body.extend_from_slice(&chunk);
        if body.len() >= MAX_BYTES {
            body.truncate(MAX_BYTES);
            break;
        }
        if contains_close_head(&body) {
            break;
        }
    }

    let html = String::from_utf8_lossy(&body);
    Ok(extract_metadata(&html))
}

/// Case-insensitive scan for `</head>`.
fn contains_close_head(body: &[u8]) -> bool {
    const NEEDLE: &[u8; 7] = b"</head>";
    body.windows(NEEDLE.len())
        .any(|w| w.eq_ignore_ascii_case(NEEDLE))
}

/// Pure extraction from an HTML head fragment.
pub fn extract_metadata(html: &str) -> PageMetadata {
    PageMetadata {
        title: extract_title(html),
        description: extract_meta(html, "name", "description")
            .or_else(|| extract_meta(html, "property", "og:description")),
    }
}

fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let content_start = lower[start..].find('>')? + start + 1;
    let end = lower[content_start..].find("</title>")? + content_start;
    let text = decode_entities(html[content_start..end].trim());
    let text = collapse_ws(&text);
    (!text.is_empty()).then_some(text)
}

/// Scans `<meta>` tags for one whose `key_attr` equals `key_val` and returns
/// its `content` attribute.
fn extract_meta(html: &str, key_attr: &str, key_val: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut search = 0;
    while let Some(rel) = lower[search..].find("<meta") {
        let tag_start = search + rel;
        let tag_end = lower[tag_start..]
            .find('>')
            .map(|i| tag_start + i + 1)
            .unwrap_or(html.len());
        let tag = &html[tag_start..tag_end];
        let tag_lower = &lower[tag_start..tag_end];

        let key_matches = attr_value(tag, tag_lower, key_attr)
            .map(|v| v.eq_ignore_ascii_case(key_val))
            .unwrap_or(false);
        if key_matches {
            if let Some(content) = attr_value(tag, tag_lower, "content") {
                let text = collapse_ws(&decode_entities(&content));
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
        search = tag_end;
    }
    None
}

/// Reads `attr="value"` (or single-quoted / unquoted) from a single tag.
/// `tag` and `tag_lower` are byte-length-identical (ASCII lowercasing), so
/// offsets computed on `tag_lower` index into `tag`.
fn attr_value(tag: &str, tag_lower: &str, attr: &str) -> Option<String> {
    let mut from = 0;
    while let Some(rel) = tag_lower[from..].find(attr) {
        let idx = from + rel;
        let before_ok = idx == 0 || !tag_lower.as_bytes()[idx - 1].is_ascii_alphanumeric();
        let after = &tag_lower[idx + attr.len()..];
        let trimmed = after.trim_start();
        if before_ok && trimmed.starts_with('=') {
            let eq_pos = idx + attr.len() + (after.len() - trimmed.len());
            let after_eq = &tag_lower[eq_pos + 1..];
            let val_off = eq_pos + 1 + (after_eq.len() - after_eq.trim_start().len());
            return Some(read_value(&tag[val_off..]));
        }
        from = idx + attr.len();
    }
    None
}

fn read_value(rest: &str) -> String {
    let mut chars = rest.chars();
    match chars.next() {
        Some(q @ ('"' | '\'')) => chars.take_while(|&c| c != q).collect(),
        _ => rest
            .chars()
            .take_while(|&c| !c.is_whitespace() && c != '>' && c != '/')
            .collect(),
    }
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_title_and_description() {
        let html = r#"<html><head><title>Hello &amp; World</title>
            <meta name="description" content="A short summary."></head>"#;
        let md = extract_metadata(html);
        assert_eq!(md.title.as_deref(), Some("Hello & World"));
        assert_eq!(md.description.as_deref(), Some("A short summary."));
    }

    #[test]
    fn falls_back_to_og_description_and_handles_attr_order() {
        let html = r#"<head><meta content="OG summary" property="og:description"/></head>"#;
        let md = extract_metadata(html);
        assert_eq!(md.description.as_deref(), Some("OG summary"));
    }

    #[test]
    fn missing_metadata_is_none() {
        let md = extract_metadata("<html><body>no head</body></html>");
        assert_eq!(md.title, None);
        assert_eq!(md.description, None);
    }

    #[test]
    fn close_head_detection_is_case_insensitive() {
        assert!(contains_close_head(b"<title>x</title></HEAD>"));
        assert!(!contains_close_head(b"<title>x</title>"));
    }
}
