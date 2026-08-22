//! Pure rendering for the stock feature: quote card, watchlist page, and the
//! daily close report. **No `Bot`, no async, no `Repo`.** Every symbol/company
//! field flows through `teloxide::utils::html::escape` before landing in an
//! HTML message — an unescaped `&`/`<` makes Telegram reject the whole message
//! and the push is silently lost (the failure mode `bot/render.rs` documents).
//!
//! The daily report is returned as a `Vec<String>` of individual lines so the
//! caller can feed it to `bot::render::chunk_lines` without ever splitting a
//! line mid-content.

use teloxide::utils::html::escape;

use crate::bot::i18n::Lang;

use super::indicators::{Snapshot, MIN_BARS_FOR_INDICATORS};
use super::service::{QuoteView, WatchlistPage};
use super::signals::Signal;
use super::symbol::{Board, Market};

const DASH: &str = "—";

fn board_label(board: Board) -> &'static str {
    match board {
        Board::Twse => "TWSE",
        Board::Tpex => "TPEx",
        Board::Us => "US",
    }
}

/// Groups an integer digit string with thousands commas: `"17158844"` ->
/// `"17,158,844"`. Operates on the already-formatted integer part.
fn group(digits: &str) -> String {
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Price with 2 decimals and thousands grouping, or an em dash for `None`.
fn price(v: Option<f64>) -> String {
    match v {
        Some(v) if v.is_finite() => {
            let s = format!("{:.2}", v.abs());
            let (int, frac) = s.split_once('.').unwrap_or((s.as_str(), "00"));
            format!("{}{}.{frac}", if v < 0.0 { "-" } else { "" }, group(int))
        }
        _ => DASH.to_owned(),
    }
}

/// One decimal, no grouping (KD/RSI), or an em dash.
fn one_dp(v: Option<f64>) -> String {
    match v {
        Some(v) if v.is_finite() => format!("{v:.1}"),
        _ => DASH.to_owned(),
    }
}

/// Signed 2-decimal with grouping (change, MACD), or an em dash.
fn signed(v: Option<f64>) -> String {
    match v {
        Some(v) if v.is_finite() => {
            let s = format!("{:.2}", v.abs());
            let (int, frac) = s.split_once('.').unwrap_or((s.as_str(), "00"));
            format!("{}{}.{frac}", if v < 0.0 { "-" } else { "+" }, group(int))
        }
        _ => DASH.to_owned(),
    }
}

fn volume(v: Option<i64>) -> String {
    match v {
        Some(v) => group(&v.abs().to_string()),
        None => DASH.to_owned(),
    }
}

/// The `▲10.00 (+0.42%)` change fragment from a price and previous close.
fn change_fragment(last: Option<f64>, prev: Option<f64>) -> String {
    let (Some(last), Some(prev)) = (last, prev) else {
        return String::new();
    };
    let diff = last - prev;
    let arrow = if diff > 0.0 {
        "▲"
    } else if diff < 0.0 {
        "▼"
    } else {
        "・"
    };
    let pct = if prev != 0.0 {
        format!(" ({}{:.2}%)", if diff >= 0.0 { "+" } else { "-" }, (diff / prev * 100.0).abs())
    } else {
        String::new()
    };
    format!("{arrow}{}{pct}", signed(Some(diff)))
}

/// The three indicator lines (MA / KD·RSI·MACD / BOLL·52w), or a single
/// "insufficient history" line when there aren't enough bars.
fn indicator_lines(
    snap: &Snapshot,
    week52_high: Option<f64>,
    week52_low: Option<f64>,
    lang: Lang,
) -> Vec<String> {
    if snap.bars_used < MIN_BARS_FOR_INDICATORS {
        return vec![lang.stk_insufficient().to_owned()];
    }
    vec![
        format!(
            "MA5 {} / MA20 {} / MA60 {}",
            price(snap.ma5),
            price(snap.ma20),
            price(snap.ma60)
        ),
        format!(
            "KD {} / {} · RSI14 {} · MACD {} ({} {})",
            one_dp(snap.k),
            one_dp(snap.d),
            one_dp(snap.rsi14),
            signed(snap.macd),
            lang.stk_macd_hist(),
            signed(snap.macd_hist)
        ),
        format!(
            "BOLL {} – {} · {} {} – {}",
            price(snap.boll_lower),
            price(snap.boll_upper),
            lang.stk_week52(),
            price(week52_low),
            price(week52_high)
        ),
    ]
}

fn signal_lines(signals: &[Signal], lang: Lang) -> Vec<String> {
    signals.iter().map(|s| lang.stk_signal(*s).to_owned()).collect()
}

/// The `/stock <symbol>` quote card (single HTML message).
pub fn render_quote_card(view: &QuoteView, lang: Lang) -> String {
    let sym = &view.symbol;
    let (display_name, currency, last_price, prev_close, w52h, w52l) = match &view.meta {
        Some(m) => (
            m.display_name.as_str(),
            m.currency.as_str(),
            m.last_price.or(view.snapshot.last_close),
            m.prev_close.or(view.snapshot.prev_close),
            m.week52_high,
            m.week52_low,
        ),
        None => (
            "",
            "",
            view.snapshot.last_close,
            view.snapshot.prev_close,
            None,
            None,
        ),
    };
    let name = if display_name.is_empty() {
        sym.local_code.as_str()
    } else {
        display_name
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "📈 <b>{} {}</b> ({} · {})",
        escape(&sym.local_code),
        escape(name),
        board_label(sym.board),
        escape(currency)
    ));
    lines.push(format!(
        "{}  {}",
        price(last_price),
        change_fragment(last_price, prev_close)
    ));
    lines.push(format!("{} {}", lang.stk_vol(), volume(view.snapshot.last_volume)));
    lines.extend(indicator_lines(&view.snapshot, w52h, w52l, lang));
    lines.extend(signal_lines(&view.signals, lang));
    if view.stale {
        lines.push(lang.stk_stale().to_owned());
    }
    lines.join("\n")
}

