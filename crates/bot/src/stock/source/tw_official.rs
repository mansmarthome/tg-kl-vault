//! TWSE / TPEx official daily-close dumps, used **only** as a Taiwan fallback
//! when Yahoo is unavailable. Keyless, official, but large (TPEx ~4 MB) and
//! only ever one session deep — so this provides a single day's close, never
//! history, and the service omits the indicator block when it has to rely on it.
//!
//! The one non-negotiable rule (design D4): a dump is trusted **only if its own
//! `Date` equals the trading day we are trying to report**. Verified 2026-08-21,
//! at 22:45 Taipei the TWSE dump still carried *yesterday*'s date while TPEx
//! carried today's — without this gate the fallback would confidently print
//! yesterday's close as today's.
//!
//! CAVEAT: the exact JSON field names below are **not** verified against a live
//! response (no half-day/off-hours sample was available when this was written).
//! Extraction is tolerant (tries several candidate keys) and the date gate
//! **fails safe** — a dump whose date we cannot parse is refused, so the worst
//! case is "no fallback", never "wrong fallback". Confirm field names against a
//! real payload on first use.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::StreamExt;
use reqwest::Client;
use serde_json::{Map, Value};
use tracing::warn;

use super::super::symbol::{Board, Symbol};
use super::SourceError;
use super::roc::{parse_tw_number, roc_to_iso};

/// Re-fetch a dump at most this often; within the window the cached parse is
/// reused so a room full of chats sharing the fallback costs one download.
const DUMP_TTL: Duration = Duration::from_secs(300);
/// Default body cap (32 MiB): comfortably above today's 4 MB TPEx payload but
/// bounded, since this bypasses `Fetcher::max_body_bytes`.
pub const DEFAULT_MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// One day's close for one Taiwan symbol from the official dump.
#[derive(Debug, Clone, PartialEq)]
pub struct TwBar {
    pub trade_date: String,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<i64>,
    pub change: Option<f64>,
    pub source: &'static str, // "twse" | "tpex"
}

struct Dump {
    fetched_at: Instant,
    /// The payload's own trading day (ISO), or empty if it carried none.
    date: String,
    /// Keyed by bare local code (`2330`, `6488`).
    quotes: std::collections::HashMap<String, TwBar>,
}

#[derive(Default)]
struct Cache {
    twse: Option<Dump>,
    tpex: Option<Dump>,
}

pub struct TwOfficialSource {
    client: Client,
    twse_endpoint: String,
    tpex_endpoint: String,
    max_body_bytes: usize,
    cache: Mutex<Cache>,
}

impl TwOfficialSource {
    pub fn new(client: Client, twse_endpoint: String, tpex_endpoint: String) -> Self {
        Self {
            client,
            twse_endpoint: twse_endpoint.trim_end_matches('/').to_owned(),
            tpex_endpoint: tpex_endpoint.trim_end_matches('/').to_owned(),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            cache: Mutex::new(Cache::default()),
        }
    }

    /// Returns a fallback close for `sym` **only** if the official dump's own
    /// date equals `want_trade_date`. `Ok(None)` means "no trustworthy
    /// fallback" — either the symbol is absent or the dump is stale/dateless.
    pub async fn fallback_quote(
        &self,
        sym: &Symbol,
        want_trade_date: &str,
    ) -> anyhow::Result<Option<TwBar>> {
        let board = sym.board;
        if !matches!(board, Board::Twse | Board::Tpex) {
            return Ok(None);
        }
        self.ensure_fresh(board).await?;

        let cache = self.cache.lock().unwrap();
        let Some(dump) = board_slot(&cache, board) else {
            return Ok(None);
        };
        // The gate. A mismatched or empty date is refused.
        if dump.date.is_empty() || dump.date != want_trade_date {
            return Ok(None);
        }
        Ok(dump.quotes.get(&sym.local_code).cloned())
    }

