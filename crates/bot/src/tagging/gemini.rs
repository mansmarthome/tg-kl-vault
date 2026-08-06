//! Google Gemini tagger (free tier). Pure HTTP client: it returns `Err` (for
//! transient failures the worker should back off on) or `Ok(vec![])` (for
//! give-up cases the worker should heuristic-finalize) — it never falls back
//! itself. That keeps it unit-testable against a local one-shot server.
//!
//! IMPLEMENTATION NOTES (verified against docs 2026-08-06; re-confirm with one
//! real call before trusting in production — see design step 3):
//!   * Endpoint `POST {endpoint}/v1beta/models/{model}:generateContent`, auth
//!     via the `x-goog-api-key` header.
//!   * We request structured output via `generationConfig.responseSchema`
//!     (the OpenAPI-subset form, proto-JSON uppercase type names). The newer
//!     `responseJsonSchema` (full JSON Schema) is the alternative the
//!     `gemini-3.1-flash-lite` sample uses; pick ONE after a real call. This
//!     code uses `responseSchema` with an `enum` of the taxonomy slugs, adding
//!     a structural constraint on top of `taxonomy::normalize`.
//!   * `generateContent` is now labelled legacy (there is an Interactions API),
//!     but for "title in, category out" it remains the simplest working choice.
//!   * NO `thinkingConfig`: Gemini 3.x Flash-Lite defaults `thinkingLevel` to
//!     `minimal`, which is what a high-throughput classifier wants; and
//!     `thinkingBudget` is superseded by `thinkingLevel` — hardcoding it would
//!     only break on a model swap.
//!   * `temperature` is hardcoded 0.0 (a config field would break Config's Eq
//!     via f32); `maxOutputTokens` 64.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tracing::{error, warn};

use crate::config::AiConfig;
use crate::ratelimit::MinIntervalLimiter;

use super::taxonomy;
use super::{TagInput, Tagger};

pub struct GeminiTagger {
    client: Client,
    endpoint: String,
    model: String,
    api_key: String,
    max_tags: usize,
    limiter: MinIntervalLimiter,
    /// Hard, process-lifetime disable set on a 4xx (bad key/model). Without it
    /// a typo'd key would fire one doomed request per bookmark, forever.
    disabled: AtomicBool,
    /// Soft cooldown set on a 429; requests short-circuit until it passes.
    cooldown_until: Mutex<Option<Instant>>,
}

impl GeminiTagger {
    pub fn new(ai: &AiConfig) -> Self {
        let spacing = Duration::from_secs_f64(60.0 / f64::from(ai.max_rpm.max(1)));
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self {
            client,
            endpoint: ai.endpoint.trim_end_matches('/').to_owned(),
            model: ai.model.clone(),
            api_key: ai.api_key.clone(),
            max_tags: ai.max_tags.max(1) as usize,
            limiter: MinIntervalLimiter::new(spacing),
            disabled: AtomicBool::new(false),
            cooldown_until: Mutex::new(None),
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::SeqCst)
    }

    fn in_cooldown(&self) -> bool {
        matches!(*self.cooldown_until.lock().unwrap(), Some(t) if Instant::now() < t)
    }

    fn set_cooldown(&self, dur: Duration) {
        *self.cooldown_until.lock().unwrap() = Some(Instant::now() + dur);
    }

    fn url(&self) -> String {
        format!("{}/v1beta/models/{}:generateContent", self.endpoint, self.model)
    }

    /// Builds the request body. Factored out so a unit test can assert its
    /// shape without any network.
    pub fn build_request_body(&self, input: &TagInput<'_>) -> Value {
        let slugs: Vec<&str> = taxonomy::all_slugs().collect();
        let prompt = format!(
            "Classify this bookmark into 1 to {max} of these categories, choosing the most \
             specific. Reply ONLY with a JSON array of category slugs from the list.\n\
             Categories: {cats}\n\nTitle: {title}\nURL: {url}\nExcerpt: {excerpt}",
            max = self.max_tags,
            cats = slugs.join(", "),
            title = input.title,
            url = input.url,
            excerpt = input.excerpt,
        );
        json!({
            "contents": [{ "parts": [{ "text": prompt }] }],
            "generationConfig": {
                "temperature": 0.0,
                "maxOutputTokens": 64,
                "responseMimeType": "application/json",
                "responseSchema": {
                    "type": "ARRAY",
                    "items": { "type": "STRING", "enum": slugs }
                }
            }
        })
    }

    /// Parses the doubly-encoded response: `candidates[0].content.parts[0].text`
    /// is itself a JSON string that must be parsed again.
    fn parse_response(&self, body: &Value) -> Vec<String> {
        let Some(text) = body["candidates"][0]["content"]["parts"][0]["text"].as_str() else {
            return Vec::new();
        };
        let Ok(raw): Result<Vec<String>, _> = serde_json::from_str(text) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for slug in raw {
            if let Some(canonical) = taxonomy::normalize(&slug) {
                if !out.iter().any(|s| s == canonical) {
                    out.push(canonical.to_owned());
                }
            }
            if out.len() >= self.max_tags {
                break;
            }
        }
        out
    }
}

