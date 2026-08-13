use feed_rs::{model::Entry, parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFeed {
    pub title: Option<String>,
    pub items: Vec<ParsedItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedItem {
    pub guid: String,
    pub link: String,
    pub title: String,
    pub description: Option<String>,
    pub content: Option<String>,
    /// Publish time in unix seconds, `None` when the feed omits it or it fails
    /// to parse. Kept as `i64` rather than `DateTime<Utc>` so `ParsedItem` stays
    /// `Eq` and chrono never leaks into the feed pipeline.
    pub published: Option<i64>,
}

pub fn parse_feed(bytes: &[u8]) -> anyhow::Result<ParsedFeed> {
    let feed = parser::parse(bytes)?;
    let items = feed.entries.iter().map(parse_entry).collect::<Vec<_>>();
    Ok(ParsedFeed { title: feed.title.map(|t| t.content), items })
}

fn parse_entry(entry: &Entry) -> ParsedItem {
    let link = entry.links.first().map(|l| l.href.clone()).unwrap_or_default();
    // Compatibility note: feed-rs normalises RSS <guid> and Atom <id> into
    // `Entry::id`. If absent, use the item link as specified in docs/02; dry-run
    // against production data.db remains the gate for detecting a Go mismatch.
    let guid = if entry.id.is_empty() { link.clone() } else { entry.id.clone() };
    let title = entry.title.as_ref().map(|t| t.content.clone()).unwrap_or_default();
    let description = entry.summary.as_ref().map(|s| s.content.clone());
    let content = entry.content.as_ref().and_then(|c| c.body.clone());
    // RSS <pubDate> / Atom <published>; fall back to <updated> for feeds (a lot
    // of Atom generators) that only ever emit the latter.
    let published = entry.published.or(entry.updated).map(|d| d.timestamp());

    ParsedItem { guid, link, title, description, content, published }
}

/// Age gate for the two push paths (scheduler and `/check`).
///
/// The dedup ledger alone cannot answer "is this actually new?": a feed that
/// churns its GUIDs (platform migration, http→https, trailing-slash change) or
/// a ledger row dropped by `prune_contents` both make a decade-old post look
/// brand new, and the whole archive gets blasted out. This is the second gate.
///
/// Deliberately permissive in two cases: an item with no date at all is *not*
/// stale (we cannot judge it, and silently swallowing every update from feeds
/// that omit `<pubDate>` would be worse), and neither is a future-dated one
/// (publisher clock skew). `max_age_days == 0` disables the gate entirely.
pub fn is_stale_item(published: Option<i64>, now: i64, max_age_days: u32) -> bool {
    if max_age_days == 0 {
        return false;
    }
    let Some(published) = published else {
        return false;
    };
    let cutoff = now - i64::from(max_age_days) * 86_400;
    published < cutoff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_guid_link_title_and_content() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>Example Feed</title>
<item><guid>post-1</guid><title>Hello</title><link>https://example.com/1</link><description>Summary</description><content:encoded xmlns:content="http://purl.org/rss/1.0/modules/content/"><![CDATA[<p>Body</p>]]></content:encoded></item>
</channel></rss>"#;
        let parsed = parse_feed(xml).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("Example Feed"));
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].guid, "post-1");
        assert_eq!(parsed.items[0].link, "https://example.com/1");
        assert_eq!(parsed.items[0].title, "Hello");
    }

    #[test]
    fn parses_rss_pubdate_into_published() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>Dated</title>
<item><guid>a</guid><title>A</title><link>https://example.com/a</link><pubDate>Tue, 01 Mar 2022 00:00:00 GMT</pubDate></item>
</channel></rss>"#;
        let parsed = parse_feed(xml).unwrap();
        assert_eq!(parsed.items[0].published, Some(1_646_092_800));
    }

    /// Atom feeds commonly ship only `<updated>`; fall back to it so those
    /// entries still get an age, instead of bypassing the gate as undated.
    #[test]
    fn falls_back_to_atom_updated_when_published_is_absent() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>Atom</title>
<entry><id>urn:a</id><title>A</title><link href="https://example.com/a"/><updated>2022-03-01T00:00:00Z</updated></entry>
</feed>"#;
        let parsed = parse_feed(xml).unwrap();
        assert_eq!(parsed.items[0].published, Some(1_646_092_800));
    }

    #[test]
    fn undated_item_has_no_published() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>Undated</title>
<item><guid>a</guid><title>A</title><link>https://example.com/a</link></item>
</channel></rss>"#;
        let parsed = parse_feed(xml).unwrap();
        assert_eq!(parsed.items[0].published, None);
    }

    #[test]
    fn stale_gate_only_trips_on_dated_items_older_than_the_cutoff() {
        const DAY: i64 = 86_400;
        let now = 1_700_000_000;

        assert!(is_stale_item(Some(now - 31 * DAY), now, 30));
        // Exactly on the cutoff is not stale — the comparison is strict.
        assert!(!is_stale_item(Some(now - 30 * DAY), now, 30));
        assert!(!is_stale_item(Some(now - 29 * DAY), now, 30));
        // Undated items are unjudgeable, so they pass.
        assert!(!is_stale_item(None, now, 30));
        // Publisher clock skew must not silently drop a genuinely new item.
        assert!(!is_stale_item(Some(now + 10 * DAY), now, 30));
        // 0 disables the gate.
        assert!(!is_stale_item(Some(now - 3650 * DAY), now, 0));
    }
}
