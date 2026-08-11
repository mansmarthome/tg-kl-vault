//! Best-effort page metadata (title + description) for bookmarks created from
//! a bare URL pasted into chat, where the client sends no title. Without this
//! a bookmark renders as a naked URL and the AI has only the URL to go on.
//!
//! Streaming fetch hard-capped at 128 KB. We stop early at `</head>` only when
//! a title was already found there; many modern sites (Next.js streaming SSR)
//! emit an empty head and put the real title in body JSON-LD or `<h1>`, so we
//! keep reading up to the byte cap when head is empty.
//!
//! Extraction is a hand-written pure function (`quick-xml` isn't HTML-tolerant,
//! and `scraper` has no business in the bot crate).
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
        // Only bail at </head> when head already yielded a title. Empty-head
        // streaming pages need body content (JSON-LD / h1).
        if contains_close_head(&body) {
            let html = String::from_utf8_lossy(&body);
            if extract_metadata(&html).title.is_some() {
                break;
            }
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

/// Pure extraction from an HTML fragment (head and/or early body).
///
/// Title priority: `og:title` → `twitter:title` → `<title>` → JSON-LD
/// `headline`/`name` → first `<h1>`.
/// Description priority: meta description → og:description → JSON-LD
/// `description`.
pub fn extract_metadata(html: &str) -> PageMetadata {
    let ld = extract_json_ld(html);
    PageMetadata {
        title: extract_meta(html, "property", "og:title")
            .or_else(|| extract_meta(html, "name", "twitter:title"))
            .or_else(|| extract_title_tag(html))
            .or_else(|| ld.as_ref().and_then(|m| m.title.clone()))
            .or_else(|| extract_h1(html)),
        description: extract_meta(html, "name", "description")
            .or_else(|| extract_meta(html, "property", "og:description"))
            .or_else(|| ld.and_then(|m| m.description)),
    }
}

fn extract_title_tag(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let content_start = lower[start..].find('>')? + start + 1;
    let end = lower[content_start..].find("</title>")? + content_start;
    let text = decode_entities(html[content_start..end].trim());
    let text = collapse_ws(&text);
    (!text.is_empty()).then_some(text)
}

fn extract_h1(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<h1")?;
    let content_start = lower[start..].find('>')? + start + 1;
    let end_rel = lower[content_start..].find("</h1>")?;
    let raw = &html[content_start..content_start + end_rel];
    // Strip nested tags inside h1 (e.g. <span>…</span>).
    let text = strip_tags(raw);
    let text = collapse_ws(&decode_entities(&text));
    (!text.is_empty()).then_some(text)
}

#[derive(Default)]
struct LdMeta {
    title: Option<String>,
    description: Option<String>,
}

/// Scans `<script type="application/ld+json">` blocks for Article-like
/// `headline`/`name` and `description`. Intentionally string-based (not a full
/// JSON parser) so we stay dependency-free and tolerate trailing commas / HTML
/// entity noise common in SSR dumps.
fn extract_json_ld(html: &str) -> Option<LdMeta> {
    let lower = html.to_ascii_lowercase();
    let mut search = 0;
    let mut best = LdMeta::default();

    while let Some(rel) = lower[search..].find("<script") {
        let tag_start = search + rel;
        let tag_end = lower[tag_start..]
            .find('>')
            .map(|i| tag_start + i + 1)
            .unwrap_or(html.len());
        let open_tag = &lower[tag_start..tag_end];
        if !open_tag.contains("ld+json") {
            search = tag_end;
            continue;
        }
        let close_rel = lower[tag_end..].find("</script>")?;
        let body = &html[tag_end..tag_end + close_rel];
        let decoded = decode_entities(body);

        if best.title.is_none() {
            best.title = json_string_field(&decoded, "headline")
                .or_else(|| json_string_field(&decoded, "name"));
        }
        if best.description.is_none() {
            best.description = json_string_field(&decoded, "description");
        }
        if best.title.is_some() && best.description.is_some() {
            break;
        }
        search = tag_end + close_rel + "</script>".len();
    }

    if best.title.is_some() || best.description.is_some() {
        Some(best)
    } else {
        None
    }
}

/// Pulls `"key"\s*:\s*"value"` from a JSON-ish blob. Handles `\"` escapes inside
/// the value; stops at the first unescaped `"`.
fn json_string_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut from = 0;
    while let Some(rel) = json[from..].find(&needle) {
        let idx = from + rel + needle.len();
        let rest = json[idx..].trim_start();
        if !rest.starts_with(':') {
            from = idx;
            continue;
        }
        let after_colon = rest[1..].trim_start();
        if !after_colon.starts_with('"') {
            from = idx;
            continue;
        }
        let value = read_json_string(&after_colon[1..])?;
        let text = collapse_ws(&value);
        if !text.is_empty() {
            return Some(text);
        }
        from = idx;
    }
    None
}

fn read_json_string(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    // Minimal \uXXXX support.
                    let hex: String = chars.by_ref().take(4).collect();
                    if hex.len() != 4 {
                        return None;
                    }
                    let code = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(code)?);
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
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
    fn prefers_og_title_over_title_tag() {
        let html = r#"<head>
            <title>Site name</title>
            <meta property="og:title" content="Real Article Title"/>
            </head>"#;
        let md = extract_metadata(html);
        assert_eq!(md.title.as_deref(), Some("Real Article Title"));
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

    #[test]
    fn empty_head_falls_back_to_json_ld_headline() {
        // Shape of Next.js streaming SSR: head closes with no title; body
        // carries schema.org Article JSON-LD + h1.
        let html = r#"<!DOCTYPE html><html><head><meta charSet="utf-8"/></head>
            <body><article>
            <script type="application/ld+json">{"@context":"https://schema.org","@type":"Article","headline":"Rust SIMD on the GPU","description":"GPU code can now use Rust's portable SIMD.","url":"https://vectorware.com/blog/simd-on-gpu"}</script>
            <h1 class="x">Rust SIMD on the GPU</h1>
            </article></body></html>"#;
        let md = extract_metadata(html);
        assert_eq!(md.title.as_deref(), Some("Rust SIMD on the GPU"));
        assert_eq!(
            md.description.as_deref(),
            Some("GPU code can now use Rust's portable SIMD.")
        );
    }

    #[test]
    fn h1_fallback_when_no_json_ld() {
        let html = r#"<html><head></head><body><h1>Only <span>Heading</span></h1></body></html>"#;
        let md = extract_metadata(html);
        assert_eq!(md.title.as_deref(), Some("Only Heading"));
    }

    #[test]
    fn json_string_field_handles_escapes() {
        let json = r#"{"headline":"Foo \"bar\" \n baz"}"#;
        assert_eq!(
            json_string_field(json, "headline").as_deref(),
            Some("Foo \"bar\" baz")
        );
    }
}
