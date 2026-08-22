use teloxide::utils::html::escape;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageData<'a> {
    pub source_title: &'a str,
    pub content_title: &'a str,
    pub raw_link: &'a str,
    pub preview_text: &'a str,
    pub telegraph_url: &'a str,
    pub tags: &'a str,
    pub enable_telegraph: bool,
}

/// Renders the Go `defaultMessageTpl`. The literal template (separator lines,
/// `原文`, whitespace) stays byte-for-byte for Go parity, but the interpolated
/// feed-derived fields are HTML-escaped — a **deliberate deviation** from the
/// Go original, which did not escape. A single unescaped `&` or `<` in a feed
/// title/URL otherwise makes Telegram reject the message with
/// `can't parse entities`, and `sender.rs` only logs it — i.e. the push is
/// silently lost. URLs in `href` are escaped too (feed URLs routinely carry
/// `&`).
pub fn render_html(data: &MessageData<'_>) -> String {
    let mut out = String::new();
    out.push_str("<b>");
    out.push_str(&escape(data.source_title));
    out.push_str("</b>");
    push_preview(&mut out, &escape(data.preview_text));
    if data.enable_telegraph {
        out.push('\n');
        out.push_str(&escape(data.content_title));
        out.push_str(" <a href=\"");
        out.push_str(&escape(data.telegraph_url));
        out.push_str("\">Telegraph</a> | <a href=\"");
        out.push_str(&escape(data.raw_link));
        out.push_str("\">原文</a>");
    } else {
        out.push('\n');
        out.push_str("<a href=\"");
        out.push_str(&escape(data.raw_link));
        out.push_str("\">");
        out.push_str(&escape(data.content_title));
        out.push_str("</a>");
    }
    out.push('\n');
    out.push_str(&escape(data.tags));
    out.push('\n');
    out
}

/// Render the Go `defaultMessageMarkdownTpl` byte-for-byte.
pub fn render_markdown(data: &MessageData<'_>) -> String {
    let mut out = String::new();
    out.push_str("** ");
    out.push_str(data.source_title);
    out.push_str(" **");
    push_preview(&mut out, data.preview_text);
    if data.enable_telegraph {
        out.push('\n');
        out.push_str(data.content_title);
        out.push_str(" [Telegraph](");
        out.push_str(data.telegraph_url);
        out.push_str(") | [原文](");
        out.push_str(data.raw_link);
        out.push(')');
    } else {
        out.push('\n');
        out.push('[');
        out.push_str(data.content_title);
        out.push_str("](");
        out.push_str(data.raw_link);
        out.push(')');
    }
    out.push('\n');
    out.push_str(data.tags);
    out.push('\n');
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedSettingData<'a> {
    pub source_id: i64,
    pub source_title: &'a str,
    pub source_link: &'a str,
    pub source_error_count: i64,
    pub error_threshold: i64,
    pub interval: i64,
    pub enable_notification: Option<i64>,
    pub enable_telegraph: Option<i64>,
    pub tag: &'a str,
    // Health (migration 0005). `now` is passed in rather than read from the
    // clock so the rendering stays a pure function and testable.
    pub last_success_at: Option<i64>,
    pub last_error: Option<&'a str>,
    pub last_error_at: Option<i64>,
    pub now: i64,
}

/// Renders the Go `feedSettingTmpl` (`internal/bot/handler/set.go`) — same
/// field order and layout, but the labels are zh-TW rather than Go's zh-CN, and
/// the feed-derived title/link/tag are HTML-escaped (deliberate deviation from
/// Go — see `render_html`). Only display text diverges; nothing here is wire
/// format.
///
/// `[最後成功]`/`[最後錯誤]` are appended *after* the Go template's last field,
/// so the field order still matches. `[抓取更新]` only ever said paused-or-not;
/// these say why and since when.
pub fn render_feed_setting(data: &FeedSettingData<'_>) -> String {
    let status = if data.source_error_count >= data.error_threshold { "暫停" } else { "抓取中" };
    let notice = match data.enable_notification {
        Some(0) => "關閉",
        Some(1) => "開啟",
        _ => "",
    };
    let telegraph = match data.enable_telegraph {
        Some(0) => "關閉",
        Some(1) => "開啟",
        _ => "",
    };
    let tag = if data.tag.is_empty() { "無".to_owned() } else { escape(data.tag) };

    let mut out = format!(
        "\n訂閱<b>設定</b>\n[id] {}\n[標題] {}\n[Link] {}\n[抓取更新] {}\n[抓取頻率] {}分鐘\n[通知] {}\n[Telegraph] {}\n[Tag] {}\n",
        data.source_id,
        escape(data.source_title),
        escape(data.source_link),
        status,
        data.interval,
        notice,
        telegraph,
        tag
    );

    out.push_str(&format!("[最後成功] {}\n", humanize_ago(data.last_success_at, data.now)));
    if let Some(error) = data.last_error.filter(|e| !e.is_empty()) {
        out.push_str(&format!(
            "[最後錯誤] {}（{}）\n",
            escape(error),
            humanize_ago(data.last_error_at, data.now)
        ));
    }
    out
}

