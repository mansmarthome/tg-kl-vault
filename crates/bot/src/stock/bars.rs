//! OHLCV bar / quote / series types and the `bar_is_sane` hygiene gate.
//! **Pure** — no network, no DB.
//!
//! Garbage in the cache is worse than no report at all, because it silently
//! poisons every downstream moving average and RSI. [`bar_is_sane`] is the gate
//! that every parsed bar must pass before it is allowed near the indicators.

use super::clock::SessionMeta;

/// One daily OHLCV bar. Every price field is `Option` because Yahoo delivers
/// OHLCV as *parallel arrays* that may independently carry `null` at any index.
/// `close` being `None` is the one fatal case (a bar with no close is dropped
/// entirely) — the others degrade gracefully.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    /// Bar start epoch (seconds).
    pub ts: i64,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<i64>,
}

/// A point-in-time quote for the quote card. Kept separate from `Bar` because
/// it carries 52-week extremes and the previous close, which come from `meta`,
/// not from any single bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub symbol: String,
    pub display_name: String,
    pub currency: String,
    pub last_price: Option<f64>,
    pub prev_close: Option<f64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    /// `meta.regularMarketTime`.
    pub market_time: i64,
}

impl Quote {
    pub fn change(&self) -> Option<f64> {
        Some(self.last_price? - self.prev_close?)
    }

    pub fn change_pct(&self) -> Option<f64> {
        let prev = self.prev_close?;
        if prev == 0.0 {
            None
        } else {
            Some((self.last_price? - prev) / prev * 100.0)
        }
    }
}

/// A parsed Yahoo chart result: the bars plus the meta fields needed to build a
/// [`SessionMeta`] and a [`Quote`]. Produced by the source layer; pure data.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub bars: Vec<Bar>,
    pub gmtoffset: i64,
    pub regular_start: i64,
    pub regular_end: i64,
    pub market_time: i64,
    pub last_price: Option<f64>,
    pub prev_close: Option<f64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    pub exchange: String,
    pub display_name: String,
    pub currency: String,
}

impl Series {
    /// Derives the session clock inputs from the raw (unsanitized) last bar —
    /// the live intraday bar is intentionally included, because
    /// `classify_session` needs it to distinguish "still open" from "closed".
    pub fn session_meta(&self) -> SessionMeta {
        let last = self.bars.last();
        SessionMeta {
            gmtoffset: self.gmtoffset,
            regular_start: self.regular_start,
            regular_end: self.regular_end,
            last_bar_start: last.map_or(0, |b| b.ts),
            last_bar_has_close: last.is_some_and(|b| b.close.is_some()),
            last_bar_volume: last.and_then(|b| b.volume).unwrap_or(0),
        }
    }
}

/// The hygiene gate. `earliest_ts` is a lower bound on plausible bar epochs and
/// `now` an upper bound (+2 days of slack for timezone skew). A bar failing any
/// check is dropped before it can pollute the cache.
pub fn bar_is_sane(b: &Bar, earliest_ts: i64, now: i64) -> bool {
    let Some(c) = b.close else {
        return false;
    };
    c.is_finite()
        && c > 0.0
        && b.high.zip(b.low).is_none_or(|(h, l)| h >= l && h.is_finite() && l.is_finite())
        && b.high.is_none_or(|h| h >= c)
        && b.low.is_none_or(|l| l <= c)
        && b.volume.is_none_or(|v| v >= 0)
        && b.ts >= earliest_ts
        && b.ts <= now + 2 * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(close: Option<f64>) -> Bar {
        Bar {
            ts: 1_000_000,
            open: Some(10.0),
            high: Some(11.0),
            low: Some(9.0),
            close,
            volume: Some(100),
        }
    }

    #[test]
    fn a_bar_without_a_close_is_never_sane() {
        assert!(!bar_is_sane(&bar(None), 0, 2_000_000));
    }

    #[test]
    fn a_normal_bar_is_sane() {
        assert!(bar_is_sane(&bar(Some(10.0)), 0, 2_000_000));
    }

    #[test]
    fn nan_infinite_and_nonpositive_closes_are_rejected() {
        assert!(!bar_is_sane(&bar(Some(f64::NAN)), 0, 2_000_000));
        assert!(!bar_is_sane(&bar(Some(f64::INFINITY)), 0, 2_000_000));
        assert!(!bar_is_sane(&bar(Some(0.0)), 0, 2_000_000));
        assert!(!bar_is_sane(&bar(Some(-1.0)), 0, 2_000_000));
    }

    #[test]
    fn high_below_low_or_below_close_is_rejected() {
        let mut b = bar(Some(10.0));
        b.high = Some(8.0); // below low(9) and below close(10)
        assert!(!bar_is_sane(&b, 0, 2_000_000));
    }

    #[test]
    fn missing_high_low_volume_still_passes_on_a_good_close() {
        let b = Bar {
            ts: 1_000_000,
            open: None,
            high: None,
            low: None,
            close: Some(10.0),
            volume: None,
        };
        assert!(bar_is_sane(&b, 0, 2_000_000));
    }

    #[test]
    fn out_of_range_timestamps_are_rejected() {
        assert!(!bar_is_sane(&bar(Some(10.0)), 2_000_000, 3_000_000)); // ts before earliest
        assert!(!bar_is_sane(&bar(Some(10.0)), 0, 100)); // ts far in the future
    }

    #[test]
    fn quote_change_needs_both_prices() {
        let mut q = Quote {
            symbol: "AAPL".into(),
            display_name: "Apple".into(),
            currency: "USD".into(),
            last_price: Some(110.0),
            prev_close: Some(100.0),
            week52_high: None,
            week52_low: None,
            market_time: 0,
        };
        assert_eq!(q.change(), Some(10.0));
        assert_eq!(q.change_pct(), Some(10.0));
        q.prev_close = None;
        assert_eq!(q.change(), None);
        assert_eq!(q.change_pct(), None);
    }

    #[test]
    fn session_meta_reads_the_last_bar() {
        let series = Series {
            bars: vec![bar(Some(10.0)), Bar { ts: 2_000_000, ..bar(Some(11.0)) }],
            gmtoffset: 28_800,
            regular_start: 1,
            regular_end: 2,
            market_time: 3,
            last_price: Some(11.0),
            prev_close: Some(10.0),
            week52_high: None,
            week52_low: None,
            exchange: "TAI".into(),
            display_name: "x".into(),
            currency: "TWD".into(),
        };
        let m = series.session_meta();
        assert_eq!(m.last_bar_start, 2_000_000);
        assert!(m.last_bar_has_close);
        assert_eq!(m.last_bar_volume, 100);
    }
}
