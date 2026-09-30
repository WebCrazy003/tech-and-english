//! Add a source from an example URL: find that site's own feed (SPEC §7.12).

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::Duration;

use futures::{StreamExt, stream};
use regex::Regex;
use serde::Serialize;
use url::Url;

use super::body::MAX_HTML_BYTES;
use super::normalize::{host_of, normalize_url, strip_html, truncate_chars};
use super::rss::{self, FeedTestResult};
use crate::clock::fmt_ts;
use crate::error::{AppError, AppResult};
use crate::http::{HttpClient, HttpRequest};

pub const PAGE_TIMEOUT: Duration = Duration::from_secs(15);
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
pub const MAX_PROBES: usize = 8;
const MAX_LINKS: usize = 6;
const PARALLEL: usize = 4;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FeedCandidate {
    pub url: String,
    pub title: Option<String>,
    pub item_count: usize,
    pub newest_published_at: Option<String>,
    pub already_added: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExamplePage {
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<String>,
    pub site_name: String,
    /// The pasted URL is a feed itself, not an article page.
    pub is_feed: bool,
    /// The page could be read, so it can be saved as an article.
    pub can_save: bool,
}

/// Result of `discover_feeds`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub page: ExamplePage,
    pub candidates: Vec<FeedCandidate>,
}

/// What the page's HTML tells us.
#[derive(Debug, Clone, PartialEq)]
pub struct PageInfo {
    pub page: ExamplePage,
    /// `<link rel="alternate">` feed URLs, resolved.
    pub feed_links: Vec<String>,
    pub platform: Option<Platform>,
    pub substack: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    WordPress,
    Ghost,
    Hugo,
    Jekyll,
}

static LINK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<link\b[^>]*>").unwrap());
static META_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<meta\b[^>]*>").unwrap());
static TITLE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<title\b[^>]*>(.*?)</title>").unwrap());
static JSONLD_DATE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""datePublished"\s*:\s*"([^"]+)""#).unwrap());
static ATTR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)([a-z_:-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))"#).unwrap());

const FEED_TYPES: &[&str] = &[
    "application/rss+xml",
    "application/atom+xml",
    "application/feed+json",
    "application/rdf+xml",
];

fn attrs(tag: &str) -> HashMap<String, String> {
    ATTR_RE
        .captures_iter(tag)
        .map(|c| {
            let v = c.get(2).or(c.get(3)).or(c.get(4)).map(|m| m.as_str()).unwrap_or("");
            (c[1].to_ascii_lowercase(), strip_html(v))
        })
        .collect()
}

/// Comment feeds and code-commit feeds are not article feeds.
fn is_article_feed(href: &str, title: Option<&str>) -> bool {
    let h = href.to_ascii_lowercase();
    let t = title.unwrap_or("").to_ascii_lowercase();
    !(h.contains("/comments/") || h.ends_with("/comments") || t.contains("comments") || h.contains("/commits/"))
}

fn parse_date(s: &str) -> Option<String> {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s.trim()) {
        return Some(fmt_ts(d.with_timezone(&chrono::Utc)));
    }
    let day = s.trim().get(..10)?;
    let d = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    Some(fmt_ts(d.and_hms_opt(0, 0, 0)?.and_utc()))
}

