//! `/feedcheck` — "are the feeds in my subscription list still working?"
//!
//! Deliberately *not* `/check`. `/check` force-fetches and pushes; this only
//! probes. It never writes `contents`, never sends an article, and never
//! touches `error_count`/`etag` — a health check that silently ingested or
//! un-paused things would be worse than no health check at all.
//!
//! Each feed gets two verdicts side by side: what the DB recorded (migration
//! 0005's `last_error`/`last_success_at`, written by the scheduler) and what a
//! live probe says right now. The combination is the useful part — "DB says
//! broken, probe says fine" is a transient blip, "DB fine, probe says 404" just
//! broke.

use futures::{stream, StreamExt};
use reqwest::StatusCode;
use teloxide::{prelude::*, types::ParseMode};

use crate::{
    bot::{
        render::humanize_ago,
        runtime::{no_preview, now_unix, to_request_error, BotState},
    },
    feed::{
        fetch::{FetchOutcome, Fetcher},
        parse::parse_feed,
    },
};

/// A feed that still parses but whose newest item predates this is alive in the
/// HTTP sense and dead in every sense the user cares about.
const STALE_FEED_DAYS: i64 = 180;

/// Telegram hard-caps messages at 4096 characters; stay well under so a long
/// title near the boundary can never push a chunk over.
const MAX_MESSAGE_CHARS: usize = 3500;

/// Live probe result for one feed. Ordered worst-first for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedHealth {
    /// Transport never completed: DNS, TLS, connect, timeout, body-size cap.
    Unreachable(String),
    /// The server answered, but not with a feed.
    Http(u16),
    /// 200 with a body that is not parseable as RSS/Atom.
    Unparseable(String),
    /// Parses, but contains no entries at all.
    Empty,
    /// Parses, but the newest dated item is ancient — the site stopped posting.
    Abandoned { newest_age_days: i64 },
    /// Healthy. `newest_age_days` is `None` when no item carries a date.
    Ok { items: usize, newest_age_days: Option<i64> },
    /// HTTP 304 against the stored validators — healthy, nothing new.
    NotModified,
}

impl FeedHealth {
    /// Lower sorts first, so what needs attention leads the report.
    fn severity(&self) -> u8 {
        match self {
            Self::Unreachable(_) => 0,
            Self::Http(_) => 1,
            Self::Unparseable(_) => 2,
            Self::Empty => 3,
            Self::Abandoned { .. } => 4,
            Self::Ok { .. } | Self::NotModified => 5,
        }
    }

    fn is_healthy(&self) -> bool {
        matches!(self, Self::Ok { .. } | Self::NotModified)
    }

    fn marker(&self) -> &'static str {
        match self {
            Self::Unreachable(_) => "❌",
            Self::Http(_) => "❌",
            Self::Unparseable(_) => "🧩",
            Self::Empty => "📭",
            Self::Abandoned { .. } => "🪦",
            Self::Ok { .. } | Self::NotModified => "✅",
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Unreachable(err) => format!("連不上：{err}"),
            Self::Http(code) => match StatusCode::from_u16(*code).ok().and_then(|s| s.canonical_reason()) {
                Some(reason) => format!("HTTP {code} {reason}"),
                None => format!("HTTP {code}"),
            },
            Self::Unparseable(err) => format!("不是有效的 RSS/Atom：{err}"),
            Self::Empty => "抓得到，但一篇文章都沒有".to_owned(),
            Self::Abandoned { newest_age_days } => {
                format!("最新一篇是 {newest_age_days} 天前，可能已停更")
            }
            Self::Ok { items, newest_age_days: Some(days) } => {
                format!("正常，{items} 篇，最新 {days} 天前")
            }
            Self::Ok { items, newest_age_days: None } => format!("正常，{items} 篇（無日期）"),
            Self::NotModified => "正常，無更新（304）".to_owned(),
        }
    }
}

/// One subscription's row in the report.
struct Probed {
    source_id: i64,
    title: String,
    link: String,
    health: FeedHealth,
    paused: bool,
    error_count: i64,
    last_error: Option<String>,
    last_success_at: Option<i64>,
}