impl Tagger for GeminiTagger {
    async fn suggest(&self, input: &TagInput<'_>) -> anyhow::Result<Vec<String>> {
        if self.disabled.load(Ordering::SeqCst) || self.in_cooldown() {
            return Ok(Vec::new());
        }
        self.limiter.until_ready().await;

        let response = self
            .client
            .post(self.url())
            .header("x-goog-api-key", &self.api_key)
            .json(&self.build_request_body(input))
            .send()
            .await;

        let response = match response {
            Ok(resp) => resp,
            // Network/timeout: transient — let the worker back off.
            Err(err) => return Err(anyhow::anyhow!("gemini request failed: {err}")),
        };

        let status = response.status();
        if status.is_success() {
            let body: Value = response.json().await.unwrap_or(Value::Null);
            return Ok(self.parse_response(&body));
        }

        // Classify by HTTP status, not body structure (the error schema keeps
        // changing). Read the body best-effort for daily-quota markers only.
        let body_text = response.text().await.unwrap_or_default();

        if status == StatusCode::TOO_MANY_REQUESTS {
            let daily = body_text.to_ascii_lowercase().contains("perday")
                || body_text.to_ascii_lowercase().contains("per day")
                || body_text.to_ascii_lowercase().contains("daily");
            let cooldown = if daily {
                Duration::from_secs(24 * 3600)
            } else {
                Duration::from_secs(300)
            };
            warn!(daily, "gemini 429; cooling down");
            self.set_cooldown(cooldown);
            return Ok(Vec::new());
        }

        if matches!(status.as_u16(), 400 | 401 | 403 | 404) {
            // Disable for the process lifetime; log exactly once.
            if !self.disabled.swap(true, Ordering::SeqCst) {
                error!(status = status.as_u16(), "gemini disabled after client error");
            }
            return Ok(Vec::new());
        }

        // 5xx and anything else: transient.
        Err(anyhow::anyhow!("gemini http {}", status.as_u16()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn tagger_for(endpoint: &str) -> GeminiTagger {
        GeminiTagger::new(&AiConfig {
            api_key: "test-key".to_owned(),
            endpoint: endpoint.to_owned(),
            max_rpm: 600, // ~0.1s spacing so tests don't stall
            max_tags: 3,
            ..AiConfig::default()
        })
    }

    /// One-shot-per-connection HTTP server (copied shape from
    /// scheduler::tests::spawn_single_response_server, no new dev-dependency).
    /// Loops accepting so we can detect an *unexpected* second request; counts
    /// connections handled.
    async fn spawn_counting_server(status_line: &'static str, body: &'static str) -> (String, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_task = counter.clone();
        // Bind before spawning so we never block the current-thread runtime on
        // a channel the spawned task must fill.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                counter_task.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                loop {
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let response = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (format!("http://{addr}"), counter)
    }

    #[test]
    fn request_body_has_schema_and_no_thinking_budget() {
        let tagger = tagger_for("http://127.0.0.1:1");
        let body = tagger.build_request_body(&TagInput {
            title: "t",
            url: "https://x.test",
            excerpt: "e",
        });
        let serialized = serde_json::to_string(&body).unwrap();
        assert!(serialized.contains("responseMimeType"));
        assert!(serialized.contains("\"enum\""));
        assert!(!serialized.contains("thinkingBudget"));
        assert!(!serialized.contains("thinkingConfig"));
    }

    #[tokio::test]
    async fn success_parses_double_encoded_array() {
        // parts[0].text is itself a JSON string.
        let (url, _c) = spawn_counting_server(
            "HTTP/1.1 200 OK",
            r#"{"candidates":[{"content":{"parts":[{"text":"[\"tech\",\"ai\"]"}]}}]}"#,
        ).await;
        let tagger = tagger_for(&url);
        let tags = tagger
            .suggest(&TagInput { title: "Rust", url: "https://x.test", excerpt: "" })
            .await
            .unwrap();
        assert_eq!(tags, vec!["tech".to_owned(), "ai".to_owned()]);
    }

    #[tokio::test]
    async fn http_400_latches_disabled_and_skips_second_request() {
        let (url, counter) = spawn_counting_server("HTTP/1.1 400 Bad Request", r#"{"error":"bad key"}"#).await;
        let tagger = tagger_for(&url);

        let first = tagger
            .suggest(&TagInput { title: "t", url: "https://x.test", excerpt: "" })
            .await
            .unwrap();
        assert!(first.is_empty());
        assert!(tagger.is_disabled());

        let second = tagger
            .suggest(&TagInput { title: "t", url: "https://x.test", excerpt: "" })
            .await
            .unwrap();
        assert!(second.is_empty());

        // Give any (erroneous) second connection a moment to land.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1, "second call must not hit the network");
    }

    #[tokio::test]
    async fn http_500_is_transient_error() {
        let (url, _c) = spawn_counting_server("HTTP/1.1 500 Internal Server Error", "{}").await;
        let tagger = tagger_for(&url);
        let result = tagger
            .suggest(&TagInput { title: "t", url: "https://x.test", excerpt: "" })
            .await;
        assert!(result.is_err(), "5xx must surface as a transient Err");
    }
}
