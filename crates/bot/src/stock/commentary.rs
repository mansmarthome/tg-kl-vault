//! AI commentary over the existing MCP bridge (`tagging::mcp::McpClient`) — no
//! new AI transport, and `tagging/gemini.rs` is untouched.
//!
//! The bottleneck here is *latency*, not cost (`McpConfig::timeout_seconds`
//! defaults to 240s), which shapes two things: the daily report sends **one**
//! batch prompt for all signal-bearing symbols (N symbols = 1 call, not N), and
//! the report path wraps that call in a shorter `ai_report_timeout_seconds` so a
//! slow agent turn can never stall the 60s worker tick — timeout ⇒ send the
//! numbers without commentary. Commentary is never on the critical path.
//!
//! Cost/traffic controls, in order: off by default → only symbols with a signal
//! enter the prompt (a flat day is zero calls) → capped at `ai_max_symbols` →
//! `stock_commentary` cache (50 chats on one symbol pay once) → a per-day quota
//! metered separately from bookmark tagging so a chatty watchlist can't starve
//! it.

use std::collections::HashMap;
use std::time::Duration;

use tracing::warn;

use crate::config::StockConfig;
use crate::db::repo::Repo;
use crate::tagging::mcp::McpClient;
use crate::tagging::quota::try_consume_key;

use crate::bot::i18n::Lang;

use super::render::ReportEntry;
use super::signals::Signal;

/// Separate daily budget from `tg-kl-vault:ai:quota` (bookmark tagging).
pub const STOCK_QUOTA_KEY: &str = "tg-kl-vault:stock:ai:quota";

const MAX_COMMENTARY_CHARS: usize = 1200;

/// One symbol's brief for a prompt. Pure input; no DB/network types.
pub struct SymbolBrief<'a> {
    pub code: &'a str,
    pub name: &'a str,
    pub close: Option<f64>,
    pub change_pct: Option<f64>,
    pub signals: &'a [Signal],
}

fn change_pct(entry: &ReportEntry) -> Option<f64> {
    let (last, prev) = (entry.snapshot.last_close?, entry.snapshot.prev_close?);
    if prev == 0.0 {
        None
    } else {
        Some((last - prev) / prev * 100.0)
    }
}

/// Indices of entries eligible for commentary: those with at least one signal
/// (a flat day yields none → zero AI calls), most-moved first, capped at `max`.
pub fn select_candidates(entries: &[ReportEntry], max: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.signals.is_empty())
        .map(|(i, _)| i)
        .collect();
    idx.sort_by(|&a, &b| {
        let pa = change_pct(&entries[a]).unwrap_or(0.0).abs();
        let pb = change_pct(&entries[b]).unwrap_or(0.0).abs();
        pb.total_cmp(&pa)
    });
    idx.truncate(max);
    idx
}

fn lang_name(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhTw => "Traditional Chinese (zh-TW)",
        Lang::En => "English",
    }
}

fn signals_phrase(signals: &[Signal], lang: Lang) -> String {
    signals.iter().map(|s| lang.stk_signal(*s)).collect::<Vec<_>>().join(", ")
}

fn brief_line(b: &SymbolBrief) -> String {
    let close = b.close.map(|c| format!("{c:.2}")).unwrap_or_else(|| "?".into());
    let pct = b.change_pct.map(|p| format!("{p:+.2}%")).unwrap_or_default();
    format!("{} {} close {close} {pct}", b.code, b.name)
}

/// Batch prompt: one line of commentary per symbol, in a strict
/// `CODE: comment` format so the reply can be split back per symbol.
pub fn build_report_prompt(briefs: &[SymbolBrief], lang: Lang) -> String {
    let mut out = format!(
        "You are a concise financial assistant. For each stock below, write ONE short \
         plain-language sentence (≤40 words) in {}, focused on today's move and the noted \
         technical signals. Output EXACTLY one line per stock in the format `CODE: comment`, \
         nothing else — no preamble, no markdown, no code fences.\n\n",
        lang_name(lang)
    );
    for b in briefs {
        out.push_str(&brief_line(b));
        let sig = signals_phrase(b.signals, lang);
        if !sig.is_empty() {
            out.push_str(" — signals: ");
            out.push_str(&sig);
        }
        out.push('\n');
    }
    out
}

