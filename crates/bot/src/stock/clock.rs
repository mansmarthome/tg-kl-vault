//! Market session clock. **Pure** — no network, no DB.
//!
//! Every design decision here rests on one idea: the authoritative market clock
//! is *the data the exchange itself reports*, delivered on every Yahoo response
//! as `meta.gmtoffset` and `meta.currentTradingPeriod.regular.{start,end}`.
//! That is strictly more than a timezone database — it already encodes DST, US
//! half-day early closes, national holidays, and even typhoon closures, none of
//! which live in tzdata. So this module pulls in **no** `chrono-tz`, hand-codes
//! **no** DST rules, and hard-codes **no** trading calendar.

use chrono::DateTime;

/// After the closing bell, wait this long for a finalized bar before declaring
/// the session `NoTrade`. Guards against typhoon days where Yahoo may still
/// announce a window but publish only a zero-volume flat bar.
pub const SETTLE_GRACE_SECS: i64 = 90 * 60;

/// The subset of a Yahoo `meta` block the clock needs. Assembled by the source
/// layer; kept `Copy` and field-only so every function here is trivially unit
/// testable with hand-written values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionMeta {
    /// `meta.gmtoffset` — seconds east of UTC, as observed on this response.
    pub gmtoffset: i64,
    /// `currentTradingPeriod.regular.start` — epoch of the opening bell.
    pub regular_start: i64,
    /// `currentTradingPeriod.regular.end` — epoch of the closing bell. This is
    /// the single most important field: the close gate.
    pub regular_end: i64,
    /// Epoch of the most recent daily bar's start.
    pub last_bar_start: i64,
    /// Whether that bar has a non-null close.
    pub last_bar_has_close: bool,
    /// That bar's volume. `> 0` distinguishes a real session from a phantom
    /// flat bar on a non-trading day.
    pub last_bar_volume: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// The exchange's own session window is not today — weekend, holiday,
    /// Lunar New Year break. No calendar required: Yahoo reflects reality.
    NoSessionToday,
    /// The bell has not rung. We must never treat the live intraday bar (which
    /// *is* dated today) as a closing price.
    Open,
    /// Bell has rung but no finalized bar yet, still within the grace window.
    Settling,
    /// A finalized bar exists for today. `trading_day` always comes from data.
    Closed { trading_day: i64 },
    /// Bell rang, grace elapsed, still no real bar (e.g. typhoon half-close).
    NoTrade { trading_day: i64 },
}

/// Market-local calendar day as an integer day index (days since the Unix
/// epoch). `div_euclid` (not `/`) so negative `gmtoffset` (US markets) floors
/// correctly instead of truncating toward zero.
pub fn market_day(epoch: i64, gmtoffset: i64) -> i64 {
    (epoch + gmtoffset).div_euclid(86_400)
}

