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

/// Render the Go `defaultMessageTpl` byte-for-byte.
///
/// Do not translate or tidy whitespace here. The separator lines, `原文`, and
/// Go template trim-marker effects are user-visible compatibility surface.
pub fn render_html(data: &MessageData<'_>) -> String {
    let mut out = String::new();
    out.push_str("<b>");
    out.push_str(data.source_title);
    out.push_str("</b>");
    push_preview(&mut out, data.preview_text);
    if data.enable_telegraph {
        out.push('\n');
        out.push_str(data.content_title);
        out.push_str(" <a href=\"");
        out.push_str(data.telegraph_url);
        out.push_str("\">Telegraph</a> | <a href=\"");
        out.push_str(data.raw_link);
        out.push_str("\">原文</a>");
    } else {
        out.push('\n');
        out.push_str("<a href=\"");
        out.push_str(data.raw_link);
        out.push_str("\">");
        out.push_str(data.content_title);
        out.push_str("</a>");
    }
    out.push('\n');
    out.push_str(data.tags);
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
}

/// Render the Go `feedSettingTmpl` (`internal/bot/handler/set.go`) byte-for-byte.
pub fn render_feed_setting(data: &FeedSettingData<'_>) -> String {
    let status = if data.source_error_count >= data.error_threshold { "暂停" } else { "抓取中" };
    let notice = match data.enable_notification {
        Some(0) => "关闭",
        Some(1) => "开启",
        _ => "",
    };
    let telegraph = match data.enable_telegraph {
        Some(0) => "关闭",
        Some(1) => "开启",
        _ => "",
    };
    let tag = if data.tag.is_empty() { "无" } else { data.tag };

    format!(
        "\n订阅<b>设置</b>\n[id] {}\n[标题] {}\n[Link] {}\n[抓取更新] {}\n[抓取频率] {}分钟\n[通知] {}\n[Telegraph] {}\n[Tag] {}\n",
        data.source_id, data.source_title, data.source_link, status, data.interval, notice, telegraph, tag
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture<'a>(preview_text: &'a str, enable_telegraph: bool) -> MessageData<'a> {
        MessageData {
            source_title: "源标题",
            content_title: "文章标题",
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
            "<b>源标题</b>\n<a href=\"https://example.com/post\">文章标题</a>\n#tag1 #tag2\n"
        );
    }

    #[test]
    fn renders_html_template_with_preview_and_telegraph() {
        assert_eq!(
            render_html(&fixture("预览文字", true)),
            "<b>源标题</b>\n---------- Preview ----------\n预览文字\n-----------------------------\n文章标题 <a href=\"https://telegra.ph/post\">Telegraph</a> | <a href=\"https://example.com/post\">原文</a>\n#tag1 #tag2\n"
        );
    }

    #[test]
    fn renders_markdown_template_without_preview_or_telegraph() {
        assert_eq!(
            render_markdown(&fixture("", false)),
            "** 源标题 **\n[文章标题](https://example.com/post)\n#tag1 #tag2\n"
        );
    }

    #[test]
    fn renders_markdown_template_with_preview_and_telegraph() {
        assert_eq!(
            render_markdown(&fixture("预览文字", true)),
            "** 源标题 **\n---------- Preview ----------\n预览文字\n-----------------------------\n文章标题 [Telegraph](https://telegra.ph/post) | [原文](https://example.com/post)\n#tag1 #tag2\n"
        );
    }

    #[test]
    fn renders_feed_setting_template_like_go() {
        let data = FeedSettingData {
            source_id: 7,
            source_title: "标题",
            source_link: "https://example.com/feed",
            source_error_count: 0,
            error_threshold: 100,
            interval: 10,
            enable_notification: Some(1),
            enable_telegraph: Some(0),
            tag: "",
        };
        assert_eq!(
            render_feed_setting(&data),
            "\n订阅<b>设置</b>\n[id] 7\n[标题] 标题\n[Link] https://example.com/feed\n[抓取更新] 抓取中\n[抓取频率] 10分钟\n[通知] 开启\n[Telegraph] 关闭\n[Tag] 无\n"
        );

        let paused = FeedSettingData { source_error_count: 101, tag: "#tag", ..data };
        assert_eq!(
            render_feed_setting(&paused),
            "\n订阅<b>设置</b>\n[id] 7\n[标题] 标题\n[Link] https://example.com/feed\n[抓取更新] 暂停\n[抓取频率] 10分钟\n[通知] 开启\n[Telegraph] 关闭\n[Tag] #tag\n"
        );
    }
}
