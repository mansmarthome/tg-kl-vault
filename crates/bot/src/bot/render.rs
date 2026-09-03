use crate::bot::i18n::Lang;

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
    pub enable_source_title: Option<i64>,
    pub tag: &'a str,
}

/// Render the per-feed setting panel in the chat's language.
pub fn render_feed_setting(lang: Lang, data: &FeedSettingData<'_>) -> String {
    lang.feed_setting(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &'static str, source_error_count: i64) -> FeedSettingData<'static> {
        FeedSettingData {
            source_id: 7,
            source_title: "Example",
            source_link: "https://example.com/feed",
            source_error_count,
            error_threshold: 100,
            interval: 10,
            enable_notification: Some(1),
            enable_telegraph: Some(0),
            enable_source_title: Some(1),
            tag,
        }
    }

    #[test]
    fn renders_in_all_languages() {
        for lang in [Lang::En, Lang::ZhTw, Lang::Ru] {
            let active = render_feed_setting(lang, &fixture("", 0));
            let paused = render_feed_setting(lang, &fixture("#tag", 101));
            assert!(!active.is_empty());
            assert!(!paused.is_empty());
            assert!(active.contains("7"));
            assert!(paused.contains("#tag"));
        }
    }
}
