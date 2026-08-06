//! Unlike `bot/render.rs` — whose templates are frozen byte-for-byte for Go
//! parity and deliberately do NOT escape — this is a NEW surface and MUST
//! escape every feed-derived string, including URLs used in href attributes.
//!
//! `teloxide::utils::html::link(url, text)` escapes the *text* but NOT the
//! *url*, and feed URLs routinely contain `&`. A single unescaped `&` triggers
//! "can't parse entities" and the message is silently lost (`sender.rs` only
//! logs it). So we escape the href too.

use teloxide::utils::html::escape;

use crate::bot::i18n::Lang;
use crate::db::models::Bookmark;

const TITLE_MAX_CHARS: usize = 70;
const NOTE_LIST_MAX_CHARS: usize = 60;

/// A bookmark paired with its tag slugs, ready to render.
pub struct RenderedBookmark<'a> {
    pub bookmark: &'a Bookmark,
    pub tags: &'a [String],
}

pub struct ListPageData<'a> {
    pub lang: Lang,
    pub total: i64,
    pub human_page: usize,
    pub total_pages: usize,
    pub items: &'a [RenderedBookmark<'a>],
}

/// Renders a list page (HTML). Empty `items` yields just the header + empty
/// state prompt.
pub fn render_list_page(data: &ListPageData) -> String {
    let lang = data.lang;
    let mut out = lang.bm_list_header(data.total, data.human_page, data.total_pages);
    out.push_str("\n\n");

    if data.items.is_empty() {
        out.push_str(lang.bm_empty());
        return out;
    }

    for item in data.items {
        out.push_str(&render_card(item, lang));
        out.push('\n');
    }
    out.trim_end().to_owned()
}

fn render_card(item: &RenderedBookmark, lang: Lang) -> String {
    let bm = item.bookmark;
    let title = display_title(&bm.title, lang);
    let mut line = format!(
        "[{}] <a href=\"{}\">{}</a>\n",
        bm.id,
        escape(&bm.url),
        escape(&title),
    );

    let mut meta = String::new();
    if let Some(host) = host_of(&bm.url) {
        meta.push_str(&escape(&host));
    }
    let tags = tags_line(item.tags, lang);
    if !meta.is_empty() {
        meta.push_str(" · ");
    }
    meta.push_str(&tags);
    meta.push_str(" · ");
    meta.push_str(&short_date(bm.created_at));
    line.push_str(&meta);
    line.push('\n');

    if !bm.note.is_empty() {
        line.push_str("📝 ");
        line.push_str(&escape(&truncate_chars(&bm.note, NOTE_LIST_MAX_CHARS)));
        line.push('\n');
    }
    line
}

/// Renders the detail page (HTML).
pub fn render_detail(bm: &Bookmark, tags: &[String], lang: Lang) -> String {
    let mut out = format!(
        "<a href=\"{}\">{}</a>\n\n",
        escape(&bm.url),
        escape(&display_title(&bm.title, lang)),
    );
    if let Some(host) = host_of(&bm.url) {
        out.push_str(&format!("🔗 {}\n", escape(&host)));
    }
    let tag_text = if bm.tag_state == 0 {
        lang.bm_tag_pending().to_owned()
    } else {
        tags_line(tags, lang)
    };
    out.push_str(&format!("🏷 {tag_text}\n"));
    if !bm.note.is_empty() {
        out.push_str(&format!("📝 {}\n", escape(&bm.note)));
    }
    out.push_str(&format!("🕘 {}\n", short_date(bm.created_at)));
    if !bm.source_title.is_empty() {
        out.push_str(&format!("📰 {}\n", escape(&bm.source_title)));
    }
    out.trim_end().to_owned()
}

/// Short tag label for the 🔖 button after tagging (`#tech #ai`), capped so the
/// client doesn't truncate mid-tag.
pub fn button_tag_label(tags: &[String], saved_fallback: &str) -> String {
    if tags.is_empty() {
        return saved_fallback.to_owned();
    }
    let joined = tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ");
    let label = format!("🔖 {}", truncate_chars(&joined, 26));
    label
}

/// Export body: Markdown grouped by tag, `- [title](url) — #tags — note`.
pub fn render_export_markdown(items: &[RenderedBookmark], lang: Lang) -> String {
    use std::collections::BTreeMap;
    let mut by_tag: BTreeMap<String, Vec<&RenderedBookmark>> = BTreeMap::new();
    for item in items {
        if item.tags.is_empty() {
            by_tag.entry(lang.bm_untagged_label().to_owned()).or_default().push(item);
        } else {
            for tag in item.tags {
                by_tag.entry(tag.clone()).or_default().push(item);
            }
        }
    }

    let mut out = String::from("# Bookmarks\n");
    for (tag, group) in by_tag {
        out.push_str(&format!("\n## {tag}\n\n"));
        for item in group {
            let bm = item.bookmark;
            let title = display_title(&bm.title, lang);
            let tags = if item.tags.is_empty() {
                String::new()
            } else {
                format!(" — {}", item.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "))
            };
            let note = if bm.note.is_empty() { String::new() } else { format!(" — {}", bm.note) };
            out.push_str(&format!("- [{}]({}){}{}\n", md_escape(&title), bm.url, tags, note));
        }
    }
    out
}

fn display_title(title: &str, lang: Lang) -> String {
    if title.trim().is_empty() {
        lang.bm_untitled().to_owned()
    } else {
        truncate_chars(title, TITLE_MAX_CHARS)
    }
}

fn tags_line(tags: &[String], lang: Lang) -> String {
    if tags.is_empty() {
        lang.bm_no_tags().to_owned()
    } else {
        tags.iter().take(3).map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")
    }
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_owned()))
        .filter(|h| !h.is_empty())
}