/// Read the `<head>` with regexes (no full HTML parser needed).
pub fn parse_page(html: &str, base: &Url) -> PageInfo {
    let lower = html.to_ascii_lowercase();
    let head_end = lower.find("</head>").unwrap_or(html.len().min(300_000));
    let head = html.get(..head_end).unwrap_or(html);

    let mut feed_links = Vec::new();
    for m in LINK_RE.find_iter(head) {
        let a = attrs(m.as_str());
        let rel = a.get("rel").map(|r| r.to_ascii_lowercase()).unwrap_or_default();
        let ty = a.get("type").map(|t| t.to_ascii_lowercase()).unwrap_or_default();
        if !rel.split_whitespace().any(|r| r == "alternate") || !FEED_TYPES.contains(&ty.as_str()) {
            continue;
        }
        let Some(href) = a.get("href").filter(|h| !h.is_empty()) else {
            continue;
        };
        let Ok(u) = base.join(href) else { continue };
        let u = u.to_string();
        if is_article_feed(&u, a.get("title").map(String::as_str)) && !feed_links.contains(&u) {
            feed_links.push(u);
        }
    }

    let mut meta: HashMap<String, String> = HashMap::new();
    for m in META_RE.find_iter(head) {
        let a = attrs(m.as_str());
        let key = a
            .get("property")
            .or_else(|| a.get("name"))
            .map(|k| k.to_ascii_lowercase());
        if let (Some(k), Some(v)) = (key, a.get("content")) {
            meta.entry(k).or_insert_with(|| v.clone());
        }
    }
    let get = |k: &str| meta.get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());

    let host = host_of(base.as_str()).unwrap_or_else(|| base.as_str().to_string());
    let title = get("og:title")
        .or_else(|| get("twitter:title"))
        .or_else(|| {
            TITLE_RE
                .captures(head)
                .map(|c| strip_html(&c[1]))
                .filter(|t| !t.is_empty())
        })
        .unwrap_or_else(|| host.clone());
    let description = get("og:description")
        .or_else(|| get("description"))
        .map(|d| truncate_chars(&d, 1000));
    let published_at = get("article:published_time")
        .or_else(|| get("datepublished"))
        .or_else(|| JSONLD_DATE_RE.captures(html).map(|c| c[1].to_string()))
        .and_then(|d| parse_date(&d));
    let generator = get("generator").unwrap_or_default().to_ascii_lowercase();
    let platform = if generator.contains("wordpress") || lower.contains("/wp-content/") {
        Some(Platform::WordPress)
    } else if generator.starts_with("ghost") {
        Some(Platform::Ghost)
    } else if generator.starts_with("hugo") {
        Some(Platform::Hugo)
    } else if generator.starts_with("jekyll") {
        Some(Platform::Jekyll)
    } else {
        None
    };
    PageInfo {
        page: ExamplePage {
            url: base.to_string(),
            title: truncate_chars(&title, 300),
            description,
            published_at,
            site_name: get("og:site_name").unwrap_or(host.clone()),
            is_feed: false,
            can_save: true,
        },
        feed_links: feed_links.into_iter().take(MAX_LINKS).collect(),
        platform,
        substack: host.ends_with(".substack.com") || lower.contains("substackcdn.com"),
    }
}

/// `medium.com/@user/post` → `medium.com/feed/@user`; `medium.com/<pub>/post` → `medium.com/feed/<pub>`;
/// `user.medium.com/post` → `medium.com/feed/@user`.
pub fn medium_feed(u: &Url) -> Option<String> {
    let host = u.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if let Some(user) = host.strip_suffix(".medium.com") {
        return Some(format!("https://medium.com/feed/@{user}"));
    }
    if host != "medium.com" {
        return None;
    }
    let first = u.path_segments()?.find(|s| !s.is_empty())?;
    if first == "feed" {
        return None;
    }
    Some(format!("https://medium.com/feed/{first}"))
}

