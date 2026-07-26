use crate::{
    db::{
        models::{Content, Source},
        repo::Repo,
    },
    feed::{fetch::FetchOutcome, fetch::Fetcher, hash::gen_hash_id, parse::parse_feed},
};

/// Port of Go's `Core.CreateSource` + `AddSourceContents`: return the
/// existing source if already known, otherwise fetch and parse the feed once
/// to learn its real title, then pre-populate `contents` with every item
/// currently in the feed. This is what keeps a freshly subscribed feed from
/// blasting its entire back catalogue to the new subscriber on the next
/// scheduler pass (the same dedup ledger the `--dry-run` gate checks).
pub async fn create_source(repo: &Repo, fetcher: &Fetcher, link: &str) -> anyhow::Result<Source> {
    if let Some(source) = repo.source_by_link(link).await? {
        return Ok(source);
    }

    let feed = match fetcher.fetch(link, None, None).await? {
        FetchOutcome::Modified(feed) => feed,
        FetchOutcome::Unchanged => anyhow::bail!("unexpected 304 fetching a new source"),
    };
    let parsed = parse_feed(&feed.body)?;
    let title = parsed.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| link.to_owned());

    let source_id = repo.insert_source(link, &title).await?;
    for item in &parsed.items {
        repo.insert_content(&Content {
            source_id: Some(source_id),
            hash_id: gen_hash_id(link, &item.guid),
            raw_id: Some(item.guid.clone()),
            raw_link: Some(item.link.clone()),
            title: Some(item.title.clone()),
            telegraph_url: None,
            created_at: None,
            updated_at: None,
        })
        .await?;
    }

    Ok(repo.get_source(source_id).await?.expect("source was just inserted"))
}
