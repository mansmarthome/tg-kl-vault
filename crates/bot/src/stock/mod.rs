//! Stock tracking feature: watchlists, quotes, technical indicators, and
//! close-of-day push reports for TWSE / TPEx (台股) and US markets.
//!
//! The layout follows the same three-layer shape as the RSS feature: pure
//! function modules (no `Bot`, no network, no `Repo`) at the bottom, an IO
//! `StockService` facade in the middle, and thin Telegram handlers / a schedule
//! worker on top. See `.context/plans/stock-tracking.md` for the full design.

pub mod bars;
pub mod clock;
pub mod commentary;
pub mod indicators;
pub mod render;
pub mod service;
pub mod signals;
pub mod source;
pub mod symbol;
pub mod worker;

pub use bars::{bar_is_sane, Bar, Quote, Series};
pub use clock::{
    classify_session, decide_push, market_date_string, market_day, push_epoch, PushDecision,
    PushTime, SessionMeta, SessionState, SETTLE_GRACE_SECS,
};
pub use indicators::{snapshot, Snapshot, MIN_BARS_FOR_INDICATORS};
pub use render::{render_daily_report, render_quote_card, render_watchlist, ReportEntry};
pub use service::{
    AddError, AddOutcome, MarketScope, QuoteView, StockService, WatchlistPage,
};
pub use signals::{detect, Signal};
pub use source::{classify_source_error, SourceError, StockSource, TwOfficialSource, YahooSource};
pub use symbol::{parse, Board, Market, Parsed, Symbol};
pub use worker::{manual_report_day, render_report_chunks, StockWorker};