/// The `/stocks` watchlist page (single HTML message). The list is at most one
/// page (`watchlist_page_size`) of rows, so no chunking is needed.
pub fn render_watchlist(page: &WatchlistPage, lang: Lang) -> String {
    if page.total == 0 {
        return lang.stk_empty().to_owned();
    }
    let pages = page.total.div_ceil(page.per_page).max(1);
    let mut lines = vec![lang.stk_list_header(page.scope, page.total, page.page_index + 1, pages)];
    for item in &page.items {
        let name = if item.display_name.is_empty() {
            item.symbol.as_str()
        } else {
            item.display_name.as_str()
        };
        let note = if item.note.is_empty() {
            String::new()
        } else {
            format!(" — {}", escape(&item.note))
        };
        // The id is shown so /stockdel <id> is discoverable.
        lines.push(format!(
            "<code>{}</code> {} · {}{}",
            item.id,
            escape(&item.symbol),
            escape(name),
            note
        ));
    }
    lines.join("\n")
}

/// One symbol's computed data for the daily report. Assembled by the worker
/// from cached bars + meta; kept plain so rendering stays pure and testable.
#[derive(Debug, Clone)]
pub struct ReportEntry {
    /// Canonical symbol (`2330.TW`) — used as the commentary cache key.
    pub canonical: String,
    pub local_code: String,
    pub display_name: String,
    pub snapshot: Snapshot,
    pub signals: Vec<Signal>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    /// True when only a single fallback close is available (TW official dump):
    /// the indicator block is suppressed and a note substituted, because a
    /// 20-day MA computed from 19 days and a hole is the worst kind of wrong —
    /// it looks right.
    pub indicators_unavailable: bool,
    /// Optional AI commentary (already length-capped, not yet HTML-escaped —
    /// escaping happens here at emit time). The remote agent is untrusted text.
    pub commentary: Option<String>,
}

