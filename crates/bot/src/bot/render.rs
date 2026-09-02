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

#[cfg(test)]
mod tests {
    use super::*;

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