//! Article body: Rust fetches the page; the frontend runs Readability and sends the text back
//! (SPEC §8.1, P2 dev spec §3). Also the waiters the daily pick uses to wait for a body.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use regex::Regex;
use serde::Serialize;
use tokio::sync::oneshot;

use crate::error::{AppError, AppResult};
use crate::http::{HttpClient, HttpRequest};

pub const MAX_HTML_BYTES: usize = 3 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FetchedHtml {
    pub html: String,
    pub final_url: String,
    pub canonical_url: Option<String>,
    pub paywall_hint: bool,
}

const PAYWALL_MARKERS: &[&str] = &[
    "\"isaccessibleforfree\":false",
    "\"isaccessibleforfree\": false",
    "\"isaccessibleforfree\":\"false\"",
    "\"isaccessibleforfree\": \"false\"",
    "class=\"paywall",
    "subscribe to continue",
    "subscribers only",
    "create a free account to continue",
    "this article is for subscribers",
];

pub fn paywall_hint(html: &str) -> bool {
    let lower = html.to_lowercase();
    PAYWALL_MARKERS.iter().any(|m| lower.contains(m))
}

static CANONICAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<link\b[^>]*\brel\s*=\s*["']?canonical["']?[^>]*>"#).unwrap());
static HREF_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\bhref\s*=\s*["']([^"']+)["']"#).unwrap());

pub fn canonical_url(html: &str, base: &str) -> Option<String> {
    let head_end = html.to_lowercase().find("</head>").unwrap_or(html.len().min(200_000));
    let tag = CANONICAL_RE.find(&html[..head_end])?.as_str();
    let href = HREF_RE.captures(tag)?.get(1)?.as_str();
    let base = url::Url::parse(base).ok()?;
    let u = base.join(href).ok()?;
    matches!(u.scheme(), "http" | "https").then(|| u.to_string())
}

pub async fn fetch_html(http: &dyn HttpClient, url: &str) -> AppResult<FetchedHtml> {
    let mut req = HttpRequest::get(url).header("Accept", "text/html,application/xhtml+xml");
    req.max_bytes = MAX_HTML_BYTES;
    let resp = http.get(req).await?;
    if !(200..300).contains(&resp.status) {
        return Err(AppError::Network(format!("HTTP {}", resp.status)));
    }
    // Bot checks (e.g. AWS WAF on Towards Data Science) answer 202 with an empty page.
    if resp.status == 202 || resp.header("x-amzn-waf-action").is_some() || resp.body.trim_ascii().is_empty() {
        return Err(AppError::Network("The site blocked the app (bot check)".into()));
    }
    let ct = resp.header("content-type").unwrap_or("").to_ascii_lowercase();
    if !ct.is_empty() && !ct.contains("html") {
        return Err(AppError::Invalid("not an HTML page".into()));
    }
    let html = String::from_utf8_lossy(&resp.body).into_owned();
    Ok(FetchedHtml {
        canonical_url: canonical_url(&html, &resp.final_url),
        paywall_hint: paywall_hint(&html),
        final_url: resp.final_url,
        html,
    })
}

/// Body status rules (P2 dev spec §3.3).
pub fn body_status(word_count: u32, description_len: usize, paywall_hint: bool, failed: bool) -> &'static str {
    if failed || word_count < 80 {
        "failed"
    } else if (paywall_hint && word_count < 400) || (word_count < 150 && description_len > 200) {
        "paywalled"
    } else {
        "ok"
    }
}

/// The daily pick waits here until the frontend has extracted a body.
#[derive(Default)]
pub struct BodyWaiters {
    waiting: Mutex<HashMap<i64, Vec<oneshot::Sender<String>>>>,
}

impl BodyWaiters {
    pub fn register(&self, article_id: i64) -> oneshot::Receiver<String> {
        let (tx, rx) = oneshot::channel();
        self.waiting.lock().unwrap().entry(article_id).or_default().push(tx);
        rx
    }

    pub fn wake(&self, article_id: i64, status: &str) {
        if let Some(list) = self.waiting.lock().unwrap().remove(&article_id) {
            for tx in list {
                let _ = tx.send(status.to_string());
            }
        }
    }

    /// Wait for a status; `None` on timeout.
    pub async fn wait(rx: oneshot::Receiver<String>, timeout: Duration) -> Option<String> {
        tokio::time::timeout(timeout, rx).await.ok().and_then(Result::ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::ReqwestClient;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn status_rules() {
        assert_eq!(body_status(1000, 100, false, false), "ok");
        assert_eq!(body_status(50, 100, false, false), "failed");
        assert_eq!(body_status(1000, 100, false, true), "failed");
        assert_eq!(body_status(300, 100, true, false), "paywalled");
        assert_eq!(body_status(500, 100, true, false), "ok", "long enough despite a marker");
        assert_eq!(body_status(120, 300, false, false), "paywalled");
    }

    #[test]
    fn finds_canonical_and_paywall() {
        let html = r#"<html><head><link rel="canonical" href="/posts/x"></head><body>
            <script type="application/ld+json">{"isAccessibleForFree": false}</script></body></html>"#;
        assert_eq!(
            canonical_url(html, "https://ex.com/a/b?utm=1").as_deref(),
            Some("https://ex.com/posts/x")
        );
        assert!(paywall_hint(html));
        assert!(!paywall_hint("<p>free text</p>"));
        assert_eq!(canonical_url("<head></head>", "https://ex.com"), None);
    }

    #[tokio::test]
    async fn fetch_rules() {
        let s = MockServer::start().await;
        Mock::given(path("/ok"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw("<html><head></head><body>hi</body></html>", "text/html; charset=utf-8"),
            )
            .mount(&s)
            .await;
        Mock::given(path("/pdf"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(vec![1u8, 2], "application/pdf"))
            .mount(&s)
            .await;
        Mock::given(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(vec![b'a'; MAX_HTML_BYTES + 10], "text/html"))
            .mount(&s)
            .await;
        Mock::given(path("/challenge"))
            .respond_with(ResponseTemplate::new(202).insert_header("x-amzn-waf-action", "challenge"))
            .mount(&s)
            .await;
        Mock::given(path("/moved"))
            .respond_with(ResponseTemplate::new(301).insert_header("location", "/ok"))
            .mount(&s)
            .await;
        let http = ReqwestClient::new().unwrap();
        let ok = fetch_html(&http, &format!("{}/ok", s.uri())).await.unwrap();
        assert!(ok.html.contains("hi"));
        assert!(
            fetch_html(&http, &format!("{}/pdf", s.uri()))
                .await
                .unwrap_err()
                .to_string()
                .contains("not an HTML")
        );
        assert!(fetch_html(&http, &format!("{}/big", s.uri())).await.is_err());
        assert!(
            fetch_html(&http, &format!("{}/challenge", s.uri()))
                .await
                .unwrap_err()
                .to_string()
                .contains("bot check")
        );
        let moved = fetch_html(&http, &format!("{}/moved", s.uri())).await.unwrap();
        assert!(moved.final_url.ends_with("/ok"));
    }

    #[tokio::test]
    async fn waiters_wake_and_time_out() {
        let w = BodyWaiters::default();
        let rx = w.register(7);
        w.wake(7, "ok");
        assert_eq!(
            BodyWaiters::wait(rx, Duration::from_millis(50)).await.as_deref(),
            Some("ok")
        );
        let rx = w.register(8);
        assert_eq!(BodyWaiters::wait(rx, Duration::from_millis(20)).await, None);
    }
}
