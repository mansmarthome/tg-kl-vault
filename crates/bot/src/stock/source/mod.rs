//! Stock data source abstraction. Mirrors `tagging::{Tagger, AnyTagger}`: an
//! `#[allow(async_fn_in_trait)]` trait (desugars to RPITIT, not dyn-compatible),
//! plus an enum for runtime dispatch — never `Box<dyn>`.

pub mod roc;
pub mod tw_official;
pub mod yahoo;

pub use tw_official::{TwBar, TwOfficialSource};
pub use yahoo::YahooSource;

use super::bars::Series;
use super::symbol::{Board, Symbol};

/// A classified data-source failure. Wrapped in `anyhow::Error` at the call
/// site and recovered via `downcast_ref`, so the service can tell "you typed a
/// bad symbol" (`NotFound`) apart from "the upstream is having a bad afternoon"
/// (`Unreachable`) — a distinction the caller renders very differently.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    #[error("symbol not found")]
    NotFound,
    #[error("rate limited")]
    RateLimited,
    #[error("http {0}")]
    Http(u16),
    #[error("unreachable: {0}")]
    Unreachable(String),
    #[error("malformed response: {0}")]
    Malformed(String),
}

/// Recovers a [`SourceError`] from an erased `anyhow::Error`: first an explicit
/// `SourceError` we attached, then a `reqwest::Error` by HTTP status, else a
/// transport failure. Shaped after `feedcheck::classify_fetch_error`.
pub fn classify_source_error(err: &anyhow::Error) -> SourceError {
    if let Some(se) = err.downcast_ref::<SourceError>() {
        return se.clone();
    }
    if let Some(re) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = re.status() {
            return status_to_error(status.as_u16());
        }
        return SourceError::Unreachable(first_line(&re.to_string()));
    }
    SourceError::Unreachable(first_line(&err.to_string()))
}

pub(crate) fn status_to_error(code: u16) -> SourceError {
    match code {
        404 => SourceError::NotFound,
        429 => SourceError::RateLimited,
        other => SourceError::Http(other),
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_owned()
}

/// A source of daily OHLCV bars + session meta for a symbol.
#[allow(async_fn_in_trait)]
pub trait StockSource: Send + Sync {
    /// May return fewer than `days` bars: the TWSE/TPEx daily dumps carry only
    /// *one* session, so callers check `bars.len()` and never assume.
    async fn series(&self, sym: &Symbol, days: u16) -> anyhow::Result<Series>;
    fn supports(&self, board: Board) -> bool;
    fn name(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_error_round_trips_through_anyhow() {
        for se in [
            SourceError::NotFound,
            SourceError::RateLimited,
            SourceError::Http(503),
            SourceError::Unreachable("dns".into()),
            SourceError::Malformed("bad json".into()),
        ] {
            let err: anyhow::Error = se.clone().into();
            assert_eq!(classify_source_error(&err), se);
        }
    }

    #[test]
    fn status_codes_map_to_the_right_variant() {
        assert_eq!(status_to_error(404), SourceError::NotFound);
        assert_eq!(status_to_error(429), SourceError::RateLimited);
        assert_eq!(status_to_error(503), SourceError::Http(503));
    }
}