/// Single-symbol prompt for the interactive 🤖 button.
pub fn build_single_prompt(b: &SymbolBrief, lang: Lang) -> String {
    let sig = signals_phrase(b.signals, lang);
    let sig = if sig.is_empty() { "none".to_owned() } else { sig };
    format!(
        "You are a concise financial assistant. In {}, write 2–3 short plain-language \
         sentences about this stock's latest move and technical picture. No markdown, no \
         code fences, no preamble.\n\n{}\nSignals: {sig}",
        lang_name(lang),
        brief_line(b),
    )
}

/// Splits a batch reply (`CODE: comment` lines) back into a per-code map, keyed
/// by the codes we actually asked about (so stray lines are ignored).
pub fn split_batch_commentary(text: &str, codes: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let Some((code, comment)) = line.split_once(':') else {
            continue;
        };
        let code = code.trim();
        let comment = comment.trim();
        if comment.is_empty() {
            continue;
        }
        if let Some(matched) = codes.iter().find(|c| c.as_str() == code) {
            out.entry(matched.clone()).or_insert_with(|| comment.to_owned());
        }
    }
    out
}

/// Trims and length-caps commentary. Does **not** HTML-escape — escaping happens
/// at render/emit time, so the cache stores reusable plain text.
pub fn sanitize(text: &str) -> String {
    text.trim().chars().take(MAX_COMMENTARY_CHARS).collect()
}

/// Attaches AI commentary to the signal-bearing entries of a report, in place.
/// Cache-first, then one batch MCP call (bounded by `ai_report_timeout_seconds`)
/// for the misses. Any failure/timeout leaves the numbers untouched — commentary
/// is strictly optional.
pub async fn annotate_entries(
    client: &McpClient,
    repo: &Repo,
    lang: Lang,
    trade_date: &str,
    entries: &mut [ReportEntry],
    cfg: &StockConfig,
) -> anyhow::Result<()> {
    let candidates = select_candidates(entries, cfg.ai_max_symbols as usize);
    if candidates.is_empty() {
        return Ok(());
    }

    // Cache pass (free); collect the misses.
    let mut misses = Vec::new();
    for &i in &candidates {
        let canonical = entries[i].canonical.clone();
        if let Some(body) = repo.get_commentary(&canonical, trade_date, lang.value()).await? {
            entries[i].commentary = Some(body);
        } else {
            misses.push(i);
        }
    }
    if misses.is_empty() {
        return Ok(());
    }

    // Quota is checked only after the cache, so a cache hit is always free.
    if !try_consume_key(repo, STOCK_QUOTA_KEY, cfg.ai_daily_quota).await? {
        return Ok(());
    }

    let briefs: Vec<SymbolBrief> = misses.iter().map(|&i| brief(&entries[i])).collect();
    let codes: Vec<String> = misses.iter().map(|&i| entries[i].local_code.clone()).collect();
    let prompt = build_report_prompt(&briefs, lang);

    let timeout = Duration::from_secs(cfg.ai_report_timeout_seconds.max(1));
    let text = match tokio::time::timeout(timeout, client.run(&prompt)).await {
        Ok(Ok(text)) if !text.trim().is_empty() => text,
        Ok(Ok(_)) => return Ok(()),
        Ok(Err(err)) => {
            warn!(error = %err, "stock commentary batch failed");
            return Ok(());
        }
        Err(_) => {
            warn!("stock commentary timed out; sending numbers only");
            return Ok(());
        }
    };

    let parsed = split_batch_commentary(&text, &codes);
    let mut used_fallback = false;
    for &i in &misses {
        let code = entries[i].local_code.clone();
        let body = match parsed.get(&code) {
            Some(b) => sanitize(b),
            None if !used_fallback => {
                // Never discard a paid turn: if the agent didn't follow the
                // per-line format, keep the whole reply as an overall note on
                // the first miss rather than throwing it away.
                used_fallback = true;
                sanitize(&text)
            }
            None => continue,
        };
        entries[i].commentary = Some(body.clone());
        repo.put_commentary(&entries[i].canonical, trade_date, lang.value(), &body, "mcp")
            .await?;
    }
    Ok(())
}

