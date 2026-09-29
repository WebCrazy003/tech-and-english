//! RSS 2.0 / Atom / JSON Feed via feed-rs, with conditional GET.

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::model::{FetchResult, RawItem};
use super::normalize::{strip_html, truncate_chars};
use super::source::FetchCtx;
use crate::clock::fmt_ts;
use crate::error::{AppError, AppResult};
use crate::http::{HttpClient, HttpRequest, HttpResponse};

fn looks_like_html(body: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&body[..body.len().min(512)]).to_ascii_lowercase();
    head.contains("<!doctype html") || head.contains("<html")
}

fn parse_feed(url: &str, resp: &HttpResponse) -> AppResult<feed_rs::model::Feed> {
    feed_rs::parser::Builder::new()
        .base_uri(Some(url))
        .build()
        .parse(&resp.body[..])
        .map_err(|_| {
            if looks_like_html(&resp.body) {
                AppError::Invalid("Not a feed (this is a web page)".into())
            } else {
                AppError::Invalid("Could not read this feed".into())
            }
        })
}

fn check_status(resp: &HttpResponse) -> AppResult<()> {
    if (200..300).contains(&resp.status) {
        Ok(())
    } else {
        Err(AppError::Network(format!("HTTP {}", resp.status)))
    }
}

fn entry_to_item(e: feed_rs::model::Entry) -> Option<RawItem> {
    let url = e
        .links
        .iter()
        .find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
        .or_else(|| e.links.first())
        .map(|l| l.href.clone())
        .or_else(|| e.id.starts_with("http").then(|| e.id.clone()))?;
    let title = strip_html(&e.title.as_ref()?.content);
    if title.is_empty() {
        return None;
    }
    let description = e
        .summary
        .as_ref()
        .map(|s| s.content.clone())
        .or_else(|| e.content.as_ref().and_then(|c| c.body.clone()))
        .map(|d| truncate_chars(&strip_html(&d), 1000))
        .filter(|d| !d.is_empty());
    Some(RawItem {
        url,
        title,
        description,
        author: e.authors.first().and_then(|p| p.name.clone()),
        published_at: e.published.or(e.updated),
        hn: None,
    })
}