fn push_entry_lines(lines: &mut Vec<String>, e: &ReportEntry, lang: Lang) {
    let name = if e.display_name.is_empty() {
        e.local_code.as_str()
    } else {
        e.display_name.as_str()
    };
    lines.push(format!(
        "{} {}  {}  {}",
        escape(&e.local_code),
        escape(name),
        price(e.snapshot.last_close),
        change_fragment(e.snapshot.last_close, e.snapshot.prev_close)
    ));
    if e.indicators_unavailable {
        lines.push(format!(
            "{} {} · {}",
            lang.stk_vol(),
            volume(e.snapshot.last_volume),
            lang.stk_indicators_unavailable()
        ));
    } else {
        lines.push(format!(
            "{} {} · MA5 {} / MA20 {} / MA60 {}",
            lang.stk_vol(),
            volume(e.snapshot.last_volume),
            price(e.snapshot.ma5),
            price(e.snapshot.ma20),
            price(e.snapshot.ma60)
        ));
        if e.snapshot.bars_used >= MIN_BARS_FOR_INDICATORS {
            lines.push(format!(
                "KD {} / {} · RSI14 {} · MACD {} ({} {})",
                one_dp(e.snapshot.k),
                one_dp(e.snapshot.d),
                one_dp(e.snapshot.rsi14),
                signed(e.snapshot.macd),
                lang.stk_macd_hist(),
                signed(e.snapshot.macd_hist)
            ));
            lines.push(format!(
                "BOLL {} – {} · {} {} – {}",
                price(e.snapshot.boll_lower),
                price(e.snapshot.boll_upper),
                lang.stk_week52(),
                price(e.week52_low),
                price(e.week52_high)
            ));
        }
    }
    for line in signal_lines(&e.signals, lang) {
        lines.push(line);
    }
    if let Some(commentary) = &e.commentary {
        // Untrusted remote-agent text: escape before it enters an HTML message.
        lines.push(format!("🤖 {}", escape(commentary)));
    }
}