/// Feed URLs to try when the page has no `<link rel="alternate">`, most likely first. At most 8.
pub fn probe_urls(base: &Url, info: &PageInfo) -> Vec<String> {
    let origin = base.origin().ascii_serialization();
    let segments: Vec<&str> = base
        .path_segments()
        .map(|s| s.filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    // The article's parent path, e.g. "/blog" for "/blog/my-post".
    let parent = (segments.len() >= 2).then(|| format!("/{}", segments[..segments.len() - 1].join("/")));

    let mut out: Vec<String> = Vec::new();
    if info.substack {
        out.push(format!("{origin}/feed"));
    }
    if let Some(m) = medium_feed(base) {
        out.push(m);
    }
    let hint = match info.platform {
        Some(Platform::WordPress) => Some("/feed/"),
        Some(Platform::Ghost) => Some("/rss/"),
        Some(Platform::Hugo) => Some("/index.xml"),
        Some(Platform::Jekyll) => Some("/feed.xml"),
        None => None,
    };
    if let Some(h) = hint {
        out.push(format!("{origin}{h}"));
        if let Some(p) = &parent {
            out.push(format!("{origin}{p}{h}"));
        }
    }
    let p = parent.as_deref();
    let generic = [
        Some("/feed".to_string()),
        Some("/rss.xml".to_string()),
        p.map(|p| format!("{p}/feed")),
        Some("/atom.xml".to_string()),
        Some("/index.xml".to_string()),
        p.map(|p| format!("{p}/rss.xml")),
        p.map(|p| format!("{p}/index.xml")),
        p.map(|p| format!("{p}/atom.xml")),
        Some("/feed.xml".to_string()),
    ];
    out.extend(generic.into_iter().flatten().map(|path| format!("{origin}{path}")));

    let mut seen = HashSet::new();
    out.into_iter()
        .filter(|u| seen.insert(normalize_url(u).unwrap_or_else(|_| u.clone())))
        .take(MAX_PROBES)
        .collect()
}

pub enum Fetched {
    Page(PageInfo),
    Feed(FeedTestResult, String),
}

fn check_url(url: &str) -> AppResult<Url> {
    let u = Url::parse(url.trim()).map_err(|_| AppError::Invalid("That is not a valid URL".into()))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(AppError::Invalid("The URL must start with http:// or https://".into()));
    }
    Ok(u)
}

/// GET the example page (15 s, 3 MB). A feed URL is accepted too.
pub async fn fetch_page(http: &dyn HttpClient, url: &str) -> AppResult<Fetched> {
    let u = check_url(url)?;
    let mut req = HttpRequest::get(u.as_str()).header("Accept", "text/html,application/xhtml+xml");
    req.timeout = PAGE_TIMEOUT;
    req.max_bytes = MAX_HTML_BYTES;
    let resp = http.get(req).await?;
    if !(200..300).contains(&resp.status) {
        return Err(AppError::Network(format!("HTTP {}", resp.status)));
    }
    let ct = resp.header("content-type").unwrap_or("").to_ascii_lowercase();
    let base = Url::parse(&resp.final_url).unwrap_or(u);
    if ct.contains("xml") || ct.contains("rss") || ct.contains("atom") || ct.contains("json") {
        let feed = rss::test_feed(http, base.as_str()).await?;
        return Ok(Fetched::Feed(feed, base.to_string()));
    }
    if !ct.is_empty() && !ct.contains("html") {
        return Err(AppError::Invalid("This link is not a web page".into()));
    }
    Ok(Fetched::Page(parse_page(&String::from_utf8_lossy(&resp.body), &base)))
}

/// Validate candidate URLs (4 at a time, 8 s each); keep working feeds in input order.
async fn validate(http: &dyn HttpClient, urls: Vec<String>) -> Vec<(String, FeedTestResult)> {
    let mut results: Vec<(usize, String, FeedTestResult)> = stream::iter(urls.into_iter().enumerate())
        .map(|(i, u)| async move {
            match tokio::time::timeout(PROBE_TIMEOUT, rss::test_feed(http, &u)).await {
                Ok(Ok(r)) if r.item_count > 0 => Some((i, u, r)),
                _ => None,
            }
        })
        .buffer_unordered(PARALLEL)
        .filter_map(|x| async { x })
        .collect()
        .await;
    results.sort_by_key(|(i, _, _)| *i);
    results.into_iter().map(|(_, u, r)| (u, r)).collect()
}