pub async fn fetch(ctx: &FetchCtx, url: &str) -> AppResult<FetchResult> {
    let mut req = HttpRequest::get(url);
    if let Some(etag) = &ctx.etag {
        req = req.header("If-None-Match", etag.clone());
    }
    if let Some(lm) = &ctx.last_modified {
        req = req.header("If-Modified-Since", lm.clone());
    }
    let resp = ctx.http.get(req).await?;
    if resp.status == 304 {
        return Ok(FetchResult {
            not_modified: true,
            ..Default::default()
        });
    }
    check_status(&resp)?;
    let feed = parse_feed(url, &resp)?;
    Ok(FetchResult {
        items: feed.entries.into_iter().filter_map(entry_to_item).collect(),
        etag: resp.header("etag").map(str::to_string),
        last_modified: resp.header("last-modified").map(str::to_string),
        not_modified: false,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedTestResult {
    pub title: Option<String>,
    pub item_count: usize,
    pub newest_published_at: Option<String>,
}

/// Fetch and parse without saving, for the "Test feed" button.
pub async fn test_feed(http: &dyn HttpClient, url: &str) -> AppResult<FeedTestResult> {
    let resp = http.get(HttpRequest::get(url)).await?;
    check_status(&resp)?;
    let feed = parse_feed(url, &resp)?;
    let newest: Option<DateTime<Utc>> = feed.entries.iter().filter_map(|e| e.published.or(e.updated)).max();
    Ok(FeedTestResult {
        title: feed.title.map(|t| strip_html(&t.content)),
        item_count: feed.entries.len(),
        newest_published_at: newest.map(fmt_ts),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::ReqwestClient;
    use std::collections::HashSet;
    use std::sync::Arc;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const RSS: &str = include_str!("../../tests/fixtures/rss2_basic.xml");
    const ATOM: &str = include_str!("../../tests/fixtures/atom_basic.xml");

    fn ctx() -> FetchCtx {
        FetchCtx {
            http: Arc::new(ReqwestClient::new().unwrap()),
            etag: None,
            last_modified: None,
            skip_hn_ids: Arc::new(HashSet::new()),
            hn_include_new: false,
            hn_base: String::new(),
        }
    }

    #[tokio::test]
    async fn parses_rss_and_keeps_etag() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/feed"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(RSS)
                    .insert_header("ETag", "\"v1\""),
            )
            .mount(&s)
            .await;
        let r = fetch(&ctx(), &format!("{}/feed", s.uri())).await.unwrap();
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.etag.as_deref(), Some("\"v1\""));
        let first = &r.items[0];
        assert_eq!(first.title, "New local AI model runs on Apple Silicon");
        assert_eq!(first.description.as_deref(), Some("A small model & a fast runtime."));
        assert!(first.published_at.is_some());
    }

    #[tokio::test]
    async fn parses_atom_with_relative_links() {
        let s = MockServer::start().await;
        Mock::given(path("/atom"))
            .respond_with(ResponseTemplate::new(200).set_body_string(ATOM))
            .mount(&s)
            .await;
        let r = fetch(&ctx(), &format!("{}/atom", s.uri())).await.unwrap();
        assert_eq!(r.items.len(), 1);
        assert!(
            r.items[0].url.starts_with("http"),
            "relative link resolved: {}",
            r.items[0].url
        );
    }

    #[tokio::test]
    async fn not_modified_with_etag() {
        let s = MockServer::start().await;
        Mock::given(path("/feed"))
            .and(header("If-None-Match", "\"v1\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&s)
            .await;
        let mut c = ctx();
        c.etag = Some("\"v1\"".into());
        let r = fetch(&c, &format!("{}/feed", s.uri())).await.unwrap();
        assert!(r.not_modified);
    }

    #[tokio::test]
    async fn errors_are_readable() {
        let s = MockServer::start().await;
        Mock::given(path("/404"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&s)
            .await;
        Mock::given(path("/html"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<!DOCTYPE html><html><body>hi</body></html>"))
            .mount(&s)
            .await;
        Mock::given(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'a'; 6 * 1024 * 1024]))
            .mount(&s)
            .await;
        let http = ReqwestClient::new().unwrap();
        let e404 = test_feed(&http, &format!("{}/404", s.uri())).await.unwrap_err();
        assert_eq!(e404.to_string(), "HTTP 404");
        let ehtml = test_feed(&http, &format!("{}/html", s.uri())).await.unwrap_err();
        assert!(ehtml.to_string().contains("web page"));
        let ebig = test_feed(&http, &format!("{}/big", s.uri())).await.unwrap_err();
        assert!(ebig.to_string().contains("too large"));
    }

    #[tokio::test]
    async fn times_out() {
        let s = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(3)))
            .mount(&s)
            .await;
        let http = ReqwestClient::new().unwrap();
        let mut req = HttpRequest::get(format!("{}/slow", s.uri()));
        req.timeout = std::time::Duration::from_millis(200);
        let e = http.get(req).await.unwrap_err();
        assert_eq!(e.to_string(), "Timed out");
    }

    #[tokio::test]
    async fn test_feed_reports_title_and_count() {
        let s = MockServer::start().await;
        Mock::given(path("/feed"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RSS))
            .mount(&s)
            .await;
        let r = test_feed(&ReqwestClient::new().unwrap(), &format!("{}/feed", s.uri()))
            .await
            .unwrap();
        assert_eq!(r.title.as_deref(), Some("Example Tech"));
        assert_eq!(r.item_count, 2);
        assert!(r.newest_published_at.is_some());
    }
}