    async fn ensure_fresh(&self, board: Board) -> anyhow::Result<()> {
        let stale = {
            let cache = self.cache.lock().unwrap();
            board_slot(&cache, board).is_none_or(|d| d.fetched_at.elapsed() > DUMP_TTL)
        };
        if !stale {
            return Ok(());
        }
        let dump = self.fetch_dump(board).await?;
        let mut cache = self.cache.lock().unwrap();
        match board {
            Board::Twse => cache.twse = Some(dump),
            Board::Tpex => cache.tpex = Some(dump),
            Board::Us => {}
        }
        Ok(())
    }

    async fn fetch_dump(&self, board: Board) -> anyhow::Result<Dump> {
        let (url, source) = match board {
            Board::Twse => (
                format!("{}/exchangeReport/STOCK_DAY_ALL", self.twse_endpoint),
                "twse",
            ),
            Board::Tpex => (
                format!("{}/tpex_mainboard_daily_close_quotes", self.tpex_endpoint),
                "tpex",
            ),
            Board::Us => return Ok(empty_dump()),
        };
        let body = self.fetch_capped(&url).await?;
        Ok(parse_dump(&body, source))
    }

    /// Streams the body with a hard cap so a runaway dump can't OOM the process,
    /// warning past 80% so growth is visible before it becomes a failure.
    async fn fetch_capped(&self, url: &str) -> anyhow::Result<String> {
        let resp = self.client.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(super::status_to_error(status.as_u16()).into());
        }
        let mut stream = resp.bytes_stream();
        let mut buf = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if buf.len() + chunk.len() > self.max_body_bytes {
                return Err(SourceError::Malformed(format!(
                    "tw dump exceeds {} byte cap",
                    self.max_body_bytes
                ))
                .into());
            }
            buf.extend_from_slice(&chunk);
        }
        if buf.len() * 5 > self.max_body_bytes * 4 {
            warn!(
                bytes = buf.len(),
                cap = self.max_body_bytes,
                "tw official dump is over 80% of the body cap; raise stock body cap soon"
            );
        }
        String::from_utf8(buf).map_err(|e| SourceError::Malformed(e.to_string()).into())
    }
}

fn board_slot(cache: &Cache, board: Board) -> Option<&Dump> {
    match board {
        Board::Twse => cache.twse.as_ref(),
        Board::Tpex => cache.tpex.as_ref(),
        Board::Us => None,
    }
}

fn empty_dump() -> Dump {
    Dump {
        fetched_at: Instant::now(),
        date: String::new(),
        quotes: std::collections::HashMap::new(),
    }
}

/// Tries each candidate key in order, returning the first present string value.
fn field<'a>(rec: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| rec.get(*k).and_then(Value::as_str))
}

