//! Yahoo Finance source: `v8/finance/chart` (full OHLCV, one symbol) and
//! `v7/finance/spark` (close-only, batched). Both endpoints are undocumented and
//! keyless — verified working 2026-08-21 but not a contract, which is why the TW
//! official fallback (step 6) and the config endpoint overrides exist.
//!
//! Two spark traps are pinned by tests below and must never be "fixed away":
//!   * **results are unordered** — index by `result[].symbol`, never position.
//!   * **unknown symbols vanish silently** — the caller computes
//!     `requested − returned = missing`; absence is not "no change".
//!
//! The serde structs are deliberately *partial*: every optional field is
//! `#[serde(default)]` and there is no `deny_unknown_fields`. Yahoo adds fields
//! without warning, and a strict struct would turn one cosmetic change into a
//! total outage.

use reqwest::Client;
use serde::Deserialize;

use super::super::bars::{Bar, Series};
use super::super::symbol::{Board, Symbol};
use super::{status_to_error, SourceError, StockSource};

pub struct YahooSource {
    client: Client,
    /// Base URL, e.g. `https://query1.finance.yahoo.com` (no trailing slash).
    endpoint: String,
}

impl YahooSource {
    pub fn new(client: Client, endpoint: String) -> Self {
        Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_owned(),
        }
    }

    async fn fetch(&self, url: &str, params: &[(&str, &str)]) -> anyhow::Result<String> {
        let resp = self.client.get(url).query(params).send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp.text().await?);
        }
        Err(status_to_error(status.as_u16()).into())
    }

    /// Batched daily poll. Yahoo caps spark at 20 symbols per call (verified);
    /// the caller chunks. Returns `(symbol, Series)` pairs — a symbol the caller
    /// requested but that is absent here simply did not come back.
    pub async fn spark_batch(&self, symbols: &[&str]) -> anyhow::Result<Vec<(String, Series)>> {
        if symbols.is_empty() {
            return Ok(Vec::new());
        }
        let csv = symbols.join(",");
        let url = format!("{}/v7/finance/spark", self.endpoint);
        let body = self
            .fetch(&url, &[("symbols", csv.as_str()), ("range", "5d"), ("interval", "1d")])
            .await?;
        Ok(parse_spark(&body)?)
    }
}

impl StockSource for YahooSource {
    async fn series(&self, sym: &Symbol, days: u16) -> anyhow::Result<Series> {
        let url = format!("{}/v8/finance/chart/{}", self.endpoint, sym.canonical);
        let body = self
            .fetch(&url, &[("interval", "1d"), ("range", range_for_days(days))])
            .await?;
        Ok(parse_chart(&body)?)
    }

    fn supports(&self, _board: Board) -> bool {
        true // Yahoo serves TWSE, TPEx, and US alike.
    }

    fn name(&self) -> &'static str {
        "yahoo"
    }
}

fn range_for_days(days: u16) -> &'static str {
    match days {
        0..=5 => "5d",
        6..=30 => "1mo",
        31..=90 => "3mo",
        91..=180 => "6mo",
        181..=365 => "1y",
        _ => "2y",
    }
}

// --- wire structs (partial by design) ---

#[derive(Deserialize)]
struct ChartEnvelope {
    #[serde(default)]
    chart: Body,
}