fn short_date(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%m-%d").to_string())
        .unwrap_or_default()
}

/// Truncates on a `char` boundary (never mid-codepoint), appending `…` when it
/// actually cut the string.
fn truncate_chars(s: &str, max: usize) -> String {
    let mut boundary = s.len();
    for (count, (idx, _)) in s.char_indices().enumerate() {
        if count == max {
            boundary = idx;
            break;
        }
    }
    if boundary == s.len() {
        s.to_owned()
    } else {
        format!("{}…", &s[..boundary])
    }
}

fn md_escape(s: &str) -> String {
    s.replace('[', "\\[").replace(']', "\\]")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bm(id: i64, title: &str, url: &str, note: &str) -> Bookmark {
        Bookmark {
            id,
            chat_id: 1,
            created_by: 1,
            url: url.to_owned(),
            title: title.to_owned(),
            note: note.to_owned(),
            source_title: String::new(),
            content_hash_id: None,
            telegraph_url: None,
            tag_state: 1,
            tag_attempts: 0,
            tag_next_attempt_at: 0,
            notify_message_id: None,
            notify_kind: 0,
            created_at: 1_690_000_000, // 2023-07-22
            updated_at: 1_690_000_000,
        }
    }

    #[test]
    fn full_card_has_id_link_host_tags() {
        let b = bm(17, "Hello", "https://www.martinkleppmann.com/x", "reread ch3");
        let tags = vec!["tech".to_owned(), "ai".to_owned()];
        let card = render_card(&RenderedBookmark { bookmark: &b, tags: &tags }, Lang::ZhTw);
        assert!(card.contains("[17]"));
        assert!(card.contains("href=\"https://www.martinkleppmann.com/x\""));
        assert!(card.contains("martinkleppmann.com")); // www stripped from host label
        assert!(card.contains("#tech #ai"));
        assert!(card.contains("📝 reread ch3"));
    }

    #[test]
    fn no_note_and_no_tags() {
        let b = bm(1, "T", "https://x.test/a", "");
        let card = render_card(&RenderedBookmark { bookmark: &b, tags: &[] }, Lang::ZhTw);
        assert!(!card.contains("📝"));
        assert!(card.contains(Lang::ZhTw.bm_no_tags()));
    }

    #[test]
    fn unparseable_url_omits_host() {
        let b = bm(1, "T", "not a url", "");
        let card = render_card(&RenderedBookmark { bookmark: &b, tags: &[] }, Lang::ZhTw);
        // No host: the meta line is just `tags · date` (one separator). With a
        // host it would be `host · tags · date` (two).
        assert_eq!(card.matches(" · ").count(), 1);
    }

    #[test]
    fn cjk_truncates_on_char_boundary() {
        let long = "字".repeat(100);
        let out = truncate_chars(&long, TITLE_MAX_CHARS);
        assert!(out.ends_with('…'));
        // Must be valid UTF-8 (would panic on a bad slice) and 70 chars + …
        assert_eq!(out.chars().count(), TITLE_MAX_CHARS + 1);
    }

    #[test]
    fn escapes_html_metacharacters_in_title() {
        let b = bm(1, "<script>&\"", "https://x.test/a?a=1&b=2", "");
        let card = render_card(&RenderedBookmark { bookmark: &b, tags: &[] }, Lang::ZhTw);
        assert!(!card.contains("<script>"));
        assert!(card.contains("&lt;script&gt;"));
        // The href's `&` is escaped too.
        assert!(card.contains("a=1&amp;b=2"));
    }

    #[test]
    fn empty_list_renders_empty_state() {
        let data = ListPageData { lang: Lang::ZhTw, total: 0, human_page: 1, total_pages: 1, items: &[] };
        let out = render_list_page(&data);
        assert!(out.contains(Lang::ZhTw.bm_empty()));
    }

    #[test]
    fn pending_detail_shows_processing_label() {
        let mut b = bm(1, "T", "https://x.test/a", "");
        b.tag_state = 0;
        let out = render_detail(&b, &[], Lang::ZhTw);
        assert!(out.contains(Lang::ZhTw.bm_tag_pending()));
    }
}
