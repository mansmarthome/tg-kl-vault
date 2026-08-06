//! URL normalization: produces the canonical string used both as the display
//! URL and as the `(chat_id, url)` dedupe key.

use url::Url;

const MAX_URL_LEN: usize = 2048;

/// Tracking query params stripped on normalization. Deliberately conservative:
/// this list does **not** include `ref` or `si`, which carry real meaning on
/// some sites (YouTube `si`, referral `ref`).
fn is_tracking_param(key: &str) -> bool {
    key.starts_with("utm_")
        || matches!(
            key,
            "gclid"
                | "fbclid"
                | "msclkid"
                | "yclid"
                | "mc_cid"
                | "mc_eid"
                | "igshid"
                | "_hsenc"
                | "_hsmi"
        )
}

/// Normalizes a raw URL string. Returns `Err` for anything that isn't a valid
/// http/https URL.
///
/// Rejecting non-http(s) schemes is a **security control**, not tidiness: a
/// stored `javascript:` URL would land verbatim inside an `href="…"`.
///
/// Deliberately does NOT: sort query params, lowercase the path, strip a
/// trailing slash, or strip `www.` — each of those breaks real URLs. As a
/// documented consequence, `www.x.com/a` and `x.com/a` are two bookmarks.
pub fn normalize_url(raw: &str) -> anyhow::Result<String> {
    let trimmed = raw.trim().trim_start_matches('<').trim_end_matches('>').trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty url");
    }
    if trimmed.len() > MAX_URL_LEN {
        anyhow::bail!("url too long");
    }

    let mut url = match Url::parse(trimmed) {
        Ok(url) => url,
        Err(url::ParseError::RelativeUrlWithoutBase) => Url::parse(&format!("https://{trimmed}"))?,
        Err(err) => return Err(err.into()),
    };

    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("unsupported url scheme: {}", url.scheme());
    }

    url.set_fragment(None);

    // Drop an explicit default port (`:80`/`:443`). `port()` is `Some` only
    // when the port is explicitly present.
    if let Some(port) = url.port() {
        let default = match url.scheme() {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
        };
        if Some(port) == default {
            let _ = url.set_port(None);
        }
    }

    strip_tracking_params(&mut url);

    Ok(url.to_string())
}

fn strip_tracking_params(url: &mut Url) {
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let kept: Vec<(String, String)> = pairs
        .iter()
        .filter(|(k, _)| !is_tracking_param(k))
        .cloned()
        .collect();
    if kept.len() == pairs.len() {
        return; // nothing to strip; don't touch (avoids re-encoding).
    }
    if kept.is_empty() {
        url.set_query(None);
        return;
    }
    let mut qp = url.query_pairs_mut();
    qp.clear();
    for (k, v) in &kept {
        qp.append_pair(k, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_https_scheme_when_missing() {
        assert_eq!(normalize_url("example.com/a").unwrap(), "https://example.com/a");
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(normalize_url("javascript:alert(1)").is_err());
        assert!(normalize_url("ftp://x.test/a").is_err());
        assert!(normalize_url("mailto:a@b.test").is_err());
    }

    #[test]
    fn strips_fragment_and_default_port() {
        assert_eq!(
            normalize_url("https://x.test:443/a#frag").unwrap(),
            "https://x.test/a"
        );
        assert_eq!(
            normalize_url("http://x.test:80/a").unwrap(),
            "http://x.test/a"
        );
    }

    #[test]
    fn strips_tracking_but_keeps_ref_and_si() {
        assert_eq!(
            normalize_url("https://x.test/a?utm_source=n&id=5&fbclid=zzz").unwrap(),
            "https://x.test/a?id=5"
        );
        // ref and si survive.
        let out = normalize_url("https://x.test/a?ref=hn&si=abc").unwrap();
        assert!(out.contains("ref=hn"));
        assert!(out.contains("si=abc"));
        // All-tracking query collapses to no query.
        assert_eq!(
            normalize_url("https://x.test/a?utm_source=n&gclid=z").unwrap(),
            "https://x.test/a"
        );
    }

    #[test]
    fn does_not_strip_www_or_trailing_slash() {
        assert_eq!(normalize_url("https://www.x.test/").unwrap(), "https://www.x.test/");
        // Distinct from the www-less form (documented consequence).
        assert_ne!(
            normalize_url("https://www.x.test/a").unwrap(),
            normalize_url("https://x.test/a").unwrap()
        );
    }

    #[test]
    fn rejects_overlong_and_empty() {
        assert!(normalize_url("").is_err());
        assert!(normalize_url(&format!("https://x.test/{}", "a".repeat(3000))).is_err());
    }

    #[test]
    fn strips_angle_brackets() {
        assert_eq!(normalize_url("<https://x.test/a>").unwrap(), "https://x.test/a");
    }
}
