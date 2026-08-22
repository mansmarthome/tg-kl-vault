//! Cross / threshold detection. **Pure** — no network, no DB.
//!
//! This is also the gate for AI commentary: a flat day produces zero signals,
//! and zero signals means zero AI calls. The classic bug here is inventing a
//! "cross" out of a `None → Some` warm-up transition; [`cross`] refuses to look
//! at a pair unless *both* the current and previous points are defined.

use super::bars::Bar;
use super::indicators::{bollinger, kd_taiwan, macd, rsi, series_arrays, sma};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    MaGoldenCross,
    MaDeadCross,
    KdGoldenCross,
    KdDeadCross,
    MacdGoldenCross,
    MacdDeadCross,
    BollBreakUpper,
    BollBreakLower,
    RsiOverbought,
    RsiOversold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cross {
    Golden,
    Dead,
}

/// Did `a` cross `b` at the final index? Returns `None` unless the last *two*
/// points of both series are defined — this is what prevents a warm-up
/// `None → Some` transition from being reported as a cross.
fn cross(a: &[Option<f64>], b: &[Option<f64>]) -> Option<Cross> {
    let n = a.len().min(b.len());
    if n < 2 {
        return None;
    }
    let (a1, b1) = (a[n - 1]?, b[n - 1]?);
    let (a0, b0) = (a[n - 2]?, b[n - 2]?);
    if a0 <= b0 && a1 > b1 {
        Some(Cross::Golden)
    } else if a0 >= b0 && a1 < b1 {
        Some(Cross::Dead)
    } else {
        None
    }
}

/// The last two defined values of a series, or `None` if either is missing.
fn last_two(v: &[Option<f64>]) -> Option<(f64, f64)> {
    let n = v.len();
    if n < 2 {
        return None;
    }
    Some((v[n - 2]?, v[n - 1]?))
}

/// Detects the signals present at the most recent bar. Empty when history is
/// too short — never a false positive from the warm-up edge.
pub fn detect(bars: &[Bar]) -> Vec<Signal> {
    let (closes, highs, lows, _) = series_arrays(bars);
    let mut out = Vec::new();

    match cross(&sma(&closes, 5), &sma(&closes, 20)) {
        Some(Cross::Golden) => out.push(Signal::MaGoldenCross),
        Some(Cross::Dead) => out.push(Signal::MaDeadCross),
        None => {}
    }

    let kd = kd_taiwan(&highs, &lows, &closes, 9);
    match cross(&kd.k, &kd.d) {
        Some(Cross::Golden) => out.push(Signal::KdGoldenCross),
        Some(Cross::Dead) => out.push(Signal::KdDeadCross),
        None => {}
    }

    let m = macd(&closes, 12, 26, 9);
    match cross(&m.macd, &m.signal) {
        Some(Cross::Golden) => out.push(Signal::MacdGoldenCross),
        Some(Cross::Dead) => out.push(Signal::MacdDeadCross),
        None => {}
    }

    if let Some((prev, last)) = last_two(&rsi(&closes, 14)) {
        if prev <= 70.0 && last > 70.0 {
            out.push(Signal::RsiOverbought);
        }
        if prev >= 30.0 && last < 30.0 {
            out.push(Signal::RsiOversold);
        }
    }

    let boll = bollinger(&closes, 20, 2.0);
    let closes_opt: Vec<Option<f64>> = closes.iter().map(|c| Some(*c)).collect();
    if let Some(Cross::Golden) = cross(&closes_opt, &boll.upper) {
        out.push(Signal::BollBreakUpper);
    }
    if let Some(Cross::Dead) = cross(&closes_opt, &boll.lower) {
        out.push(Signal::BollBreakLower);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars_from(closes: &[f64]) -> Vec<Bar> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| Bar {
                ts: i as i64 * 86_400,
                open: Some(c),
                high: Some(c + 0.5),
                low: Some(c - 0.5),
                close: Some(c),
                volume: Some(1000),
            })
            .collect()
    }

    #[test]
    fn no_cross_reported_when_the_previous_value_was_none() {
        // Exactly at the warm-up boundary of the 20-day MA: the 20th bar makes
        // MA20 defined for the first time, so its previous value is None and no
        // MA cross may be reported off that transition.
        let closes: Vec<f64> = (1..=20).map(f64::from).collect();
        let sigs = detect(&bars_from(&closes));
        assert!(!sigs.contains(&Signal::MaGoldenCross));
        assert!(!sigs.contains(&Signal::MaDeadCross));
    }

    #[test]
    fn flat_series_produces_no_signals() {
        // The basis of the "AI bill stays zero on a flat day" claim.
        let closes = vec![100.0; 80];
        assert!(detect(&bars_from(&closes)).is_empty());
    }

    #[test]
    fn short_history_produces_no_signals() {
        assert!(detect(&[]).is_empty());
        assert!(detect(&bars_from(&[10.0, 11.0, 12.0])).is_empty());
    }

    #[test]
    fn a_genuine_ma_golden_cross_is_detected() {
        // A flat baseline keeps MA5 == MA20 (equal on the penultimate bar), then
        // a single jump on the final bar lifts the faster MA5 above the slower
        // MA20 — a clean golden cross landing exactly on the last index.
        let mut closes = vec![100.0; 30];
        closes.push(200.0);
        let sigs = detect(&bars_from(&closes));
        assert!(
            sigs.contains(&Signal::MaGoldenCross),
            "expected MA golden cross, got {sigs:?}"
        );
    }

    #[test]
    fn cross_helper_ignores_a_none_previous() {
        let a = vec![None, Some(2.0)];
        let b = vec![Some(1.0), Some(1.0)];
        assert_eq!(cross(&a, &b), None);
    }

    #[test]
    fn cross_helper_detects_golden_and_dead() {
        assert_eq!(
            cross(&[Some(1.0), Some(3.0)], &[Some(2.0), Some(2.0)]),
            Some(Cross::Golden)
        );
        assert_eq!(
            cross(&[Some(3.0), Some(1.0)], &[Some(2.0), Some(2.0)]),
            Some(Cross::Dead)
        );
    }
}