/// Parses a dump body (a JSON array of records, optionally wrapped in a `data`
/// field). Fail-safe: an unrecognizable body yields an empty, dateless dump,
/// which the gate then refuses.
fn parse_dump(body: &str, source: &'static str) -> Dump {
    let records = match serde_json::from_str::<Value>(body) {
        Ok(Value::Array(rows)) => rows,
        Ok(Value::Object(obj)) => obj
            .get("data")
            .or_else(|| obj.get("aaData"))
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    let mut date = String::new();
    let mut quotes = std::collections::HashMap::new();
    for rec in records {
        let Value::Object(rec) = rec else { continue };
        let Some(code) = field(&rec, &["Code", "SecuritiesCompanyCode", "股票代號"]) else {
            continue;
        };
        if date.is_empty() {
            if let Some(iso) = field(&rec, &["Date", "日期"]).and_then(roc_to_iso) {
                date = iso;
            }
        }
        let bar = TwBar {
            trade_date: date.clone(),
            open: field(&rec, &["OpeningPrice", "Open", "開盤價"]).and_then(parse_tw_number),
            high: field(&rec, &["HighestPrice", "High", "最高價"]).and_then(parse_tw_number),
            low: field(&rec, &["LowestPrice", "Low", "最低價"]).and_then(parse_tw_number),
            close: field(&rec, &["ClosingPrice", "Close", "收盤價"]).and_then(parse_tw_number),
            volume: field(&rec, &["TradeVolume", "TradingShares", "成交股數"])
                .and_then(parse_tw_number)
                .map(|v| v as i64),
            change: field(&rec, &["Change", "漲跌"]).and_then(parse_tw_number),
            source,
        };
        quotes.insert(code.to_owned(), bar);
    }
    // Backfill each bar's trade_date once we know the dump date (the first row
    // may have preceded the date discovery in a malformed feed).
    if !date.is_empty() {
        for bar in quotes.values_mut() {
            if bar.trade_date.is_empty() {
                bar.trade_date = date.clone();
            }
        }
    }

    Dump {
        fetched_at: Instant::now(),
        date,
        quotes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stock::symbol::{parse, Parsed};
    use crate::testutil::spawn_scripted_server;

    // Minimal TPEx-shaped fixture: has Date, a Chinese Change, and a plain one.
    const TPEX_TODAY: &str = r#"[
      {"Date":"1150821","SecuritiesCompanyCode":"6488","Close":"720.00","Open":"715.00","High":"725.00","Low":"710.00","TradingShares":"1234567","Change":"-52.00 "},
      {"Date":"1150821","SecuritiesCompanyCode":"3105","Close":"420.00","Open":"420.00","High":"422.00","Low":"418.00","TradingShares":"7654321","Change":"除息 "}
    ]"#;

    // TWSE-shaped fixture carrying YESTERDAY's date.
    const TWSE_STALE: &str = r#"[
      {"Date":"1150820","Code":"2330","ClosingPrice":"2410.00","OpeningPrice":"2400.00","HighestPrice":"2415.00","LowestPrice":"2395.00","TradeVolume":"17158844","Change":"10.00"}
    ]"#;

    fn sym(raw: &str) -> Symbol {
        match parse(raw) {
            Parsed::Resolved(s) => s,
            other => panic!("{raw} -> {other:?}"),
        }
    }

    #[test]
    fn parse_dump_reads_date_and_quotes() {
        let dump = parse_dump(TPEX_TODAY, "tpex");
        assert_eq!(dump.date, "2026-08-21");
        let bar = &dump.quotes["6488"];
        assert_eq!(bar.close, Some(720.0));
        assert_eq!(bar.change, Some(-52.0));
        assert_eq!(bar.volume, Some(1_234_567));
        // Chinese Change -> None, not 0.0.
        assert_eq!(dump.quotes["3105"].change, None);
    }

    #[tokio::test]
    async fn fallback_is_served_when_the_dump_date_matches() {
        let base = spawn_scripted_server(vec![(200, TPEX_TODAY)]).await;
        let src = TwOfficialSource::new(Client::new(), "http://unused".into(), base);
        let bar = src.fallback_quote(&sym("6488.TWO"), "2026-08-21").await.unwrap();
        assert_eq!(bar.unwrap().close, Some(720.0));
    }

    #[tokio::test]
    async fn taiwan_fallback_is_rejected_when_its_own_date_is_stale() {
        // TWSE dump says 2026-08-20 but we want to report 2026-08-21.
        let base = spawn_scripted_server(vec![(200, TWSE_STALE)]).await;
        let src = TwOfficialSource::new(Client::new(), base, "http://unused".into());
        let bar = src.fallback_quote(&sym("2330.TW"), "2026-08-21").await.unwrap();
        assert_eq!(bar, None, "a stale-dated dump must be refused");
        // ...but it IS served for the day it actually covers.
        let base2 = spawn_scripted_server(vec![(200, TWSE_STALE)]).await;
        let src2 = TwOfficialSource::new(Client::new(), base2, "http://unused".into());
        let ok = src2.fallback_quote(&sym("2330.TW"), "2026-08-20").await.unwrap();
        assert_eq!(ok.unwrap().close, Some(2410.0));
    }

    #[tokio::test]
    async fn an_unparseable_dump_fails_safe_to_no_fallback() {
        let base = spawn_scripted_server(vec![(200, "<html>not json</html>")]).await;
        let src = TwOfficialSource::new(Client::new(), base, "http://unused".into());
        let bar = src.fallback_quote(&sym("2330.TW"), "2026-08-21").await.unwrap();
        assert_eq!(bar, None);
    }
}