/// Renders a unix timestamp as a coarse "how long ago", which is all anyone
/// needs when triaging a feed. `None` means it never happened.
pub fn humanize_ago(at: Option<i64>, now: i64) -> String {
    let Some(at) = at else {
        return "從未".to_owned();
    };
    let secs = now.saturating_sub(at);
    if secs < 0 {
        return "剛剛".to_owned();
    }
    match secs {
        s if s < 60 => "剛剛".to_owned(),
        s if s < 3600 => format!("{}分鐘前", s / 60),
        s if s < 86_400 => format!("{}小時前", s / 3600),
        s => format!("{}天前", s / 86_400),
    }
}

fn push_preview(out: &mut String, preview_text: &str) {
    if preview_text.is_empty() {
        return;
    }
    out.push('\n');
    out.push_str("---------- Preview ----------\n");
    out.push_str(preview_text);
    out.push('\n');
    out.push_str("-----------------------------");
}

/// Packs lines into messages under `limit` characters, never splitting a line.
/// A single line longer than the limit gets its own (over-limit) message rather
/// than being silently truncated — better a rejected send than a lie. Shared by
/// `/feedcheck` and the stock close reports, both of which build a `Vec<String>`
/// of individual lines precisely so this can chunk them cleanly.
pub(crate) fn chunk_lines(lines: &[String], limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in lines {
        let extra = line.chars().count() + usize::from(!current.is_empty());
        if !current.is_empty() && current.chars().count() + extra > limit {
            out.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture<'a>(preview_text: &'a str, enable_telegraph: bool) -> MessageData<'a> {
        MessageData {
            source_title: "源標題",
            content_title: "文章標題",
            raw_link: "https://example.com/post",
            preview_text,
            telegraph_url: "https://telegra.ph/post",
            tags: "#tag1 #tag2",
            enable_telegraph,
        }
    }

    #[test]
    fn renders_html_template_without_preview_or_telegraph() {
        assert_eq!(
            render_html(&fixture("", false)),
            "<b>源標題</b>\n<a href=\"https://example.com/post\">文章標題</a>\n#tag1 #tag2\n"
        );
    }

    #[test]
    fn renders_html_template_with_preview_and_telegraph() {
        assert_eq!(
            render_html(&fixture("預覽文字", true)),
            "<b>源標題</b>\n---------- Preview ----------\n預覽文字\n-----------------------------\n文章標題 <a href=\"https://telegra.ph/post\">Telegraph</a> | <a href=\"https://example.com/post\">原文</a>\n#tag1 #tag2\n"
        );
    }

    #[test]
    fn renders_markdown_template_without_preview_or_telegraph() {
        assert_eq!(
            render_markdown(&fixture("", false)),
            "** 源標題 **\n[文章標題](https://example.com/post)\n#tag1 #tag2\n"
        );
    }

    #[test]
    fn renders_markdown_template_with_preview_and_telegraph() {
        assert_eq!(
            render_markdown(&fixture("預覽文字", true)),
            "** 源標題 **\n---------- Preview ----------\n預覽文字\n-----------------------------\n文章標題 [Telegraph](https://telegra.ph/post) | [原文](https://example.com/post)\n#tag1 #tag2\n"
        );
    }

    #[test]
    fn html_escapes_feed_title_link_and_tags() {
        let data = MessageData {
            source_title: "Ben & Jerry's",
            content_title: "Rust <T> & you",
            raw_link: "https://x.test/a?b=1&c=2",
            preview_text: "1 < 2 & 3",
            telegraph_url: "https://telegra.ph/x?u=1&v=2",
            tags: "#a&b",
            enable_telegraph: true,
        };
        let out = render_html(&data);
        // No raw entity-breaking characters survive in the HTML body...
        assert!(!out.contains("<T>"));
        assert!(out.contains("Ben &amp; Jerry's"));
        assert!(out.contains("Rust &lt;T&gt; &amp; you"));
        assert!(out.contains("href=\"https://x.test/a?b=1&amp;c=2\""));
        assert!(out.contains("href=\"https://telegra.ph/x?u=1&amp;v=2\""));
        assert!(out.contains("1 &lt; 2 &amp; 3"));
        assert!(out.contains("#a&amp;b"));
        // ...while our own template tags stay intact.
        assert!(out.contains("<b>Ben &amp; Jerry's</b>"));
        assert!(out.contains(">Telegraph</a>"));
    }

    #[test]
    fn feed_setting_escapes_title_link_tag() {
        let data = FeedSettingData {
            source_id: 7,
            source_title: "A & B <feed>",
            source_link: "https://x.test/f?a=1&b=2",
            source_error_count: 0,
            error_threshold: 100,
            interval: 10,
            enable_notification: Some(1),
            enable_telegraph: Some(0),
            tag: "#x&y",
            last_success_at: None,
            last_error: Some("boom & <crash>"),
            last_error_at: None,
            now: 1_700_000_000,
        };
        let out = render_feed_setting(&data);
        assert!(out.contains("[最後錯誤] boom &amp; &lt;crash&gt;"));
        assert!(out.contains("[標題] A &amp; B &lt;feed&gt;"));
        assert!(out.contains("[Link] https://x.test/f?a=1&amp;b=2"));
        assert!(out.contains("[Tag] #x&amp;y"));
        // Our own <b> label is preserved.
        assert!(out.contains("訂閱<b>設定</b>"));
    }

    #[test]
    fn renders_feed_setting_template_like_go() {
        let data = FeedSettingData {
            source_id: 7,
            source_title: "標題",
            source_link: "https://example.com/feed",
            source_error_count: 0,
            error_threshold: 100,
            interval: 10,
            enable_notification: Some(1),
            enable_telegraph: Some(0),
            tag: "",
            last_success_at: None,
            last_error: None,
            last_error_at: None,
            now: 1_700_000_000,
        };
        assert_eq!(
            render_feed_setting(&data),
            "\n訂閱<b>設定</b>\n[id] 7\n[標題] 標題\n[Link] https://example.com/feed\n[抓取更新] 抓取中\n[抓取頻率] 10分鐘\n[通知] 開啟\n[Telegraph] 關閉\n[Tag] 無\n[最後成功] 從未\n"
        );

        let paused = FeedSettingData { source_error_count: 101, tag: "#tag", ..data };
        assert_eq!(
            render_feed_setting(&paused),
            "\n訂閱<b>設定</b>\n[id] 7\n[標題] 標題\n[Link] https://example.com/feed\n[抓取更新] 暫停\n[抓取頻率] 10分鐘\n[通知] 開啟\n[Telegraph] 關閉\n[Tag] #tag\n[最後成功] 從未\n"
        );

        // Health lines are strictly appended: the Go template's own output is
        // still an exact prefix of ours.
        let healthy = FeedSettingData {
            last_success_at: Some(1_700_000_000 - 7200),
            last_error: Some("HTTP 404"),
            last_error_at: Some(1_700_000_000 - 3 * 86_400),
            ..data
        };
        assert_eq!(
            render_feed_setting(&healthy),
            "\n訂閱<b>設定</b>\n[id] 7\n[標題] 標題\n[Link] https://example.com/feed\n[抓取更新] 抓取中\n[抓取頻率] 10分鐘\n[通知] 開啟\n[Telegraph] 關閉\n[Tag] 無\n[最後成功] 2小時前\n[最後錯誤] HTTP 404（3天前）\n"
        );
    }

    #[test]
    fn chunk_lines_splits_without_cutting_a_line() {
        let lines: Vec<String> = (0..50).map(|i| format!("line-{i:03}-{}", "x".repeat(20))).collect();
        let chunks = chunk_lines(&lines, 100);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 100, "chunk over limit: {}", chunk.chars().count());
        }
        assert_eq!(chunks.join("\n"), lines.join("\n"), "no content lost or reordered");
    }

    #[test]
    fn an_overlong_single_line_gets_its_own_chunk_rather_than_truncation() {
        let long = "y".repeat(200);
        let chunks = chunk_lines(&["short".to_owned(), long.clone()], 100);
        assert_eq!(chunks, vec!["short".to_owned(), long]);
    }

    #[test]
    fn humanize_ago_buckets_by_magnitude() {
        let now = 1_700_000_000;
        assert_eq!(humanize_ago(None, now), "從未");
        assert_eq!(humanize_ago(Some(now), now), "剛剛");
        assert_eq!(humanize_ago(Some(now - 59), now), "剛剛");
        assert_eq!(humanize_ago(Some(now - 60), now), "1分鐘前");
        assert_eq!(humanize_ago(Some(now - 7200), now), "2小時前");
        assert_eq!(humanize_ago(Some(now - 3 * 86_400), now), "3天前");
        // Clock skew must not underflow into a giant number.
        assert_eq!(humanize_ago(Some(now + 500), now), "剛剛");
    }
}