fn brief(entry: &ReportEntry) -> SymbolBrief<'_> {
    SymbolBrief {
        code: &entry.local_code,
        name: &entry.display_name,
        close: entry.snapshot.last_close,
        change_pct: change_pct(entry),
        signals: &entry.signals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stock::indicators::Snapshot;

    fn entry(code: &str, signals: Vec<Signal>, last: f64, prev: f64) -> ReportEntry {
        ReportEntry {
            canonical: format!("{code}.TW"),
            local_code: code.to_owned(),
            display_name: format!("{code} Co"),
            snapshot: Snapshot {
                last_close: Some(last),
                prev_close: Some(prev),
                ..Snapshot::default()
            },
            signals,
            week52_high: None,
            week52_low: None,
            indicators_unavailable: false,
            commentary: None,
        }
    }

    #[test]
    fn only_symbols_with_signals_enter_the_prompt() {
        let entries = vec![
            entry("2330", vec![Signal::KdGoldenCross], 110.0, 100.0),
            entry("2317", vec![], 100.0, 100.0), // flat, no signals
        ];
        let idx = select_candidates(&entries, 10);
        assert_eq!(idx, vec![0], "the flat/no-signal symbol is excluded");
    }

    #[test]
    fn candidates_are_capped_at_ai_max_symbols() {
        let entries: Vec<ReportEntry> = (0..5)
            .map(|i| entry(&format!("{i}"), vec![Signal::KdGoldenCross], 100.0 + i as f64 * 5.0, 100.0))
            .collect();
        let idx = select_candidates(&entries, 3);
        assert_eq!(idx.len(), 3);
        // Sorted by |change%| desc: symbol "4" (+20%) first.
        assert_eq!(entries[idx[0]].local_code, "4");
    }

    #[test]
    fn report_prompt_contains_every_symbol_its_move_and_the_language() {
        let e0 = entry("2330", vec![Signal::KdGoldenCross], 110.0, 100.0);
        let e1 = entry("6488", vec![Signal::MacdDeadCross], 90.0, 100.0);
        let briefs = vec![brief(&e0), brief(&e1)];
        let prompt = build_report_prompt(&briefs, Lang::ZhTw);
        assert!(prompt.contains("2330"));
        assert!(prompt.contains("6488"));
        assert!(prompt.contains("+10.00%"));
        assert!(prompt.contains("Traditional Chinese"));
        assert!(prompt.contains("CODE: comment"));
    }

    #[test]
    fn batch_commentary_splits_into_per_symbol_rows() {
        let text = "2330: strong close on a KD golden cross\n6488: weak, MACD rolled over\nnoise line";
        let codes = vec!["2330".to_owned(), "6488".to_owned()];
        let map = split_batch_commentary(text, &codes);
        assert_eq!(map["2330"], "strong close on a KD golden cross");
        assert_eq!(map["6488"], "weak, MACD rolled over");
        assert_eq!(map.len(), 2, "stray lines and unknown codes are ignored");
    }

    #[test]
    fn unparseable_output_yields_no_per_symbol_rows() {
        // A reply that ignores the format produces an empty split; the caller
        // keeps it as a single overall note rather than discarding it.
        let map = split_batch_commentary("Here is my analysis of the market today.", &["2330".to_owned()]);
        assert!(map.is_empty());
    }

    #[test]
    fn sanitize_caps_length_and_trims() {
        assert_eq!(sanitize("  hi  "), "hi");
        assert_eq!(sanitize(&"x".repeat(5000)).chars().count(), MAX_COMMENTARY_CHARS);
    }
}
