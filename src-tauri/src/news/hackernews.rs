//! Official Hacker News Firebase API (no auth).

use chrono::{TimeZone, Utc};
use futures::{StreamExt, stream};
use serde::Deserialize;

use super::model::{FetchResult, HnStats, RawItem};
use super::normalize::{strip_html, truncate_chars};
use super::source::FetchCtx;
use crate::error::{AppError, AppResult};
use crate::http::HttpRequest;

pub const DEFAULT_BASE: &str = "https://hacker-news.firebaseio.com/v0";

#[derive(Deserialize)]
struct HnItem {
    id: i64,
    #[serde(rename = "type")]
    kind: Option<String>,
    title: Option<String>,
    url: Option<String>,
    text: Option<String>,
    by: Option<String>,
    score: Option<i64>,
    descendants: Option<i64>,
    time: Option<i64>,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    dead: bool,
}

/// "hn:top,best,show" → [(endpoint, how many ids to take)].
fn lists(feed_url: &str, include_new: bool) -> Vec<(&'static str, usize)> {
    let spec = feed_url.strip_prefix("hn:").unwrap_or("top,best,show");
    let mut out: Vec<(&'static str, usize)> = spec
        .split(',')
        .filter_map(|s| match s.trim() {
            "top" => Some(("topstories", 60)),
            "best" => Some(("beststories", 30)),
            "show" => Some(("showstories", 20)),
            "new" => Some(("newstories", 30)),
            _ => None,
        })
        .collect();
    if include_new && !out.iter().any(|(e, _)| *e == "newstories") {
        out.push(("newstories", 30));
    }
    out
}

async fn get_json<T: for<'de> Deserialize<'de>>(ctx: &FetchCtx, url: String) -> AppResult<T> {
    let resp = ctx.http.get(HttpRequest::get(url)).await?;
    if !(200..300).contains(&resp.status) {
        return Err(AppError::Network(format!("HTTP {}", resp.status)));
    }
    serde_json::from_slice(&resp.body).map_err(|e| AppError::Network(format!("bad HN response: {e}")))
}

fn to_raw(it: HnItem) -> Option<RawItem> {
    if it.deleted || it.dead || it.kind.as_deref() != Some("story") {
        return None;
    }
    let title = strip_html(it.title.as_deref()?);
    if title.is_empty() {
        return None;
    }
    let url = it
        .url
        .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={}", it.id));
    Some(RawItem {
        url,
        title,
        description: it
            .text
            .map(|t| truncate_chars(&strip_html(&t), 1000))
            .filter(|t| !t.is_empty()),
        author: it.by,
        published_at: it.time.and_then(|t| Utc.timestamp_opt(t, 0).single()),
        hn: Some(HnStats {
            id: it.id,
            points: it.score.unwrap_or(0),
            comments: it.descendants.unwrap_or(0),
        }),
    })
}

pub async fn fetch(ctx: &FetchCtx, feed_url: &str) -> AppResult<FetchResult> {
    let base = if ctx.hn_base.is_empty() {
        DEFAULT_BASE
    } else {
        ctx.hn_base.as_str()
    };
    let mut ids: Vec<i64> = Vec::new();
    let mut list_errors = 0;
    let wanted = lists(feed_url, ctx.hn_include_new);
    for (endpoint, n) in &wanted {
        match get_json::<Vec<i64>>(ctx, format!("{base}/{endpoint}.json")).await {
            Ok(v) => ids.extend(v.into_iter().take(*n)),
            Err(e) => {
                tracing::warn!(endpoint, error = %e, "HN list failed");
                list_errors += 1;
            }
        }
    }
    if list_errors == wanted.len() {
        return Err(AppError::Network("Could not reach Hacker News".into()));
    }
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(*id) && !ctx.skip_hn_ids.contains(id));

    let items: Vec<RawItem> = stream::iter(ids)
        .map(|id| async move { get_json::<Option<HnItem>>(ctx, format!("{base}/item/{id}.json")).await })
        .buffer_unordered(8)
        .filter_map(|r| async move {
            match r {
                Ok(Some(it)) => to_raw(it),
                Ok(None) => None,
                Err(e) => {
                    tracing::debug!(error = %e, "HN item failed");
                    None
                }
            }
        })
        .collect()
        .await;
    Ok(FetchResult {
        items,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::ReqwestClient;
    use std::collections::HashSet;
    use std::sync::Arc;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ctx(base: String, skip: &[i64]) -> FetchCtx {
        FetchCtx {
            http: Arc::new(ReqwestClient::new().unwrap()),
            etag: None,
            last_modified: None,
            skip_hn_ids: Arc::new(skip.iter().copied().collect::<HashSet<_>>()),
            hn_include_new: false,
            hn_base: base,
        }
    }

    async fn server() -> MockServer {
        let s = MockServer::start().await;
        Mock::given(path("/topstories.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/hn_topstories.json")),
            )
            .mount(&s)
            .await;
        Mock::given(path("/item/1.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/hn_item_story.json")),
            )
            .mount(&s)
            .await;
        Mock::given(path("/item/2.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/hn_item_ask.json")),
            )
            .mount(&s)
            .await;
        Mock::given(path("/item/3.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/hn_item_dead.json")),
            )
            .mount(&s)
            .await;
        Mock::given(path("/item/4.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("null"))
            .mount(&s)
            .await;
        s
    }

    #[tokio::test]
    async fn fetches_stories_and_skips_dead() {
        let s = server().await;
        let r = fetch(&ctx(s.uri(), &[]), "hn:top").await.unwrap();
        let mut ids: Vec<i64> = r.items.iter().map(|i| i.hn.as_ref().unwrap().id).collect();
        ids.sort();
        assert_eq!(ids, vec![1, 2]);
        let ask = r.items.iter().find(|i| i.hn.as_ref().unwrap().id == 2).unwrap();
        assert_eq!(ask.url, "https://news.ycombinator.com/item?id=2");
        let story = r.items.iter().find(|i| i.hn.as_ref().unwrap().id == 1).unwrap();
        assert_eq!(story.hn.as_ref().unwrap().points, 412);
        assert_eq!(story.hn.as_ref().unwrap().comments, 150);
    }

    #[tokio::test]
    async fn skips_recently_checked_ids() {
        let s = server().await;
        let r = fetch(&ctx(s.uri(), &[1]), "hn:top").await.unwrap();
        assert!(r.items.iter().all(|i| i.hn.as_ref().unwrap().id != 1));
    }

    #[tokio::test]
    async fn all_lists_failing_is_an_error() {
        let s = MockServer::start().await;
        assert!(fetch(&ctx(s.uri(), &[]), "hn:top,best").await.is_err());
    }

    #[test]
    fn parses_list_spec() {
        assert_eq!(lists("hn:top,best,show", false).len(), 3);
        assert_eq!(lists("hn:top", true), vec![("topstories", 60), ("newstories", 30)]);
    }
}
