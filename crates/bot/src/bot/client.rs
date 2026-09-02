use teloxide::{prelude::Requester, Bot};
use tracing::warn;

use crate::config::{Config, TelegramMode};

/// Build the single `Bot` used by both scheduler/workers and the inbound
/// dispatcher. Applies `telegram.endpoint` (self-hosted Bot API) when set and
/// best-effort clears any leftover webhook so a previous mode does not
/// silently conflict with the current one.
///
/// Endpoint failures are returned so the caller can fail the process with
/// context; `delete_webhook` failures are only logged — a stale webhook is a
/// soft condition and must not prevent the bot from starting.
pub async fn build_bot(config: &Config) -> anyhow::Result<Bot> {
    let bot = if config.telegram.endpoint.is_empty() {
        Bot::new(config.bot_token.clone())
    } else {
        let url = reqwest::Url::parse(&config.telegram.endpoint).map_err(|err| {
            anyhow::anyhow!(
                "invalid telegram.endpoint {:?}: {err}",
                config.telegram.endpoint
            )
        })?;
        Bot::new(config.bot_token.clone()).set_api_url(url)
    };

    if matches!(config.telegram.mode, TelegramMode::Polling) {
        if let Err(err) = bot.delete_webhook().await {
            warn!(error = %err, "delete_webhook failed; continuing (polling may conflict with a stale webhook)");
        }
    }

    Ok(bot)
}

/// Pure helper extracted from `main` so the broadcast-vs-polling decision can
/// be unit-tested without spinning up the dispatcher. Returns `true` when
/// the long-polling `Dispatcher` should be started.
pub fn should_run_dispatcher(mode: TelegramMode) -> bool {
    matches!(mode, TelegramMode::Polling)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_run_dispatcher_polling() {
        assert!(should_run_dispatcher(TelegramMode::Polling));
    }

    #[test]
    fn should_run_dispatcher_broadcast_is_false() {
        assert!(!should_run_dispatcher(TelegramMode::Broadcast));
    }

    #[tokio::test]
    async fn build_bot_rejects_invalid_endpoint() {
        let mut cfg = Config::default();
        cfg.telegram.endpoint = "not a url".to_owned();
        let err = build_bot(&cfg).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("telegram.endpoint"),
            "expected endpoint error, got: {msg}"
        );
    }
}
