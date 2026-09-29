use std::collections::HashSet;
use std::sync::Arc;

use crate::db::repo::feeds::Feed;
use crate::error::AppResult;
use crate::http::HttpClient;

use super::model::FetchResult;
use super::{hackernews, rss};

/// Everything a source needs for one fetch.
#[derive(Clone)]
pub struct FetchCtx {
    pub http: Arc<dyn HttpClient>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// HN ids refreshed less than 1 h ago: skip their item requests.
    pub skip_hn_ids: Arc<HashSet<i64>>,
    pub hn_include_new: bool,
    pub hn_base: String,
}

/// One entry point for every feed kind (the `FeedSource` of SPEC §7.2).
pub async fn fetch(feed: &Feed, ctx: &FetchCtx) -> AppResult<FetchResult> {
    match feed.kind.as_str() {
        "hn" => hackernews::fetch(ctx, &feed.url).await,
        _ => rss::fetch(ctx, &feed.url).await,
    }
}
