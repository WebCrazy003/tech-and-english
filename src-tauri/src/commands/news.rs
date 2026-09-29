use chrono::Duration;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::CmdResult;
use crate::clock::fmt_ts;
use crate::db::repo::articles::{self, ArticleFilter, ArticleListItem, Page};
use crate::db::repo::feeds::{self, Feed, FeedInput};
use crate::db::repo::topics::{self, Topic, TopicInput};
use crate::error::AppError;
use crate::news::model::InteractionKind;
use crate::news::pick::DailyPick;
use crate::news::rss::FeedTestResult;
use crate::news::topics::{compile, match_topics};
use crate::state::AppState;

#[tauri::command]
pub async fn list_topics(state: State<'_, AppState>) -> CmdResult<Vec<Topic>> {
    state.db.call(|c| topics::list(c)).await
}

#[tauri::command]
pub async fn upsert_topic(state: State<'_, AppState>, topic: TopicInput) -> CmdResult<Topic> {
    let now = fmt_ts(state.clock.now());
    let t = state.db.call(move |c| topics::upsert(c, &topic, &now)).await?;
    state.news.reload_topics().await?;
    Ok(t)
}

#[tauri::command]
pub async fn delete_topic(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.db.call(move |c| topics::delete(c, id)).await?;
    state.news.reload_topics().await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchPreview {
    pub matched: usize,
    pub total: usize,
}

/// "Would match N of the last 72 h articles" while editing a topic.
#[tauri::command]
pub async fn preview_topic_matches(
    state: State<'_, AppState>,
    keywords: Vec<String>,
    excluded_keywords: Vec<String>,
) -> CmdResult<MatchPreview> {
    let since = fmt_ts(state.clock.now() - Duration::hours(72));
    let topic = Topic {
        id: 0,
        name: "preview".into(),
        keywords,
        excluded_keywords,
        priority: 3,
        enabled: true,
        notify: false,
        notify_threshold: None,
    };
    state
        .db
        .call(move |c| {
            let compiled = compile(&[topic]);
            let mut st =
                c.prepare("SELECT title, description FROM articles WHERE discovered_at >= ?1 AND hidden = 0")?;
            let rows = st
                .query_map([since], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let matched = rows
                .iter()
                .filter(|(t, d)| !match_topics(&compiled, t, d.as_deref()).is_empty())
                .count();
            Ok(MatchPreview {
                matched,
                total: rows.len(),
            })
        })
        .await
}

#[tauri::command]
pub async fn list_feeds(state: State<'_, AppState>) -> CmdResult<Vec<Feed>> {
    state.db.call(|c| feeds::list(c)).await
}

#[tauri::command]
pub async fn upsert_feed(state: State<'_, AppState>, feed: FeedInput) -> CmdResult<Feed> {
    let checked = feeds::validate(&feed)?;
    if checked.id.is_none() && checked.kind == "rss" {
        state.news.test_feed(&checked.url).await?;
    }
    let now = fmt_ts(state.clock.now());
    let saved = state.db.call(move |c| feeds::upsert(c, &checked, &now)).await?;
    if saved.last_fetched_at.is_none() && saved.enabled {
        let news = state.news.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = news.fetch_cycle(false).await {
                tracing::warn!(error = %e, "fetch after adding feed failed");
            }
        });
    }
    Ok(saved)
}

#[tauri::command]
pub async fn delete_feed(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.db.call(move |c| feeds::delete(c, id)).await
}

#[tauri::command]
pub async fn test_feed(state: State<'_, AppState>, url: String) -> CmdResult<FeedTestResult> {
    let url = url.trim().to_string();
    feeds::validate(&FeedInput {
        id: None,
        kind: "rss".into(),
        name: "test".into(),
        url: url.clone(),
        source_weight: 0.5,
        enabled: true,
    })?;
    state.news.test_feed(&url).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResult {
    pub new_count: u32,
}

#[tauri::command]
pub async fn refresh_now(state: State<'_, AppState>) -> CmdResult<RefreshResult> {
    let new_count = state.news.fetch_cycle(true).await?;
    // A manual refresh may make today's pick possible.
    state.pick.ensure_today().await?;
    Ok(RefreshResult { new_count })
}

#[tauri::command]
pub async fn list_articles(
    state: State<'_, AppState>,
    filter: Option<ArticleFilter>,
    cursor: Option<String>,
    limit: Option<u32>,
) -> CmdResult<Page<ArticleListItem>> {
    let f = filter.unwrap_or_default();
    state
        .db
        .call(move |c| articles::list_items(c, &f, cursor.as_deref(), limit.unwrap_or(50)))
        .await
}

#[tauri::command]
pub async fn get_article(state: State<'_, AppState>, id: i64) -> CmdResult<ArticleListItem> {
    state.db.call(move |c| articles::get_item(c, id)).await
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiInteraction {
    Opened,
    Read,
    Saved,
    Liked,
    NotInterested,
}

#[tauri::command]
pub async fn record_interaction(state: State<'_, AppState>, article_id: i64, kind: UiInteraction) -> CmdResult<()> {
    let kind = match kind {
        UiInteraction::Opened => InteractionKind::Opened,
        UiInteraction::Read => InteractionKind::Read,
        UiInteraction::Saved => InteractionKind::Saved,
        UiInteraction::Liked => InteractionKind::Liked,
        UiInteraction::NotInterested => InteractionKind::NotInterested,
    };
    state.news.record_interaction(article_id, kind).await
}

#[tauri::command]
pub async fn set_saved(state: State<'_, AppState>, article_id: i64, saved: bool) -> CmdResult<()> {
    if saved {
        state.news.record_interaction(article_id, InteractionKind::Saved).await
    } else {
        state.news.unsave(article_id).await
    }
}

/// Record "opened" and open the article in the default browser.
#[tauri::command]
pub async fn open_article(app: AppHandle, state: State<'_, AppState>, article_id: i64) -> CmdResult<()> {
    let item = state.db.call(move |c| articles::get_item(c, article_id)).await?;
    app.opener()
        .open_url(item.url.clone(), None::<&str>)
        .map_err(|e| AppError::Internal(format!("could not open browser: {e}")))?;
    state.news.record_interaction(article_id, InteractionKind::Opened).await
}

#[tauri::command]
pub async fn get_today_pick(state: State<'_, AppState>) -> CmdResult<Option<DailyPick>> {
    state.pick.today().await
}

#[tauri::command]
pub async fn get_pick_preview(state: State<'_, AppState>) -> CmdResult<Option<ArticleListItem>> {
    state.pick.preview().await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsStatus {
    pub article_count: i64,
    pub feed_count: usize,
    pub feeds_with_errors: usize,
    pub last_fetched_at: Option<String>,
}

#[tauri::command]
pub async fn news_status(state: State<'_, AppState>) -> CmdResult<NewsStatus> {
    state
        .db
        .call(|c| {
            let feeds = feeds::list(c)?;
            Ok(NewsStatus {
                article_count: articles::count(c)?,
                feed_count: feeds.len(),
                feeds_with_errors: feeds.iter().filter(|f| f.last_error.is_some()).count(),
                last_fetched_at: feeds.iter().filter_map(|f| f.last_fetched_at.clone()).max(),
            })
        })
        .await
}
