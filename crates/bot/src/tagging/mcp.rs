//! MCP (Model Context Protocol) client for a pi-mcp-bridge endpoint, plus the
//! `McpTagger` built on it.
//!
//! The bridge is a **stateless Streamable-HTTP** MCP server with
//! `enableJsonResponse` — so each call is a plain JSON-RPC `POST` returning
//! `application/json` (no `initialize` handshake, no session id, no SSE). We
//! use the async job tools (`pi_run_async` + `pi_result` polling) so a slow
//! agent turn doesn't hit a proxy's request timeout (the README notes ~100s on
//! Cloudflare's free plan).
//!
//! Verified against pi-mcp-bridge `src/index.ts` @ 2026-08-06; confirm the live
//! JSON shape with one real `curl` before trusting in production.

use std::time::{Duration, Instant};

use reqwest::Client;
use serde_json::{json, Value};

use crate::config::McpConfig;

use super::taxonomy;
use super::{TagInput, Tagger};

/// Thin JSON-RPC-over-HTTP client for the bridge's tools.
pub struct McpClient {
    http: Client,
    cfg: McpConfig,
}

impl McpClient {
    pub fn new(http: Client, cfg: McpConfig) -> Self {
        Self { http, cfg }
    }

    /// Builds its own HTTP client (used where no shared client is at hand, e.g.
    /// the worker's tagger).
    pub fn from_config(cfg: McpConfig) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self::new(http, cfg)
    }

    /// One JSON-RPC `tools/call`. Returns the tool's `content[0].text`, or `Err`
    /// on HTTP failure, a JSON-RPC `error`, or a tool result with `isError`.
    async fn call_tool(&self, name: &str, arguments: Value) -> anyhow::Result<String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        let mut req = self
            .http
            .post(&self.cfg.endpoint)
            .header("authorization", format!("Bearer {}", self.cfg.token))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .json(&body);
        if !self.cfg.cf_access_client_id.is_empty() {
            req = req
                .header("CF-Access-Client-Id", &self.cfg.cf_access_client_id)
                .header("CF-Access-Client-Secret", &self.cfg.cf_access_client_secret);
        }

        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("mcp http {}: {}", status.as_u16(), truncate(&text));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("mcp bad json ({e}): {}", truncate(&text)))?;
        if let Some(err) = v.get("error") {
            anyhow::bail!("mcp jsonrpc error: {err}");
        }
        let result = &v["result"];
        let content_text = result["content"][0]["text"].as_str().map(str::to_owned);
        if result.get("isError").and_then(Value::as_bool).unwrap_or(false) {
            anyhow::bail!("mcp tool error: {}", content_text.unwrap_or_default());
        }
        content_text.ok_or_else(|| anyhow::anyhow!("mcp: no text content"))
    }

    /// Runs a prompt on the remote agent and returns its final text. Enqueues
    /// via `pi_run_async`, then polls `pi_result` until terminal or the overall
    /// deadline (best-effort `pi_abort` on timeout).
    pub async fn run(&self, prompt: &str) -> anyhow::Result<String> {
        let deadline = Instant::now() + Duration::from_secs(self.cfg.timeout_seconds.max(1));
        let start = self
            .call_tool(
                "pi_run_async",
                json!({ "prompt": prompt, "timeout_seconds": self.cfg.timeout_seconds }),
            )
            .await?;
        let job_id = serde_json::from_str::<Value>(&start)
            .ok()
            .and_then(|v| v.get("job_id").and_then(Value::as_str).map(str::to_owned))
            .ok_or_else(|| anyhow::anyhow!("mcp: no job_id in {}", truncate(&start)))?;

        let poll = Duration::from_millis(self.cfg.poll_interval_ms.max(200));
        loop {
            let res = self.call_tool("pi_result", json!({ "job_id": job_id })).await?;
            let parsed: Value = serde_json::from_str(&res).unwrap_or(Value::Null);
            match parsed.get("status").and_then(Value::as_str).unwrap_or("") {
                "done" => {
                    return Ok(parsed
                        .get("result")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned())
                }
                "error" | "aborted" => {
                    let err = parsed.get("error").and_then(Value::as_str).unwrap_or("unknown");
                    anyhow::bail!("mcp job {job_id} failed: {err}");
                }
                _ => {} // queued | running
            }
            if Instant::now() >= deadline {
                let _ = self.call_tool("pi_abort", json!({ "job_id": job_id })).await;
                anyhow::bail!("mcp job {job_id} timed out");
            }
            tokio::time::sleep(poll).await;
        }
    }
}

fn truncate(s: &str) -> String {
    s.chars().take(200).collect()
}

