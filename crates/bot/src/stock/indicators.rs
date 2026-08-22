//! Technical indicators. **Pure** — no network, no DB.
//!
//! Single load-bearing rule: **every series returns a vector the same length as
//! its input, with `None` wherever the value is undefined.** This makes
//! "insufficient history" a non-event — no error, no panic, and no zero-fill
//! (zero-fill would draw a cliff from 0 to the price on every average and make
//! every "golden cross" test pass for the wrong reason).

use super::bars::Bar;

/// MACD 26 + signal 9 is the strictest warm-up; below this no indicator block
/// should be rendered.
pub const MIN_BARS_FOR_INDICATORS: usize = 35;

/// Simple moving average. First defined index is `period - 1`.
pub fn sma(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let period = period.max(1);
    let mut out = vec![None; values.len()];
    if values.len() < period {
        return out;
    }
    let mut sum: f64 = values[..period].iter().sum();
    out[period - 1] = Some(sum / period as f64);
    for i in period..values.len() {
        sum += values[i] - values[i - period];
        out[i] = Some(sum / period as f64);
    }
    out
}

/// Exponential moving average, seeded with `SMA(period)` at index `period - 1`.
pub fn ema(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let period = period.max(1);
    let mut out = vec![None; values.len()];
    if values.len() < period {
        return out;
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let seed: f64 = values[..period].iter().sum::<f64>() / period as f64;
    out[period - 1] = Some(seed);
    let mut prev = seed;
    for i in period..values.len() {
        prev = values[i] * alpha + prev * (1.0 - alpha);
        out[i] = Some(prev);
    }
    out
}

fn rsi_value(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss == 0.0 {
        100.0
    } else {
        let rs = avg_gain / avg_loss;
        100.0 - 100.0 / (1.0 + rs)
    }
}

/// Wilder-smoothed RSI. First defined index is `period` (needs `period` deltas).
pub fn rsi(closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let period = period.max(1);
    let n = closes.len();
    let mut out = vec![None; n];
    if n <= period {
        return out;
    }
    let (mut gain, mut loss) = (0.0, 0.0);
    for i in 1..=period {
        let d = closes[i] - closes[i - 1];
        if d >= 0.0 {
            gain += d;
        } else {
            loss -= d;
        }
    }
    let mut avg_gain = gain / period as f64;
    let mut avg_loss = loss / period as f64;
    out[period] = Some(rsi_value(avg_gain, avg_loss));
    let p = period as f64;
    for i in period + 1..n {
        let d = closes[i] - closes[i - 1];
        let (g, l) = if d >= 0.0 { (d, 0.0) } else { (0.0, -d) };
        avg_gain = (avg_gain * (p - 1.0) + g) / p;
        avg_loss = (avg_loss * (p - 1.0) + l) / p;
        out[i] = Some(rsi_value(avg_gain, avg_loss));
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Macd {
    pub macd: Vec<Option<f64>>,
    pub signal: Vec<Option<f64>>,
    pub histogram: Vec<Option<f64>>,
}

/// MACD (default 12/26/9). The signal line is the EMA of the *defined* portion
/// of the MACD line, mapped back onto the original indices.
pub fn macd(closes: &[f64], fast: usize, slow: usize, signal: usize) -> Macd {
    let n = closes.len();
    let ef = ema(closes, fast);
    let es = ema(closes, slow);
    let mut macd_line = vec![None; n];
    for i in 0..n {
        if let (Some(a), Some(b)) = (ef[i], es[i]) {
            macd_line[i] = Some(a - b);
        }
    }

    let mut signal_line = vec![None; n];
    let mut histogram = vec![None; n];
    if let Some(start) = macd_line.iter().position(Option::is_some) {
        // The MACD line is contiguous from `start` (both EMAs are), so
        // filter_map preserves alignment with no internal gaps.
        let defined: Vec<f64> = macd_line[start..].iter().filter_map(|x| *x).collect();
        for (k, s) in ema(&defined, signal).into_iter().enumerate() {
            signal_line[start + k] = s;
            if let (Some(m), Some(sg)) = (macd_line[start + k], s) {
                histogram[start + k] = Some(m - sg);
            }
        }
    }
    Macd {
        macd: macd_line,
        signal: signal_line,
        histogram,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Kd {
    pub k: Vec<Option<f64>>,
    pub d: Vec<Option<f64>>,
}

/// Taiwan-convention KD — what a zh-TW user means by "KD", **not** the US slow
/// stochastic. RSV over `rsv_period` days; `K = 2/3·K_prev + 1/3·RSV`,
/// `D = 2/3·D_prev + 1/3·K`, both seeded at 50. Getting this wrong is the #1
/// cause of "the numbers don't match my broker app". First defined index is
/// `rsv_period - 1`.
pub fn kd_taiwan(highs: &[f64], lows: &[f64], closes: &[f64], rsv_period: usize) -> Kd {
    let rsv_period = rsv_period.max(1);
    let n = closes.len();
    let mut k = vec![None; n];
    let mut d = vec![None; n];
    if n < rsv_period || highs.len() != n || lows.len() != n {
        return Kd { k, d };
    }
    let (mut kp, mut dp) = (50.0f64, 50.0f64);
    for i in rsv_period - 1..n {
        let window = i + 1 - rsv_period..=i;
        let hi = highs[window.clone()].iter().copied().fold(f64::MIN, f64::max);
        let lo = lows[window].iter().copied().fold(f64::MAX, f64::min);
        let denom = hi - lo;
        // Zero range (a perfectly flat window) has no defined RSV; use 0 so the
        // series stays finite. Rare, and never produces a cross on its own.
        let rsv = if denom.abs() < f64::EPSILON {
            0.0
        } else {
            (closes[i] - lo) / denom * 100.0
        };
        kp = 2.0 / 3.0 * kp + 1.0 / 3.0 * rsv;
        dp = 2.0 / 3.0 * dp + 1.0 / 3.0 * kp;
        k[i] = Some(kp);
        d[i] = Some(dp);
    }
    Kd { k, d }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bollinger {
    pub middle: Vec<Option<f64>>,
    pub upper: Vec<Option<f64>>,
    pub lower: Vec<Option<f64>>,
}

/// Bollinger Bands. Population standard deviation (÷n), the charting-package
/// convention. `k` is a parameter (call sites pass `2.0`) — deliberately not a
/// config field, because `Config` derives `Eq` and cannot hold an `f64`.
pub fn bollinger(closes: &[f64], period: usize, k: f64) -> Bollinger {
    let period = period.max(1);
    let n = closes.len();
    let middle = sma(closes, period);
    let mut upper = vec![None; n];
    let mut lower = vec![None; n];
    for i in 0..n {
        if let Some(m) = middle[i] {
            let window = &closes[i + 1 - period..=i];
            let var = window.iter().map(|x| (x - m).powi(2)).sum::<f64>() / period as f64;
            let sd = var.sqrt();
            upper[i] = Some(m + k * sd);
            lower[i] = Some(m - k * sd);
        }
    }
    Bollinger {
        middle,
        upper,
        lower,
    }
}

/// The last defined value of each indicator, for the quote card / report line.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub bars_used: usize,
    pub last_close: Option<f64>,
    pub prev_close: Option<f64>,
    pub last_volume: Option<i64>,
    pub ma5: Option<f64>,
    pub ma20: Option<f64>,
    pub ma60: Option<f64>,
    pub rsi14: Option<f64>,
    pub macd: Option<f64>,
    pub macd_signal: Option<f64>,
    pub macd_hist: Option<f64>,
    pub k: Option<f64>,
    pub d: Option<f64>,
    pub boll_mid: Option<f64>,
    pub boll_upper: Option<f64>,
    pub boll_lower: Option<f64>,
}

/// Extracts aligned close/high/low/volume arrays from bars that have a close.
/// Missing high/low fall back to the close (a doji-like degenerate bar), never
/// dropped, so KD/Bollinger stay aligned with the close series.
pub(crate) fn series_arrays(bars: &[Bar]) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<Option<i64>>) {
    let mut closes = Vec::new();
    let mut highs = Vec::new();
    let mut lows = Vec::new();
    let mut vols = Vec::new();
    for b in bars {
        if let Some(c) = b.close {
            closes.push(c);
            highs.push(b.high.unwrap_or(c));
            lows.push(b.low.unwrap_or(c));
            vols.push(b.volume);
        }
    }
    (closes, highs, lows, vols)
}

fn last_defined(v: &[Option<f64>]) -> Option<f64> {
    v.iter().rev().find_map(|x| *x)
}

pub fn snapshot(bars: &[Bar]) -> Snapshot {
    let (closes, highs, lows, vols) = series_arrays(bars);
    let macd = macd(&closes, 12, 26, 9);
    let kd = kd_taiwan(&highs, &lows, &closes, 9);
    let boll = bollinger(&closes, 20, 2.0);
    Snapshot {
        bars_used: closes.len(),
        last_close: closes.last().copied(),
        prev_close: (closes.len() >= 2).then(|| closes[closes.len() - 2]),
        last_volume: vols.last().copied().flatten(),
        ma5: last_defined(&sma(&closes, 5)),
        ma20: last_defined(&sma(&closes, 20)),
        ma60: last_defined(&sma(&closes, 60)),
        rsi14: last_defined(&rsi(&closes, 14)),
        macd: last_defined(&macd.macd),
        macd_signal: last_defined(&macd.signal),
        macd_hist: last_defined(&macd.histogram),
        k: last_defined(&kd.k),
        d: last_defined(&kd.d),
        boll_mid: last_defined(&boll.middle),
        boll_upper: last_defined(&boll.upper),
        boll_lower: last_defined(&boll.lower),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Option<f64>, b: f64) {
        let a = a.expect("expected a defined value");
        assert!((a - b).abs() < 1e-3, "{a} vs {b}");
    }

    #[test]
    fn every_indicator_returns_input_length_with_none_prefix() {
        let v: Vec<f64> = (1..=50).map(f64::from).collect();
        for out in [sma(&v, 5), ema(&v, 5), rsi(&v, 14)] {
            assert_eq!(out.len(), v.len());
        }
        assert_eq!(macd(&v, 12, 26, 9).macd.len(), v.len());
        let (h, l, c, _) = series_arrays(&[]);
        assert!(h.is_empty() && l.is_empty() && c.is_empty());
    }

    #[test]
    fn sma_first_defined_index_is_period_minus_one() {
        let v = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let s = sma(&v, 3);
        assert_eq!(s[0], None);
        assert_eq!(s[1], None);
        approx(s[2], 2.0); // (1+2+3)/3
        approx(s[3], 3.0);
        approx(s[4], 4.0);
    }

    #[test]
    fn rsi_of_a_monotonically_increasing_series_is_one_hundred() {
        let v: Vec<f64> = (1..=30).map(f64::from).collect();
        approx(last_defined(&rsi(&v, 14)), 100.0);
    }

    #[test]
    fn kd_taiwan_seeds_at_fifty_and_matches_pinned_vector() {
        // closes = highs = lows, monotonically increasing, rsv_period = 3.
        let c = vec![10.0, 11.0, 12.0, 13.0];
        let kd = kd_taiwan(&c, &c, &c, 3);
        // Undefined until the first RSV at index rsv_period-1 = 2.
        assert_eq!(kd.k[0], None);
        assert_eq!(kd.k[1], None);
        // i=2: RSV=100, K = 2/3*50 + 1/3*100 = 66.667, D = 2/3*50 + 1/3*K = 55.556
        approx(kd.k[2], 66.6667);
        approx(kd.d[2], 55.5556);
        // i=3: RSV=100, K = 2/3*66.667 + 1/3*100 = 77.778, D = 2/3*55.556 + 1/3*K
        approx(kd.k[3], 77.7778);
        approx(kd.d[3], 62.9630);
    }

    #[test]
    fn macd_signal_and_histogram_align() {
        let v: Vec<f64> = (1..=60).map(f64::from).collect();
        let m = macd(&v, 12, 26, 9);
        // MACD line defined from index 25 (slow-1); signal from 25+8=33.
        assert_eq!(m.macd[24], None);
        assert!(m.macd[25].is_some());
        assert_eq!(m.signal[32], None);
        assert!(m.signal[33].is_some());
        // histogram defined exactly where both are.
        assert!(m.histogram[33].is_some());
        assert_eq!(m.histogram[32], None);
    }

    #[test]
    fn bollinger_bands_bracket_the_middle() {
        let v: Vec<f64> = (1..=30).map(f64::from).collect();
        let b = bollinger(&v, 20, 2.0);
        let (u, m, l) = (
            last_defined(&b.upper).unwrap(),
            last_defined(&b.middle).unwrap(),
            last_defined(&b.lower).unwrap(),
        );
        assert!(u > m && m > l);
    }

    #[test]
    fn short_history_yields_none_not_panic() {
        for v in [vec![], vec![1.0], (1..=34).map(f64::from).collect::<Vec<_>>()] {
            assert!(last_defined(&sma(&v, 60)).is_none());
            assert!(last_defined(&rsi(&v, 14)).is_none() || v.len() > 14);
        }
    }

    #[test]
    fn period_zero_is_clamped_not_a_divide_by_zero() {
        let v = vec![1.0, 2.0, 3.0];
        // Would panic / produce NaN if period 0 weren't clamped to 1.
        approx(sma(&v, 0)[0], 1.0);
        approx(ema(&v, 0)[0], 1.0);
    }

    #[test]
    fn snapshot_reports_bars_used_and_last_close() {
        let bars: Vec<Bar> = (1..=60)
            .map(|i| Bar {
                ts: i64::from(i) * 86_400,
                open: Some(f64::from(i)),
                high: Some(f64::from(i) + 1.0),
                low: Some(f64::from(i) - 1.0),
                close: Some(f64::from(i)),
                volume: Some(1000),
            })
            .collect();
        let s = snapshot(&bars);
        assert_eq!(s.bars_used, 60);
        approx(s.last_close, 60.0);
        approx(s.prev_close, 59.0);
        assert!(s.ma5.is_some() && s.ma60.is_some());
    }
}