/// The daily close report as individual lines (feed to `chunk_lines`). Never
/// truncates a line; the caller caps the number of chunks and appends an
/// overflow note.
pub fn render_daily_report(
    market: Market,
    trade_date: &str,
    entries: &[ReportEntry],
    late: bool,
    lang: Lang,
) -> Vec<String> {
    let mut header = lang.stk_report_header(market, trade_date, entries.len());
    if late {
        header.push(' ');
        header.push_str(lang.stk_delayed());
    }
    let mut lines = vec![header, String::new()];
    for e in entries {
        push_entry_lines(&mut lines, e, lang);
    }
    if entries.iter().any(|e| e.commentary.is_some()) {
        lines.push(String::new());
        lines.push(lang.stk_ai_disclaimer().to_owned());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::render::chunk_lines;
    use crate::db::models::WatchItem;
    use crate::stock::symbol::{parse, Parsed, Symbol};
    use crate::stock::MarketScope;

    fn sym(raw: &str) -> Symbol {
        match parse(raw) {
            Parsed::Resolved(s) => s,
            other => panic!("{raw} -> {other:?}"),
        }
    }

    fn snap(bars_used: usize) -> Snapshot {
        Snapshot {
            bars_used,
            last_close: Some(2410.0),
            prev_close: Some(2400.0),
            last_volume: Some(17_158_844),
            ma5: Some(2395.0),
            ma20: Some(2340.0),
            ma60: Some(2180.0),
            rsi14: Some(61.3),
            macd: Some(12.4),
            macd_signal: Some(9.3),
            macd_hist: Some(3.1),
            k: Some(82.1),
            d: Some(75.4),
            boll_mid: Some(2325.0),
            boll_upper: Some(2470.0),
            boll_lower: Some(2180.0),
        }
    }

    #[test]
    fn number_helpers_group_and_dash() {
        assert_eq!(price(Some(2410.0)), "2,410.00");
        assert_eq!(price(Some(1135.5)), "1,135.50");
        assert_eq!(price(None), "—");
        assert_eq!(signed(Some(10.0)), "+10.00");
        assert_eq!(signed(Some(-52.0)), "-52.00");
        assert_eq!(volume(Some(17_158_844)), "17,158,844");
        assert_eq!(one_dp(Some(82.14)), "82.1");
    }

    #[test]
    fn missing_indicators_render_an_em_dash_not_zero() {
        let mut s = snap(60);
        s.ma60 = None;
        let card = render_quote_card(
            &QuoteView {
                symbol: sym("2330.TW"),
                meta: None,
                snapshot: s,
                signals: vec![],
                stale: false,
            },
            Lang::ZhTw,
        );
        assert!(card.contains("MA60 —"), "None indicator must be an em dash, got: {card}");
        assert!(!card.contains("MA60 0"));
    }

    #[test]
    fn company_names_are_html_escaped() {
        let entry = ReportEntry {
            canonical: "2330.TW".into(),
            local_code: "2330".into(),
            display_name: "A & B <script>".into(),
            snapshot: snap(60),
            signals: vec![Signal::KdGoldenCross],
            week52_high: Some(2535.0),
            week52_low: Some(1135.0),
            indicators_unavailable: false,
            commentary: Some("buy <b>now</b> & profit".into()),
        };
        let lines = render_daily_report(Market::Tw, "2026-08-21", &[entry], false, Lang::ZhTw);
        let text = lines.join("\n");
        assert!(text.contains("A &amp; B &lt;script&gt;"));
        assert!(!text.contains("<script>"));
        assert!(text.contains("⭐ KD 黃金交叉"));
        // Untrusted agent commentary is escaped too.
        assert!(text.contains("buy &lt;b&gt;now&lt;/b&gt; &amp; profit"));
        assert!(text.contains(Lang::ZhTw.stk_ai_disclaimer()));
    }

    #[test]
    fn report_lines_never_exceed_the_chunk_limit_when_chunked() {
        let entries: Vec<ReportEntry> = (0..30)
            .map(|i| ReportEntry {
                canonical: format!("{:04}.TW", 2000 + i),
                local_code: format!("{:04}", 2000 + i),
                display_name: "測試公司".into(),
                snapshot: snap(60),
                signals: vec![Signal::KdGoldenCross, Signal::MacdDeadCross],
                week52_high: Some(2535.0),
                week52_low: Some(1135.0),
                indicators_unavailable: false,
                commentary: None,
            })
            .collect();
        let lines = render_daily_report(Market::Tw, "2026-08-21", &entries, true, Lang::ZhTw);
        for chunk in chunk_lines(&lines, 3500) {
            assert!(chunk.chars().count() <= 3500);
        }
    }

    #[test]
    fn fallback_without_history_omits_indicators_entirely() {
        let entry = ReportEntry {
            canonical: "2330.TW".into(),
            local_code: "2330".into(),
            display_name: "台積電".into(),
            snapshot: snap(1), // single fallback bar
            signals: vec![],
            week52_high: None,
            week52_low: None,
            indicators_unavailable: true,
            commentary: None,
        };
        let text = render_daily_report(Market::Tw, "2026-08-21", &[entry], false, Lang::ZhTw).join("\n");
        assert!(text.contains(Lang::ZhTw.stk_indicators_unavailable()));
        assert!(!text.contains("MA5"), "no indicator line when unavailable");
    }

    #[test]
    fn empty_watchlist_renders_the_empty_hint() {
        let page = WatchlistPage {
            items: vec![],
            total: 0,
            page_index: 0,
            per_page: 8,
            scope: MarketScope::All,
        };
        assert_eq!(render_watchlist(&page, Lang::ZhTw), Lang::ZhTw.stk_empty());
    }

    #[test]
    fn watchlist_lists_items_with_ids() {
        let item = WatchItem {
            id: 42,
            chat_id: 1,
            created_by: 1,
            symbol: "2330.TW".into(),
            market: "tw".into(),
            exchange: "TAI".into(),
            display_name: "台積電".into(),
            currency: "TWD".into(),
            note: "core holding".into(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        };
        let page = WatchlistPage {
            items: vec![item],
            total: 1,
            page_index: 0,
            per_page: 8,
            scope: MarketScope::Tw,
        };
        let text = render_watchlist(&page, Lang::ZhTw);
        assert!(text.contains("<code>42</code>"));
        assert!(text.contains("2330.TW"));
        assert!(text.contains("台積電"));
        assert!(text.contains("core holding"));
    }
}