pub async fn handle_feedcheck(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;
    let subs = state
        .repo
        .subscriptions_for_user(chat_id)
        .await
        .map_err(to_request_error)?;
    if subs.is_empty() {
        bot.send_message(msg.chat.id, "目前沒有訂閱").await?;
        return Ok(());
    }

    bot.send_message(msg.chat.id, format!("正在檢查{}個訂閱源…", subs.len()))
        .await?;

    let now = now_unix();
    // `fetch.concurrency` finally does something: the scheduler is strictly
    // sequential, but a manual check of 100 feeds at 30s timeout each would
    // otherwise take the better part of an hour.
    let concurrency = state.config.fetch.concurrency.max(1);
    let mut probed: Vec<Probed> = stream::iter(subs.into_iter().filter(|s| s.source_id.is_some()))
        .map(|sub| async move {
            let link = sub.link.clone().unwrap_or_default();
            let health = if link.is_empty() {
                FeedHealth::Unreachable("訂閱未記錄網址".to_owned())
            } else {
                probe(
                    &state.fetcher,
                    &link,
                    sub.etag.as_deref(),
                    sub.last_modified.as_deref(),
                    now,
                )
                .await
            };
            Probed {
                source_id: sub.source_id.unwrap_or_default(),
                title: sub.title.clone().unwrap_or_default(),
                link,
                health,
                paused: sub.is_paused(),
                error_count: sub.error_count.unwrap_or(0),
                last_error: sub.last_error.clone(),
                last_success_at: sub.last_success_at,
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    probed.sort_by_key(|p| (p.health.severity(), p.source_id));

    for chunk in chunk_lines(&render_report(&probed, now), MAX_MESSAGE_CHARS) {
        bot.send_message(msg.chat.id, chunk)
            .parse_mode(ParseMode::Html)
            .link_preview_options(no_preview())
            .await?;
    }
    Ok(())
}

/// Read-only probe. Sends the stored validators so an unchanged feed answers
/// 304 cheaply, but stores nothing back — persisting a validator here would
/// make the *next* scheduler pass believe it had already seen this response.
///
/// Takes the `Fetcher` rather than the whole `BotState` on purpose: with no
/// `Repo` in scope, "this never writes to the database" is enforced by the
/// signature instead of by a comment.
async fn probe(
    fetcher: &Fetcher,
    link: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    now: i64,
) -> FeedHealth {
    match fetcher.fetch(link, etag, last_modified).await {
        Ok(FetchOutcome::Unchanged) => FeedHealth::NotModified,
        Ok(FetchOutcome::Modified(feed)) => match parse_feed(&feed.body) {
            Ok(parsed) if parsed.items.is_empty() => FeedHealth::Empty,
            Ok(parsed) => {
                let newest_age_days = parsed
                    .items
                    .iter()
                    .filter_map(|item| item.published)
                    .max()
                    .map(|published| now.saturating_sub(published).max(0) / 86_400);
                match newest_age_days {
                    Some(days) if days > STALE_FEED_DAYS => {
                        FeedHealth::Abandoned { newest_age_days: days }
                    }
                    _ => FeedHealth::Ok { items: parsed.items.len(), newest_age_days },
                }
            }
            Err(err) => FeedHealth::Unparseable(first_line(&err.to_string())),
        },
        Err(err) => classify_fetch_error(&err),
    }
}

/// `Fetcher::fetch` erases everything into `anyhow::Error`, so recover the
/// distinction the user actually needs: "the site said no" (fixable by
/// unsubscribing) vs "we could not reach it" (probably transient).
fn classify_fetch_error(err: &anyhow::Error) -> FeedHealth {
    if let Some(req_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = req_err.status() {
            return FeedHealth::Http(status.as_u16());
        }
        if req_err.is_timeout() {
            return FeedHealth::Unreachable("請求逾時".to_owned());
        }
        if req_err.is_connect() {
            return FeedHealth::Unreachable("無法建立連線".to_owned());
        }
        if req_err.is_redirect() {
            return FeedHealth::Unreachable("重新導向次數過多".to_owned());
        }
    }
    FeedHealth::Unreachable(first_line(&err.to_string()))
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    match line.char_indices().nth(120) {
        Some((idx, _)) => format!("{}…", &line[..idx]),
        None => line.to_owned(),
    }
}

/// Builds the report as individual lines so `chunk_lines` can split it without
/// ever cutting a line in half.
fn render_report(probed: &[Probed], now: i64) -> Vec<String> {
    let total = probed.len();
    let unhealthy = probed.iter().filter(|p| !p.health.is_healthy()).count();
    let paused = probed.iter().filter(|p| p.paused).count();

    let mut lines = vec![format!(
        "<b>訂閱健檢</b>：共{total}個源，{}個有問題，{paused}個已暫停",
        unhealthy
    )];

    if unhealthy == 0 && paused == 0 {
        lines.push(String::new());
        lines.push("全部正常 🎉".to_owned());
        return lines;
    }

    lines.push(String::new());
    for p in probed.iter().filter(|p| !p.health.is_healthy() || p.paused) {
        lines.push(format!(
            "{} [{}] <a href=\"{}\">{}</a>",
            p.health.marker(),
            p.source_id,
            escape(&p.link),
            escape(if p.title.is_empty() { &p.link } else { &p.title })
        ));
        lines.push(format!("　└ {}", escape(&p.health.describe())));

        if p.paused {
            lines.push("　└ ⏸ 已暫停抓取，用 /set 選此源後可恢復".to_owned());
        }
        // The DB's memory of past failures, which the live probe above cannot
        // see. Disagreement between the two is the informative case.
        if p.error_count > 0 {
            let detail = p.last_error.as_deref().unwrap_or("未記錄原因");
            lines.push(format!(
                "　└ 排程記錄：連續失敗{}次（{}）",
                p.error_count,
                escape(&first_line(detail))
            ));
        }
        lines.push(format!(
            "　└ 上次成功抓取：{}",
            humanize_ago(p.last_success_at, now)
        ));
    }

    let healthy = total - probed.iter().filter(|p| !p.health.is_healthy() || p.paused).count();
    lines.push(String::new());
    lines.push(format!("其餘{healthy}個源正常。失效的可用 /unsub 退訂。"));
    lines
}

/// Packs lines into messages under `limit` characters, never splitting a line.
/// A single line longer than the limit gets its own (over-limit) message rather
/// than being silently truncated — better a rejected send than a lie.
fn chunk_lines(lines: &[String], limit: usize) -> Vec<String> {
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

/// Same minimal HTML escaping the rest of the bot uses for feed-derived text.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        testutil::{spawn_single_response_server, spawn_single_response_server_with},
    };

    const GOOD_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>Good</title>
<item><guid>a</guid><title>A</title><link>https://example.com/a</link><pubDate>Tue, 01 Mar 2022 00:00:00 GMT</pubDate></item>
</channel></rss>"#;

    const EMPTY_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>Empty</title></channel></rss>"#;

    fn fetcher() -> Fetcher {
        Fetcher::new(&Config::default()).unwrap()
    }

    /// 2022-03-01 plus a bit, so `GOOD_FEED` reads as recent in these tests
    /// without the assertions drifting as real time passes.
    const NOW_JUST_AFTER_FEED: i64 = 1_646_092_800 + 86_400;

    #[tokio::test]
    async fn probe_classifies_a_healthy_feed() {
        let url = spawn_single_response_server(GOOD_FEED).await;
        let health = probe(&fetcher(), &url, None, None, NOW_JUST_AFTER_FEED).await;
        assert_eq!(health, FeedHealth::Ok { items: 1, newest_age_days: Some(1) });
        assert!(health.is_healthy());
    }

    #[tokio::test]
    async fn probe_classifies_a_dead_url_by_status() {
        let url = spawn_single_response_server_with(404, "text/html", "nope").await;
        assert_eq!(probe(&fetcher(), &url, None, None, 0).await, FeedHealth::Http(404));
    }

    #[tokio::test]
    async fn probe_classifies_a_page_that_is_not_a_feed() {
        let url = spawn_single_response_server_with(200, "text/html", "<html><body>hi</body></html>").await;
        let health = probe(&fetcher(), &url, None, None, 0).await;
        assert!(matches!(health, FeedHealth::Unparseable(_)), "got {health:?}");
    }

    #[tokio::test]
    async fn probe_classifies_a_feed_with_no_entries() {
        let url = spawn_single_response_server(EMPTY_FEED).await;
        assert_eq!(probe(&fetcher(), &url, None, None, 0).await, FeedHealth::Empty);
    }

    /// Still serving 200 and still parsing, but nothing new in years — the case
    /// a pure HTTP check would happily call healthy.
    #[tokio::test]
    async fn probe_flags_an_abandoned_feed() {
        let url = spawn_single_response_server(GOOD_FEED).await;
        let five_years_on = 1_646_092_800 + 5 * 365 * 86_400;
        let health = probe(&fetcher(), &url, None, None, five_years_on).await;
        assert!(matches!(health, FeedHealth::Abandoned { .. }), "got {health:?}");
        assert!(!health.is_healthy());
    }

    #[tokio::test]
    async fn probe_reports_an_unreachable_host_without_a_status() {
        // Port 1 on loopback: connection refused, no HTTP status at all.
        let health = probe(&fetcher(), "http://127.0.0.1:1/feed", None, None, 0).await;
        assert!(matches!(health, FeedHealth::Unreachable(_)), "got {health:?}");
    }

    fn probed(source_id: i64, health: FeedHealth) -> Probed {
        Probed {
            source_id,
            title: format!("Feed {source_id}"),
            link: format!("https://example.com/{source_id}"),
            health,
            paused: false,
            error_count: 0,
            last_error: None,
            last_success_at: None,
        }
    }

    #[test]
    fn report_leads_with_the_broken_feeds() {
        let mut all = vec![
            probed(1, FeedHealth::Ok { items: 10, newest_age_days: Some(2) }),
            probed(2, FeedHealth::Http(404)),
            probed(3, FeedHealth::Empty),
            probed(4, FeedHealth::Unreachable("請求逾時".to_owned())),
        ];
        all.sort_by_key(|p| (p.health.severity(), p.source_id));
        assert_eq!(all.iter().map(|p| p.source_id).collect::<Vec<_>>(), vec![4, 2, 3, 1]);

        let text = chunk_lines(&render_report(&all, 0), MAX_MESSAGE_CHARS).join("\n");
        assert!(text.contains("共4個源，3個有問題"));
        assert!(text.contains("HTTP 404 Not Found"));
        assert!(text.contains("其餘1個源正常"));
        // Healthy feeds are summarised, not enumerated.
        assert!(!text.contains("Feed 1</a>"));
    }

    #[test]
    fn all_healthy_report_says_so_without_listing_anything() {
        let all = vec![
            probed(1, FeedHealth::Ok { items: 3, newest_age_days: Some(1) }),
            probed(2, FeedHealth::NotModified),
        ];
        let text = chunk_lines(&render_report(&all, 0), MAX_MESSAGE_CHARS).join("\n");
        assert!(text.contains("全部正常"));
        assert!(!text.contains("其餘"));
    }

    /// A paused source is invisible to the scheduler, so it must be reported
    /// even when a live probe finds it perfectly healthy.
    #[test]
    fn paused_sources_are_reported_even_when_the_probe_succeeds() {
        let mut p = probed(1, FeedHealth::Ok { items: 5, newest_age_days: Some(1) });
        p.paused = true;
        p.error_count = 101;
        p.last_error = Some("HTTP status client error (503)".to_owned());
        let text = chunk_lines(&render_report(&[p], 0), MAX_MESSAGE_CHARS).join("\n");
        assert!(text.contains("已暫停抓取"));
        assert!(text.contains("連續失敗101次"));
        assert!(text.contains("正常，5 篇"), "the live probe verdict is still shown");
    }

    #[test]
    fn feed_text_is_html_escaped() {
        let mut p = probed(1, FeedHealth::Unparseable("expected <feed> & got".to_owned()));
        p.title = "A & B <script>".to_owned();
        let text = chunk_lines(&render_report(&[p], 0), MAX_MESSAGE_CHARS).join("\n");
        assert!(text.contains("A &amp; B &lt;script&gt;"));
        assert!(text.contains("expected &lt;feed&gt; &amp; got"));
        assert!(!text.contains("<script>"));
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
    fn first_line_trims_multiline_errors_on_a_char_boundary() {
        assert_eq!(first_line("boom\nstack trace here"), "boom");
        let long = "錯".repeat(300);
        let trimmed = first_line(&long);
        assert_eq!(trimmed.chars().count(), 121);
    }
}
