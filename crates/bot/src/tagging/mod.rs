//! Pluggable auto-tagging. Default Gemini free tier, with a local keyword
//! heuristic as the fall-back when no API key is present.

pub mod gemini;
pub mod heuristic;
pub mod mcp;
pub mod metadata;
pub mod quota;
pub mod taxonomy;
pub mod url_norm;
pub mod worker;

use crate::config::{AiProvider, Config};

use gemini::GeminiTagger;
use heuristic::HeuristicTagger;
use mcp::{McpClient, McpTagger};

/// The text a tagger reasons over.
pub struct TagInput<'a> {
    pub title: &'a str,
    pub url: &'a str,
    pub excerpt: &'a str,
}

/// A tagger returns a list of taxonomy slugs (possibly empty). `async fn` in a
/// trait desugars to RPITIT, which is **not** dyn-compatible — so consumers are
/// generic and dispatch goes through `AnyTagger`, mirroring the existing
/// `MessageSender`/`PreviewPublisher` pattern (`Scheduler<P, S>`).
#[allow(async_fn_in_trait)]
pub trait Tagger: Send + Sync {
    async fn suggest(&self, input: &TagInput<'_>) -> anyhow::Result<Vec<String>>;
}

/// Enum dispatch over the concrete taggers (no `Box<dyn Tagger>`: not
/// dyn-compatible).
pub enum AnyTagger {
    Gemini(GeminiTagger),
    Heuristic(HeuristicTagger),
    Mcp(McpTagger),
}

impl Tagger for AnyTagger {
    async fn suggest(&self, input: &TagInput<'_>) -> anyhow::Result<Vec<String>> {
        match self {
            Self::Gemini(t) => t.suggest(input).await,
            Self::Heuristic(t) => t.suggest(input).await,
            Self::Mcp(t) => t.suggest(input).await,
        }
    }
}

impl AnyTagger {
    /// Whether this tagger's daily quota should be metered (Gemini only).
    pub fn is_gemini(&self) -> bool {
        matches!(self, Self::Gemini(_))
    }
}

/// Builds the configured tagger. `provider = "auto"` picks Gemini when an
/// api_key is present, otherwise the heuristic. `mcp` is explicit only.
pub fn build_tagger(cfg: &Config) -> AnyTagger {
    let ai = &cfg.bookmark.ai;
    match ai.provider {
        AiProvider::Mcp => AnyTagger::Mcp(McpTagger::new(
            McpClient::from_config(ai.mcp.clone()),
            ai.max_tags as usize,
        )),
        AiProvider::Gemini => AnyTagger::Gemini(GeminiTagger::new(ai)),
        AiProvider::Auto if !ai.api_key.is_empty() => AnyTagger::Gemini(GeminiTagger::new(ai)),
        AiProvider::Auto | AiProvider::Heuristic => {
            AnyTagger::Heuristic(HeuristicTagger::new(ai.max_tags as usize))
        }
    }
}
