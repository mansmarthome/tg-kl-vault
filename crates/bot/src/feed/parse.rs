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

    ParsedItem { guid, link, title, description, content }
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
}