fn to_candidates(found: Vec<(String, FeedTestResult)>, existing: &HashSet<String>) -> Vec<FeedCandidate> {
    let mut seen_urls = HashSet::new();
    let mut seen_feeds = HashSet::new();
    let mut out: Vec<FeedCandidate> = found
        .into_iter()
        .filter_map(|(url, r)| {
            let norm = normalize_url(&url).unwrap_or_else(|_| url.clone());
            // The same feed under two URLs (e.g. /feed and /rss.xml) is shown once.
            let same = (r.title.clone(), r.item_count, r.newest_published_at.clone());
            if !seen_urls.insert(norm.clone()) || !seen_feeds.insert(same) {
                return None;
            }
            Some(FeedCandidate {
                already_added: existing.contains(&norm),
                url,
                title: r.title,
                item_count: r.item_count,
                newest_published_at: r.newest_published_at,
            })
        })
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.item_count));
    out
}

/// The example page and the feeds found for its site, most items first.
/// `existing` holds the URLs of the feeds already in the list.
pub async fn discover(
    http: &dyn HttpClient,
    url: &str,
    existing: &[String],
) -> AppResult<(ExamplePage, Vec<FeedCandidate>)> {
    let existing: HashSet<String> = existing.iter().filter_map(|u| normalize_url(u).ok()).collect();
    let fetched = match fetch_page(http, url).await {
        Ok(f) => f,
        // Medium often blocks apps from reading the page, but its feed still works.
        Err(e) => match Url::parse(url.trim()).ok().and_then(|u| medium_feed(&u)) {
            Some(feed_url) => {
                let found = validate(http, vec![feed_url]).await;
                if found.is_empty() {
                    return Err(e);
                }
                let page = ExamplePage {
                    url: url.trim().to_string(),
                    title: found[0].1.title.clone().unwrap_or_else(|| "Medium".into()),
                    description: None,
                    published_at: None,
                    site_name: "Medium".into(),
                    is_feed: false,
                    can_save: false,
                };
                return Ok((page, to_candidates(found, &existing)));
            }
            None => return Err(e),
        },
    };
    let info = match fetched {
        Fetched::Feed(feed, feed_url) => {
            let host = host_of(&feed_url).unwrap_or_default();
            let page = ExamplePage {
                url: feed_url.clone(),
                title: feed.title.clone().unwrap_or_else(|| host.clone()),
                description: None,
                published_at: None,
                site_name: feed.title.clone().unwrap_or(host),
                is_feed: true,
                can_save: false,
            };
            return Ok((page, to_candidates(vec![(feed_url, feed)], &existing)));
        }
        Fetched::Page(info) => info,
    };
    let mut found = validate(http, info.feed_links.clone()).await;
    let mut probes = 0;
    if found.is_empty() {
        let base = Url::parse(&info.page.url).map_err(|_| AppError::Invalid("bad page URL".into()))?;
        let urls = probe_urls(&base, &info);
        probes = urls.len();
        found = validate(http, urls).await;
    }
    let candidates = to_candidates(found, &existing);
    tracing::info!(
        links = info.feed_links.len(),
        probes,
        found = candidates.len(),
        "feed discovery done"
    );
    Ok((info.page, candidates))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::ReqwestClient;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const RSS: &str = include_str!("../../tests/fixtures/rss2_basic.xml");

    fn html(head: &str) -> String {
        format!("<!DOCTYPE html><html><head>{head}</head><body><p>Hello</p></body></html>")
    }

    fn page(body: String) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body.into_bytes(), "text/html; charset=utf-8")
    }

    fn feed() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(RSS.as_bytes().to_vec(), "application/rss+xml")
    }

    async fn run(s: &MockServer, p: &str, existing: &[String]) -> (ExamplePage, Vec<FeedCandidate>) {
        discover(&ReqwestClient::new().unwrap(), &format!("{}{p}", s.uri()), existing)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn alternate_link_with_relative_href_and_page_meta() {
        let s = MockServer::start().await;
        let head = r#"<title>Fallback</title>
            <meta property="og:title" content="Understanding Iceberg &amp; Parquet">
            <meta property="og:site_name" content="Data Blog">
            <meta name="description" content="A friendly guide.">
            <meta property="article:published_time" content="2026-09-01T10:00:00+02:00">
            <link rel="alternate" type="application/rss+xml" title="Comments" href="/comments/feed">
            <link rel="alternate" type="application/rss+xml" title="Posts" href="../feed.xml">"#;
        Mock::given(path("/blog/post/"))
            .respond_with(page(html(head)))
            .mount(&s)
            .await;
        Mock::given(path("/blog/feed.xml")).respond_with(feed()).mount(&s).await;
        Mock::given(path("/comments/feed")).respond_with(feed()).mount(&s).await;
        let (p, c) = run(&s, "/blog/post/", &[]).await;
        assert_eq!(p.title, "Understanding Iceberg & Parquet");
        assert_eq!(p.site_name, "Data Blog");
        assert_eq!(p.description.as_deref(), Some("A friendly guide."));
        assert_eq!(p.published_at.as_deref(), Some("2026-09-01T08:00:00Z"));
        assert!(!p.is_feed);
        assert_eq!(c.len(), 1, "comment feed skipped: {c:?}");
        assert_eq!(c[0].url, format!("{}/blog/feed.xml", s.uri()));
        assert_eq!(c[0].title.as_deref(), Some("Example Tech"));
        assert_eq!(c[0].item_count, 2);
        assert!(!c[0].already_added);
    }

    fn info(html_text: &str, url: &str) -> (Url, PageInfo) {
        let u = Url::parse(url).unwrap();
        let i = parse_page(html_text, &u);
        (u, i)
    }

    #[test]
    fn substack_probe() {
        let (u, i) = info(&html("<title>Post</title>"), "https://joe.substack.com/p/data-modeling");
        assert!(i.substack);
        assert_eq!(probe_urls(&u, &i)[0], "https://joe.substack.com/feed");
        // A Substack on its own domain is found by its CDN links.
        let (u, i) = info(
            &html(r#"<link rel="icon" href="https://substackcdn.com/icon.png">"#),
            "https://www.practical.dev/p/x",
        );
        assert!(i.substack);
        assert_eq!(probe_urls(&u, &i)[0], "https://www.practical.dev/feed");
    }

    #[test]
    fn generator_hints() {
        let cases = [
            (
                r#"<meta name="generator" content="WordPress 6.8">"#,
                "https://a.dev/feed/",
            ),
            (r#"<meta name="generator" content="Ghost 6.0">"#, "https://a.dev/rss/"),
            (
                r#"<meta name="generator" content="Hugo 0.150.0">"#,
                "https://a.dev/index.xml",
            ),
            (
                r#"<meta name="generator" content="Jekyll v4.4.1">"#,
                "https://a.dev/feed.xml",
            ),
        ];
        for (meta, first) in cases {
            let (u, i) = info(&html(meta), "https://a.dev/2026/09/my-post/");
            let probes = probe_urls(&u, &i);
            assert_eq!(probes[0], first, "{meta}");
            assert!(probes.len() <= MAX_PROBES);
        }
        let (_, i) = info(
            "<html><head></head><body><img src=\"/wp-content/x.png\"></body></html>",
            "https://a.dev/x",
        );
        assert_eq!(i.platform, Some(Platform::WordPress), "wp-content in the body");
    }

    #[test]
    fn medium_rewrite() {
        let m = |s: &str| medium_feed(&Url::parse(s).unwrap());
        assert_eq!(
            m("https://medium.com/@jane/why-dbt-1a2b"),
            Some("https://medium.com/feed/@jane".into())
        );
        assert_eq!(
            m("https://medium.com/towards-data-engineering/kafka-101-9f"),
            Some("https://medium.com/feed/towards-data-engineering".into())
        );
        assert_eq!(
            m("https://jane.medium.com/post-1"),
            Some("https://medium.com/feed/@jane".into())
        );
        assert_eq!(m("https://example.com/@jane/x"), None);
    }

    #[test]
    fn probe_order_and_limit() {
        let (u, i) = info(&html("<title>x</title>"), "https://a.dev/blog/my-post");
        let p = probe_urls(&u, &i);
        assert_eq!(
            p,
            vec![
                "https://a.dev/feed",
                "https://a.dev/rss.xml",
                "https://a.dev/blog/feed",
                "https://a.dev/atom.xml",
                "https://a.dev/index.xml",
                "https://a.dev/blog/rss.xml",
                "https://a.dev/blog/index.xml",
                "https://a.dev/blog/atom.xml",
            ]
        );
        // Root page: no parent path.
        let (u, i) = info(&html(""), "https://a.dev/");
        assert_eq!(probe_urls(&u, &i).len(), 5);
    }

    #[tokio::test]
    async fn wordpress_page_found_by_probe() {
        let s = MockServer::start().await;
        Mock::given(path("/2026/09/post/"))
            .respond_with(page(html(r#"<meta name="generator" content="WordPress 6.8">"#)))
            .mount(&s)
            .await;
        Mock::given(path("/feed/")).respond_with(feed()).mount(&s).await;
        let (_, c) = run(&s, "/2026/09/post/", &[]).await;
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].url, format!("{}/feed/", s.uri()));
    }

    #[tokio::test]
    async fn no_feed_and_html_candidates_are_rejected() {
        let s = MockServer::start().await;
        // Every path answers with the same HTML page (like a single-page app).
        Mock::given(method("GET"))
            .respond_with(page(html("<title>Start Data Engineering</title>")))
            .expect(1 + 5)
            .mount(&s)
            .await;
        let (p, c) = run(&s, "/", &[]).await;
        assert_eq!(p.title, "Start Data Engineering");
        assert!(c.is_empty());
    }

    #[tokio::test]
    async fn probe_limit_is_respected() {
        let s = MockServer::start().await;
        Mock::given(path("/a/b/c/post"))
            .respond_with(page(html(r#"<meta name="generator" content="Ghost 6">"#)))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .expect(MAX_PROBES as u64)
            .mount(&s)
            .await;
        let (_, c) = run(&s, "/a/b/c/post", &[]).await;
        assert!(c.is_empty());
    }

    #[tokio::test]
    async fn already_added_and_same_feed_twice() {
        let s = MockServer::start().await;
        Mock::given(path("/post"))
            .respond_with(page(html(
                r#"<link rel="alternate" type="application/rss+xml" href="/feed">
                   <link rel="alternate" type="application/atom+xml" href="/rss.xml">"#,
            )))
            .mount(&s)
            .await;
        Mock::given(path("/feed")).respond_with(feed()).mount(&s).await;
        Mock::given(path("/rss.xml")).respond_with(feed()).mount(&s).await;
        let existing = vec![format!("{}/feed/", s.uri())];
        let (_, c) = run(&s, "/post", &existing).await;
        assert_eq!(c.len(), 1, "same feed under two URLs is shown once");
        assert!(c[0].already_added, "matched after normalization");
    }

    #[tokio::test]
    async fn a_pasted_feed_url_is_its_own_candidate() {
        let s = MockServer::start().await;
        Mock::given(path("/feed")).respond_with(feed()).mount(&s).await;
        let (p, c) = run(&s, "/feed", &[]).await;
        assert!(p.is_feed);
        assert_eq!(p.title, "Example Tech");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn json_ld_date() {
        let (_, i) = info(
            r#"<html><head><script type="application/ld+json">{"@type":"NewsArticle","datePublished":"2026-09-25T17:41:04+00:00"}</script></head></html>"#,
            "https://x.substack.com/p/a",
        );
        assert_eq!(i.page.published_at.as_deref(), Some("2026-09-25T17:41:04Z"));
    }

    #[tokio::test]
    async fn bad_urls() {
        let http = ReqwestClient::new().unwrap();
        assert!(discover(&http, "not a url", &[]).await.is_err());
        assert!(discover(&http, "ftp://x.com/a", &[]).await.is_err());
    }
}