/// Renders a market-local day index as `'YYYY-MM-DD'` for storage in the
/// `trade_date` columns. `day * 86400` is midnight UTC of that civil date, so
/// formatting in UTC recovers the intended calendar date.
pub fn market_date_string(day: i64) -> String {
    DateTime::from_timestamp(day * 86_400, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// The heart of the feature. See the module doc for why this needs no calendar.
pub fn classify_session(now: i64, m: SessionMeta) -> SessionState {
    let today = market_day(now, m.gmtoffset);
    let session_day = market_day(m.regular_start, m.gmtoffset);

    // Gate 1: the exchange's declared session window isn't today.
    if session_day != today {
        return SessionState::NoSessionToday;
    }

    // The bell has not rung. This one line is the feature's most important
    // defense: it was verified in testing that "the last bar is dated today"
    // is *true intraday*, so relying on that alone would print the open jitter
    // as the close.
    if now < m.regular_end {
        return SessionState::Open;
    }

    // Gate 2: the bell rang — is there a *finalized* bar for today? `volume > 0`
    // is the discriminator: on a typhoon day Yahoo may announce the window and
    // carry a flat prior-close bar with zero volume.
    let bar_day = market_day(m.last_bar_start, m.gmtoffset);
    if bar_day == today && m.last_bar_has_close && m.last_bar_volume > 0 {
        return SessionState::Closed { trading_day: today };
    }
    if now >= m.regular_end + SETTLE_GRACE_SECS {
        return SessionState::NoTrade { trading_day: today };
    }
    SessionState::Settling
}

/// A user's preferred push time, as minutes from market-local midnight
/// (`0..=1439`). Stored directly in `stock_push_settings.push_minute` so the
/// hot loop never parses `"HH:MM"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushTime(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushDecision {
    /// Not eligible this pass. The `&'static str` is a log reason only.
    Skip(&'static str),
    Send {
        trading_day: i64,
        /// The computed fire epoch (UTC).
        at: i64,
        /// `now` is more than `late_threshold_secs` past `at`; renderers add a
        /// "delayed" marker.
        late: bool,
    },
}

/// UTC epoch at which the close report should fire for `trading_day`.
///
/// With an explicit preference we clamp *up* to `regular_end` — a push time
/// before the close is meaningless for a *close* report and must never fire at
/// 09:00 with yesterday's numbers. Without one, we fire `default_delay_secs`
/// after the bell.
pub fn push_epoch(
    trading_day: i64,
    m: SessionMeta,
    pref: Option<PushTime>,
    default_delay_secs: i64,
) -> i64 {
    match pref {
        Some(PushTime(minute)) => {
            let local_midnight = trading_day * 86_400 - m.gmtoffset;
            (local_midnight + minute * 60).max(m.regular_end)
        }
        None => m.regular_end + default_delay_secs,
    }
}

/// Decides whether to push a close report for this chat/market on this pass.
///
/// Storing "the day we already sent" (`last_sent_day`) rather than "the next
/// push instant" is what makes all four same-day time-change cases fall out for
/// free, and what makes weekends/holidays a silent no-op (the latest bar is
/// still the previous session, whose row already exists).
#[allow(clippy::too_many_arguments)]
pub fn decide_push(
    now: i64,
    session: SessionState,
    meta: SessionMeta,
    enabled: bool,
    pref: Option<PushTime>,
    default_delay_secs: i64,
    last_sent_day: Option<i64>,
    late_threshold_secs: i64,
) -> PushDecision {
    let trading_day = match session {
        SessionState::Closed { trading_day } => trading_day,
        SessionState::NoSessionToday => return PushDecision::Skip("no session today"),
        SessionState::Open => return PushDecision::Skip("market open"),
        SessionState::Settling => return PushDecision::Skip("settling"),
        // No finalized bar means no close to report. The idempotency key is
        // never reached; nothing is sent.
        SessionState::NoTrade { .. } => return PushDecision::Skip("no trade today"),
    };

    if !enabled {
        return PushDecision::Skip("push disabled");
    }

    // Idempotency. The key is the *exchange* trading day, never the local clock,
    // so a backwards clock jump cannot resend and a same-day push-time change
    // after we've already sent produces nothing.
    if last_sent_day == Some(trading_day) {
        return PushDecision::Skip("already sent today");
    }

    // Same market-local-day boundary is non-negotiable: we backfill *within*
    // the trading day (an 17:00 and a 21:00 report carry identical numbers) but
    // never across days. `SessionMeta` always describes the latest session, so
    // earlier days are structurally unreachable anyway — this guards the edge
    // where the clock has rolled past local midnight while this row was pending.
    if market_day(now, meta.gmtoffset) != trading_day {
        return PushDecision::Skip("stale day, no backfill");
    }

    let at = push_epoch(trading_day, meta, pref, default_delay_secs);
    if now < at {
        return PushDecision::Skip("not yet time");
    }
    let late = now >= at + late_threshold_secs;
    PushDecision::Send {
        trading_day,
        at,
        late,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verified fixtures (2026-08-21), quoted from the design doc.
    // AAPL: regular start 1787319000 (09:30 ET), end 1787342400 (16:00 ET),
    //       gmtoffset -14400 (EDT). regularMarketTime observed 1787324271.
    // 2330.TW: regular start 1787274000 (09:00 TPE), end 1787290200
    //          (13:30 TPE), gmtoffset 28800.
    const AAPL_START: i64 = 1_787_319_000;
    const AAPL_END: i64 = 1_787_342_400;
    const AAPL_OFFSET: i64 = -14_400;
    const TW_START: i64 = 1_787_274_000;
    const TW_END: i64 = 1_787_290_200;
    const TW_OFFSET: i64 = 28_800;

    fn aapl_meta(last_bar_start: i64, volume: i64) -> SessionMeta {
        SessionMeta {
            gmtoffset: AAPL_OFFSET,
            regular_start: AAPL_START,
            regular_end: AAPL_END,
            last_bar_start,
            last_bar_has_close: true,
            last_bar_volume: volume,
        }
    }

    #[test]
    fn a_live_intraday_bar_dated_today_is_never_treated_as_a_close() {
        // 14:57 UTC = 10:57 ET — bell has not rung, but today's bar exists.
        let now = 1_787_324_271;
        let m = aapl_meta(AAPL_START, 1_000_000);
        assert!(now < AAPL_END, "precondition: still intraday");
        assert_eq!(market_day(AAPL_START, AAPL_OFFSET), market_day(now, AAPL_OFFSET));
        assert_eq!(classify_session(now, m), SessionState::Open);
    }

    #[test]
    fn taiwan_close_is_1330_local_and_needs_no_timezone_database() {
        // 1787290200 + 28800 = 1787319000; % 86400 = 48600s = 13:30 local.
        assert_eq!((TW_END + TW_OFFSET).rem_euclid(86_400), 13 * 3600 + 30 * 60);
        let today = market_day(TW_END, TW_OFFSET);
        let m = SessionMeta {
            gmtoffset: TW_OFFSET,
            regular_start: TW_START,
            regular_end: TW_END,
            last_bar_start: TW_START,
            last_bar_has_close: true,
            last_bar_volume: 17_000_000,
        };
        // One second after the close, a real bar exists -> Closed.
        assert_eq!(
            classify_session(TW_END + 1, m),
            SessionState::Closed { trading_day: today }
        );
        // One second before -> Open.
        assert_eq!(classify_session(TW_END - 1, m), SessionState::Open);
    }

    #[test]
    fn us_dst_offset_comes_from_the_payload_so_both_halves_of_the_year_work() {
        // EDT (-14400): close 16:00 local.
        let edt = market_day(AAPL_END, -14_400);
        // EST (-18000) would shift the very same epoch back an hour of local
        // time, but because we read the offset from the payload both compute a
        // coherent local day. We never hard-code either.
        let est_meta = SessionMeta {
            gmtoffset: -18_000,
            regular_start: AAPL_START - 3600,
            regular_end: AAPL_END - 3600,
            last_bar_start: AAPL_START - 3600,
            last_bar_has_close: true,
            last_bar_volume: 5,
        };
        let now = est_meta.regular_end + 1;
        assert_eq!(
            classify_session(now, est_meta),
            SessionState::Closed {
                trading_day: market_day(now, -18_000)
            }
        );
        let _ = edt;
    }

    #[test]
    fn typhoon_day_a_scheduled_window_with_no_volume_is_not_a_trading_day() {
        let now = AAPL_END + SETTLE_GRACE_SECS + 1;
        // Window is today, bell rang, grace elapsed, but the bar has no volume.
        let m = aapl_meta(AAPL_START, 0);
        assert_eq!(
            classify_session(now, m),
            SessionState::NoTrade {
                trading_day: market_day(now, AAPL_OFFSET)
            }
        );
    }

    #[test]
    fn weekend_is_no_session_today() {
        // now two days after the session window -> Gate 1 fires.
        let now = AAPL_END + 2 * 86_400;
        let m = aapl_meta(AAPL_START, 1_000);
        assert_eq!(classify_session(now, m), SessionState::NoSessionToday);
    }

    #[test]
    fn settling_before_grace_elapses() {
        // Bell rang 10 minutes ago, no finalized bar yet.
        let now = AAPL_END + 600;
        let m = aapl_meta(AAPL_START - 86_400, 0); // stale bar
        assert_eq!(classify_session(now, m), SessionState::Settling);
    }

    #[test]
    fn changing_the_push_time_after_todays_report_never_produces_a_second_one() {
        let td = market_day(AAPL_END, AAPL_OFFSET);
        let session = SessionState::Closed { trading_day: td };
        let m = aapl_meta(AAPL_START, 1);
        let now = AAPL_END + 3600;
        // Same day already stamped -> Skip regardless of the new preference.
        for pref in [None, Some(PushTime(17 * 60)), Some(PushTime(23 * 60))] {
            assert!(matches!(
                decide_push(now, session, m, true, pref, 3600, Some(td), 1800),
                PushDecision::Skip(_)
            ));
        }
    }

    #[test]
    fn a_three_day_outage_backfills_nothing() {
        // Process wakes Thursday evening after a Mon–Wed outage. `meta` only
        // ever describes the *latest* session (Thursday), so Mon/Tue/Wed are
        // structurally unreachable; exactly one report — Thursday's — is due.
        let td = market_day(AAPL_END, AAPL_OFFSET);
        let session = SessionState::Closed { trading_day: td };
        let m = aapl_meta(AAPL_START, 1);
        let now = AAPL_END + 2 * 3600;
        match decide_push(now, session, m, true, None, 3600, None, 1800) {
            PushDecision::Send { trading_day, .. } => assert_eq!(trading_day, td),
            other => panic!("expected Send, got {other:?}"),
        }
    }

    #[test]
    fn a_backwards_clock_jump_cannot_resend() {
        let td = market_day(AAPL_END, AAPL_OFFSET);
        let session = SessionState::Closed { trading_day: td };
        let m = aapl_meta(AAPL_START, 1);
        // Clock jumps back before the close, but the day is already stamped.
        let now = AAPL_START + 60;
        assert!(matches!(
            decide_push(now, session, m, true, None, 3600, Some(td), 1800),
            PushDecision::Skip(_)
        ));
    }

    #[test]
    fn push_epoch_clamps_a_preference_up_to_the_close() {
        let td = market_day(AAPL_END, AAPL_OFFSET);
        let m = aapl_meta(AAPL_START, 1);
        // 09:00 local is before the 16:00 close -> clamped up to regular_end.
        assert_eq!(push_epoch(td, m, Some(PushTime(9 * 60)), 3600), AAPL_END);
        // None -> close + delay.
        assert_eq!(push_epoch(td, m, None, 3600), AAPL_END + 3600);
        // A time after the close is honored.
        let evening = td * 86_400 - AAPL_OFFSET + 20 * 3600;
        assert_eq!(push_epoch(td, m, Some(PushTime(20 * 60)), 3600), evening);
    }

    #[test]
    fn late_flag_trips_past_the_threshold() {
        let td = market_day(AAPL_END, AAPL_OFFSET);
        let session = SessionState::Closed { trading_day: td };
        let m = aapl_meta(AAPL_START, 1);
        let at = push_epoch(td, m, None, 3600);
        match decide_push(at + 1801, session, m, true, None, 3600, None, 1800) {
            PushDecision::Send { late, .. } => assert!(late),
            other => panic!("expected Send, got {other:?}"),
        }
        match decide_push(at + 60, session, m, true, None, 3600, None, 1800) {
            PushDecision::Send { late, .. } => assert!(!late),
            other => panic!("expected Send, got {other:?}"),
        }
    }

    #[test]
    fn disabled_and_not_yet_time_both_skip() {
        let td = market_day(AAPL_END, AAPL_OFFSET);
        let session = SessionState::Closed { trading_day: td };
        let m = aapl_meta(AAPL_START, 1);
        assert!(matches!(
            decide_push(AAPL_END + 3600, session, m, false, None, 3600, None, 1800),
            PushDecision::Skip(_)
        ));
        // Enabled but before the fire epoch.
        assert!(matches!(
            decide_push(AAPL_END + 10, session, m, true, None, 3600, None, 1800),
            PushDecision::Skip(_)
        ));
    }

    #[test]
    fn market_date_string_is_utc_midnight_of_the_index() {
        assert_eq!(market_date_string(0), "1970-01-01");
        assert_eq!(market_date_string(market_day(TW_END, TW_OFFSET)), "2026-08-21");
    }
}