/// Extracts a JSON array of tag slugs from possibly-chatty agent output
/// (tolerates ```json fences and surrounding prose), then canonicalizes.
fn extract_slugs(text: &str, max_tags: usize) -> Vec<String> {
    let direct: Option<Vec<String>> = serde_json::from_str(text.trim()).ok();
    let raw = direct.or_else(|| {
        let start = text.find('[')?;
        let end = text.rfind(']')?;
        if end < start {
            return None;
        }
        serde_json::from_str(&text[start..=end]).ok()
    });
    let mut out = Vec::new();
    for slug in raw.unwrap_or_default() {
        if let Some(canonical) = taxonomy::normalize(&slug) {
            if !out.iter().any(|s| s == canonical) {
                out.push(canonical.to_owned());
            }
        }
        if out.len() >= max_tags {
            break;
        }
    }
    out
}

/// Tagger that delegates classification to the remote agent, constrained to the
/// fixed taxonomy. Returns `Ok(vec![])` when nothing usable comes back, `Err`
/// on transport failure — matching the worker's fallback contract.
pub struct McpTagger {
    client: McpClient,
    max_tags: usize,
}

impl McpTagger {
    pub fn new(client: McpClient, max_tags: usize) -> Self {
        Self { client, max_tags: max_tags.max(1) }
    }

    fn prompt(&self, input: &TagInput<'_>) -> String {
        let slugs: Vec<&str> = taxonomy::all_slugs().collect();
        format!(
            "Classify this bookmark into 1 to {max} of these categories, choosing the most \
             specific. Reply ONLY with a JSON array of category slugs from the list — no prose.\n\
             Categories: {cats}\n\nTitle: {title}\nURL: {url}\nExcerpt: {excerpt}",
            max = self.max_tags,
            cats = slugs.join(", "),
            title = input.title,
            url = input.url,
            excerpt = input.excerpt,
        )
    }
}

impl Tagger for McpTagger {
    async fn suggest(&self, input: &TagInput<'_>) -> anyhow::Result<Vec<String>> {
        let text = self.client.run(&self.prompt(input)).await?;
        Ok(extract_slugs(&text, self.max_tags))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn client_for(endpoint: &str) -> McpClient {
        McpClient::from_config(McpConfig {
            endpoint: endpoint.to_owned(),
            token: "t".to_owned(),
            poll_interval_ms: 200,
            timeout_seconds: 5,
            ..McpConfig::default()
        })
    }

    /// Scripted bridge: `pi_run_async` → a queued job, `pi_result` → the given
    /// terminal payload. Loops accepting; binds before spawning.
    async fn spawn_bridge(result_payload: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                // Read headers + body (small requests; Content-Length framed).
                loop {
                    let n = socket.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    let s = String::from_utf8_lossy(&buf);
                    if let Some(hdr_end) = s.find("\r\n\r\n") {
                        let len = s
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if buf.len() >= hdr_end + 4 + len {
                            break;
                        }
                    }
                }
                let req = String::from_utf8_lossy(&buf).to_string();
                let content_text = if req.contains("pi_run_async") {
                    r#"{"job_id":"j1","status":"queued"}"#.to_owned()
                } else if req.contains("pi_result") {
                    result_payload.to_owned()
                } else {
                    "ok".to_owned()
                };
                let envelope = json!({
                    "jsonrpc": "2.0", "id": 1,
                    "result": { "content": [{ "type": "text", "text": content_text }] }
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    envelope.len(),
                    envelope
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        format!("http://{addr}/mcp")
    }

    #[tokio::test]
    async fn run_returns_done_result() {
        let url = spawn_bridge(r#"{"status":"done","result":"[\"tech\",\"ai\"]"}"#).await;
        let out = client_for(&url).run("hi").await.unwrap();
        assert_eq!(out, "[\"tech\",\"ai\"]");
    }

    #[tokio::test]
    async fn run_surfaces_error_status() {
        let url = spawn_bridge(r#"{"status":"error","error":"boom"}"#).await;
        let err = client_for(&url).run("hi").await.unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[tokio::test]
    async fn tagger_parses_slugs_from_done_result() {
        let url = spawn_bridge(r#"{"status":"done","result":"[\"tech\",\"programming\",\"not-a-real-tag\"]"}"#).await;
        let tagger = McpTagger::new(client_for(&url), 3);
        let tags = tagger
            .suggest(&TagInput { title: "Rust", url: "https://x.test", excerpt: "" })
            .await
            .unwrap();
        assert_eq!(tags, vec!["tech".to_owned(), "programming".to_owned()]);
    }

    #[test]
    fn extract_slugs_tolerates_fences_and_prose() {
        let out = extract_slugs("Sure! ```json\n[\"tech\", \"ai\"]\n```", 3);
        assert_eq!(out, vec!["tech".to_owned(), "ai".to_owned()]);
        assert!(extract_slugs("no array here", 3).is_empty());
    }
}