#[derive(Deserialize)]
struct SparkEnvelope {
    #[serde(default)]
    spark: Body,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Body {
    result: Option<Vec<ResultEntry>>,
    error: Option<ApiError>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ApiError {
    code: String,
    description: String,
}

/// One `chart.result[i]` entry, and also one `spark.result[i].response[0]`
/// entry (spark carries the same shape with only `close` populated).
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ResultEntry {
    // spark wraps the chart shape one level deeper under `response`.
    symbol: String,
    response: Vec<Inner>,
    #[serde(flatten)]
    inner: Inner,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Inner {
    meta: Meta,
    timestamp: Vec<i64>,
    indicators: Indicators,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Meta {
    exchange_name: String,
    currency: String,
    long_name: String,
    short_name: String,
    gmtoffset: i64,
    regular_market_time: i64,
    regular_market_price: Option<f64>,
    chart_previous_close: Option<f64>,
    previous_close: Option<f64>,
    fifty_two_week_high: Option<f64>,
    fifty_two_week_low: Option<f64>,
    current_trading_period: Option<CurrentTradingPeriod>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CurrentTradingPeriod {
    regular: Period,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Period {
    start: i64,
    end: i64,
    gmtoffset: i64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Indicators {
    quote: Vec<Quote>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Quote {
    open: Vec<Option<f64>>,
    high: Vec<Option<f64>>,
    low: Vec<Option<f64>>,
    close: Vec<Option<f64>>,
    volume: Vec<Option<i64>>,
}

/// Builds a `Series` from one chart/spark inner result. OHLCV are parallel
/// arrays that may each be `null` at any index and may be *shorter* than
/// `timestamp`; every access is `.get(i).copied().flatten()`, never `[i]`, so a
/// mismatch yields `None` rather than a panic, and a `null` close becomes `None`
/// (never zero-filled, which would poison every downstream indicator).
fn inner_to_series(symbol: &str, r: &Inner) -> Series {
    let q = r.indicators.quote.first();
    let bars = r
        .timestamp
        .iter()
        .enumerate()
        .map(|(i, &ts)| Bar {
            ts,
            open: q.and_then(|q| q.open.get(i).copied().flatten()),
            high: q.and_then(|q| q.high.get(i).copied().flatten()),
            low: q.and_then(|q| q.low.get(i).copied().flatten()),
            close: q.and_then(|q| q.close.get(i).copied().flatten()),
            volume: q.and_then(|q| q.volume.get(i).copied().flatten()),
        })
        .collect();

    let regular = r.meta.current_trading_period.as_ref().map(|p| &p.regular);
    let display_name = [r.meta.long_name.as_str(), r.meta.short_name.as_str(), symbol]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or(symbol)
        .to_owned();

    Series {
        bars,
        gmtoffset: r.meta.gmtoffset,
        regular_start: regular.map_or(0, |p| p.start),
        regular_end: regular.map_or(0, |p| p.end),
        market_time: r.meta.regular_market_time,
        last_price: r.meta.regular_market_price,
        prev_close: r.meta.chart_previous_close.or(r.meta.previous_close),
        week52_high: r.meta.fifty_two_week_high,
        week52_low: r.meta.fifty_two_week_low,
        exchange: r.meta.exchange_name.clone(),
        display_name,
        currency: r.meta.currency.clone(),
    }
}

fn envelope_error(err: &ApiError) -> SourceError {
    if err.code.eq_ignore_ascii_case("Not Found") {
        SourceError::NotFound
    } else {
        SourceError::Malformed(format!("{}: {}", err.code, err.description))
    }
}

pub(crate) fn parse_chart(body: &str) -> Result<Series, SourceError> {
    let env: ChartEnvelope =
        serde_json::from_str(body).map_err(|e| SourceError::Malformed(e.to_string()))?;
    if let Some(err) = &env.chart.error {
        return Err(envelope_error(err));
    }
    let result = env.chart.result.unwrap_or_default();
    let entry = result.first().ok_or(SourceError::NotFound)?;
    Ok(inner_to_series("", &entry.inner))
}

pub(crate) fn parse_spark(body: &str) -> Result<Vec<(String, Series)>, SourceError> {
    let env: SparkEnvelope =
        serde_json::from_str(body).map_err(|e| SourceError::Malformed(e.to_string()))?;
    if let Some(err) = &env.spark.error {
        return Err(envelope_error(err));
    }
    Ok(env
        .spark
        .result
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| {
            r.response
                .first()
                .map(|inner| (r.symbol.clone(), inner_to_series(&r.symbol, inner)))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stock::symbol::{parse, Parsed};
    use crate::testutil::spawn_scripted_server;

    const CHART_AAPL: &str = r#"{"chart":{"result":[{"meta":{"symbol":"AAPL","exchangeName":"NMS","currency":"USD","regularMarketTime":1787324271,"gmtoffset":-14400,"longName":"Apple Inc.","regularMarketPrice":230.5,"chartPreviousClose":228.0,"fiftyTwoWeekHigh":260.1,"fiftyTwoWeekLow":164.0,"currentTradingPeriod":{"regular":{"start":1787319000,"end":1787342400,"gmtoffset":-14400}}},"timestamp":[1787232600,1787319000,1787405400],"indicators":{"quote":[{"open":[227.0,229.0,231.0],"high":[228.5,231.0,232.0],"low":[226.0,228.0,230.0],"close":[228.0,null,230.5],"volume":[1000000,1100000,900000]}]}}],"error":null}}"#;

    const CHART_NOT_FOUND: &str = r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#;

    // Response order AAPL, 2330.TW, MSFT — deliberately NOT the request order.
    const SPARK_UNORDERED: &str = r#"{"spark":{"result":[
      {"symbol":"AAPL","response":[{"meta":{"symbol":"AAPL","gmtoffset":-14400,"currency":"USD","regularMarketPrice":230.5,"currentTradingPeriod":{"regular":{"start":1787319000,"end":1787342400,"gmtoffset":-14400}}},"timestamp":[1787319000],"indicators":{"quote":[{"close":[230.5]}]}}]},
      {"symbol":"2330.TW","response":[{"meta":{"symbol":"2330.TW","gmtoffset":28800,"currency":"TWD","regularMarketPrice":2410.0},"timestamp":[1787274000],"indicators":{"quote":[{"close":[2410.0]}]}}]},
      {"symbol":"MSFT","response":[{"meta":{"symbol":"MSFT","gmtoffset":-14400,"currency":"USD","regularMarketPrice":410.0},"timestamp":[1787319000],"indicators":{"quote":[{"close":[410.0]}]}}]}
    ],"error":null}}"#;

    fn sym(raw: &str) -> Symbol {
        match parse(raw) {
            Parsed::Resolved(s) => s,
            other => panic!("{raw} -> {other:?}"),
        }
    }

    #[test]
    fn null_bars_are_dropped_not_zero_filled() {
        let series = parse_chart(CHART_AAPL).unwrap();
        assert_eq!(series.bars.len(), 3);
        assert_eq!(series.bars[0].close, Some(228.0));
        // The null close must be None, never Some(0.0).
        assert_eq!(series.bars[1].close, None);
        assert_eq!(series.bars[2].close, Some(230.5));
        assert_eq!(series.gmtoffset, -14400);
        assert_eq!(series.regular_end, 1787342400);
        assert_eq!(series.display_name, "Apple Inc.");
        assert_eq!(series.prev_close, Some(228.0));
    }

    #[test]
    fn not_found_envelope_classifies_as_notfound() {
        assert_eq!(parse_chart(CHART_NOT_FOUND), Err(SourceError::NotFound));
    }

    #[test]
    fn mismatched_array_lengths_are_handled_not_a_panic() {
        // close array shorter than timestamp; the missing tail must be None.
        let body = r#"{"chart":{"result":[{"meta":{"gmtoffset":0},"timestamp":[1,2,3],"indicators":{"quote":[{"close":[10.0]}]}}],"error":null}}"#;
        let series = parse_chart(body).unwrap();
        assert_eq!(series.bars.len(), 3);
        assert_eq!(series.bars[0].close, Some(10.0));
        assert_eq!(series.bars[1].close, None);
        assert_eq!(series.bars[2].close, None);
    }

    #[test]
    fn spark_results_are_indexed_by_symbol_not_position() {
        let out = parse_spark(SPARK_UNORDERED).unwrap();
        let map: std::collections::HashMap<_, _> = out.into_iter().collect();
        // If we indexed by position, MSFT would carry 2330.TW's price.
        assert_eq!(map["MSFT"].last_price, Some(410.0));
        assert_eq!(map["2330.TW"].last_price, Some(2410.0));
        assert_eq!(map["2330.TW"].gmtoffset, 28800);
    }

    #[test]
    fn spark_silently_missing_symbols_are_reported_as_missing() {
        let requested = ["AAPL", "MSFT", "2330.TW", "ZZZZNOTREAL"];
        let returned: std::collections::HashSet<String> =
            parse_spark(SPARK_UNORDERED).unwrap().into_iter().map(|(s, _)| s).collect();
        let missing: Vec<&str> = requested
            .iter()
            .copied()
            .filter(|s| !returned.contains(*s))
            .collect();
        assert_eq!(missing, vec!["ZZZZNOTREAL"]);
    }

    #[test]
    fn garbage_body_is_malformed_not_a_panic() {
        assert!(matches!(parse_chart("not json"), Err(SourceError::Malformed(_))));
    }

    #[tokio::test]
    async fn series_over_http_parses_the_chart() {
        let base = spawn_scripted_server(vec![(200, CHART_AAPL)]).await;
        let src = YahooSource::new(Client::new(), base);
        let series = src.series(&sym("AAPL"), 180).await.unwrap();
        assert_eq!(series.bars.len(), 3);
        assert_eq!(series.currency, "USD");
    }

    #[tokio::test]
    async fn a_404_becomes_not_found() {
        let base = spawn_scripted_server(vec![(404, CHART_NOT_FOUND)]).await;
        let src = YahooSource::new(Client::new(), base);
        let err = src.series(&sym("9999.TW"), 180).await.unwrap_err();
        assert_eq!(super::super::classify_source_error(&err), SourceError::NotFound);
    }

    #[tokio::test]
    async fn spark_batch_over_http() {
        let base = spawn_scripted_server(vec![(200, SPARK_UNORDERED)]).await;
        let src = YahooSource::new(Client::new(), base);
        let out = src.spark_batch(&["AAPL", "MSFT", "2330.TW"]).await.unwrap();
        assert_eq!(out.len(), 3);
    }
}
