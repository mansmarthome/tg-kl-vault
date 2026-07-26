//! Error types.
//!
//! Note that `html_to_nodes` is infallible: malformed input produces
//! best-effort output, never an error. Only the API client fails.

/// Errors from the Telegraph API client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The API returned `ok: false` with this `error` string.
    #[error("telegraph api error: {0}")]
    Api(String),

    #[cfg(feature = "client")]
    #[error("http error")]
    Http(#[from] reqwest::Error),

    /// Distinct from `Api` so callers can put the offending token on cooldown
    /// rather than concluding the page is broken.
    #[error("flood wait: retry after {0}s")]
    FloodWait(u64),

    #[error("content too large")]
    TooLarge,
}
