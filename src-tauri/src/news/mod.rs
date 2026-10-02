pub mod body;
pub mod difficulty;
pub mod discover;
pub mod hackernews;
pub mod ingest;
pub mod learning;
pub mod model;
pub mod normalize;
pub mod pick;
pub mod ranking;
pub mod retention;
pub mod rss;
pub mod source;
pub mod topics;
pub mod why;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use futures::{StreamExt, stream};
use serde_json::json;

use crate::clock::{Clock, fmt_ts, parse_ts};
use crate::db::Db;
use crate::db::repo::feeds::{self, Feed};
use crate::db::repo::{app_state, articles, interactions, topics as topics_repo};
use crate::error::{AppError, AppResult};
use crate::events::{self, EventSink};
use crate::http::HttpClient;
use crate::mode::{Mode, ModeManager};
use crate::settings::SettingsStore;
use ingest::RecentTitles;
use learning::LESSON_THRESHOLD;
use model::InteractionKind;
use normalize::{jaccard, word_set};
use ranking::{LessonInputs, RankInputs, affinity_deltas};
use source::FetchCtx;
use topics::CompiledTopic;

/// Articles discovered within this window are rescored every cycle (freshness changes).
pub const RESCORE_WINDOW_DAYS: i64 = 7;
/// Learning articles are kept and rescored for lessons this long (SPEC §7.11).
pub const LESSON_WINDOW_DAYS: i64 = retention::DELETE_AFTER_DAYS;
/// Example articles and feeds added from an example get this source weight.
pub const EXAMPLE_SOURCE_WEIGHT: f64 = 0.6;
const MAX_BACKOFF: Duration = Duration::hours(6);
const OFFLINE_RETRY_SECS: [i64; 3] = [30, 60, 120];
/// Background body check after a fetch: new articles from the last day, best first.
const CHECK_BODIES_DAYS: i64 = 1;
const CHECK_BODIES_MAX: u32 = 40;

#[derive(Default)]
struct OfflineState {
    retry_at: Option<DateTime<Utc>>,
    attempts: usize,
}

pub struct NewsService {
    db: Db,
    http: Arc<dyn HttpClient>,
    clock: Arc<dyn Clock>,
    settings: Arc<SettingsStore>,
    events: Arc<dyn EventSink>,
    mode: Arc<ModeManager>,
    topics: RwLock<Arc<Vec<CompiledTopic>>>,
    fetch_lock: tokio::sync::Mutex<()>,
    offline: Mutex<OfflineState>,
    hn_base: String,
    /// The daily pick waits here for the frontend to extract a body.
    pub body_waiters: body::BodyWaiters,
}

/// Is `feed` due for a fetch at `now`? The interval doubles per consecutive failure (max 6 h).
pub fn is_due(feed: &Feed, now: DateTime<Utc>, base_minutes: u32) -> bool {
    let Some(last) = feed.last_fetched_at.as_deref().and_then(parse_ts) else {
        return true;
    };
    let factor = 2i64.pow(feed.consecutive_failures.min(5));
    let interval = (Duration::minutes(base_minutes as i64) * factor as i32).min(MAX_BACKOFF);
    now - last >= interval
}

impl NewsService {
    pub async fn new(
        db: Db,
        http: Arc<dyn HttpClient>,
        clock: Arc<dyn Clock>,
        settings: Arc<SettingsStore>,
        events: Arc<dyn EventSink>,
        mode: Arc<ModeManager>,
        hn_base: Option<String>,
    ) -> AppResult<Arc<Self>> {
        let list = db.call(|c| topics_repo::list(c)).await?;
        Ok(Arc::new(Self {
            db,
            http,
            clock,
            settings,
            events,
            mode,
            topics: RwLock::new(Arc::new(topics::compile(&list))),
            fetch_lock: tokio::sync::Mutex::new(()),
            offline: Mutex::new(OfflineState::default()),
            hn_base: hn_base.unwrap_or_default(),
            body_waiters: body::BodyWaiters::default(),
        }))
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    fn compiled_topics(&self) -> Arc<Vec<CompiledTopic>> {
        self.topics.read().unwrap().clone()
    }

    /// Recompile topics after an edit, re-match recent (and learning) articles and rescore.
    pub async fn reload_topics(&self) -> AppResult<()> {
        let list = self.db.call(|c| topics_repo::list(c)).await?;
        let compiled = Arc::new(topics::compile(&list));
        *self.topics.write().unwrap() = compiled.clone();
        let now = self.clock.now();
        let since = fmt_ts(now - Duration::days(RESCORE_WINDOW_DAYS));
        let lesson_since = fmt_ts(now - Duration::days(LESSON_WINDOW_DAYS));
        self.db
            .tx(move |tx| {
                let mut ids = articles::ids_discovered_since(tx, &since)?;
                ids.extend(articles::learning_ids_since(tx, &lesson_since, LESSON_THRESHOLD)?);
                ingest::rematch_all(tx, &ids, &compiled)
            })
            .await?;
        self.rescore().await?;
        events::emit(self.events.as_ref(), events::NEWS_UPDATED, &json!({ "newCount": 0 }));
        Ok(())
    }

    /// Fetch every due feed (or all enabled feeds if `force`), store items, rescore.
    /// Returns the number of new articles. Cycles never overlap.
    pub async fn fetch_cycle(&self, force: bool) -> AppResult<u32> {
        let _guard = if force {
            self.fetch_lock.lock().await
        } else {
            match self.fetch_lock.try_lock() {
                Ok(g) => g,
                Err(_) => return Ok(0),
            }
        };
        let now = self.clock.now();
        if !force && self.offline.lock().unwrap().retry_at.is_some_and(|r| now < r) {
            return Ok(0);
        }
        let settings = self.settings.get();
        let base = match self.mode.get() {
            Mode::Standard => settings.fetch_interval_standard_min,
            Mode::Hibernate => settings.fetch_interval_hibernate_min,
        };
        let all = self.db.call(|c| feeds::list(c)).await?;
        let due: Vec<Feed> = all
            .into_iter()
            .filter(|f| f.enabled && (force || is_due(f, now, base)))
            .collect();
        if due.is_empty() {
            return Ok(0);
        }

        let since_1h = fmt_ts(now - Duration::hours(1));
        let skip = Arc::new(self.db.call(move |c| articles::hn_fresh_ids(c, &since_1h)).await?);
        let results: Vec<(Feed, AppResult<model::FetchResult>)> = stream::iter(due)
            .map(|feed| {
                let ctx = FetchCtx {
                    http: self.http.clone(),
                    etag: feed.etag.clone(),
                    last_modified: feed.last_modified.clone(),
                    skip_hn_ids: skip.clone(),
                    hn_include_new: settings.hn_include_new,
                    hn_base: self.hn_base.clone(),
                };
                async move {
                    let r = match tokio::time::timeout(StdDuration::from_secs(60), source::fetch(&feed, &ctx)).await {
                        Ok(r) => r,
                        Err(_) => Err(AppError::Network("Timed out".into())),
                    };
                    (feed, r)
                }
            })
            .buffer_unordered(4)
            .collect()
            .await;

        let all_offline = results.iter().all(|(_, r)| matches!(r, Err(AppError::Network(_))));
        if all_offline && !force {
            let mut off = self.offline.lock().unwrap();
            let secs = OFFLINE_RETRY_SECS[off.attempts.min(OFFLINE_RETRY_SECS.len() - 1)];
            off.attempts += 1;
            off.retry_at = Some(now + Duration::seconds(secs));
            tracing::info!(retry_in_secs = secs, "all feeds failed; network probably offline");
            return Ok(0);
        }
        *self.offline.lock().unwrap() = OfflineState::default();

        let topics = self.compiled_topics();
        let (new_count, ok_count) = self
            .db
            .tx(move |tx| {
                let now_s = fmt_ts(now);
                let mut recent = RecentTitles::load(tx, now)?;
                let (mut new, mut ok) = (0u32, 0u32);
                for (feed, r) in results {
                    match r {
                        Ok(res) => {
                            let max_age = ingest::max_age_days(&feed, &settings);
                            let stats = ingest::ingest(tx, &feed, &res.items, &topics, &mut recent, now, max_age)?;
                            feeds::record_success(
                                tx,
                                feed.id,
                                &now_s,
                                res.etag.as_deref(),
                                res.last_modified.as_deref(),
                            )?;
                            tracing::debug!(feed = feed.id, new = stats.new, merged = stats.merged, "feed fetched");
                            new += stats.new;
                            ok += 1;
                        }
                        Err(e) => {
                            tracing::warn!(feed = feed.id, error = %e, "feed fetch failed");
                            feeds::record_failure(tx, feed.id, &now_s, &e.to_string())?;
                        }
                    }
                }
                if ok > 0 && app_state::get(tx, app_state::FIRST_FETCH_COMPLETED_AT)?.is_none() {
                    app_state::set(tx, app_state::FIRST_FETCH_COMPLETED_AT, &now_s)?;
                }
                Ok((new, ok))
            })
            .await?;
        tracing::info!(new = new_count, feeds_ok = ok_count, "fetch cycle done");
        self.rescore().await?;
        events::emit(
            self.events.as_ref(),
            events::NEWS_UPDATED,
            &json!({ "newCount": new_count }),
        );
        if new_count > 0 {
            let since = fmt_ts(now - Duration::days(CHECK_BODIES_DAYS));
            let ids = self
                .db
                .call(move |c| articles::unchecked_bodies(c, &since, CHECK_BODIES_MAX))
                .await?;
            if !ids.is_empty() {
                events::emit(
                    self.events.as_ref(),
                    events::CHECK_BODIES,
                    &json!({ "articleIds": ids }),
                );
            }
        }
        Ok(new_count)
    }

    /// Recompute scores of recent articles (SPEC §7.6).
    pub async fn rescore(&self) -> AppResult<()> {
        let now = self.clock.now();
        let weights = self.settings.get().ranking_weights;
        self.db
            .tx(move |tx| {
                let since = fmt_ts(now - Duration::days(RESCORE_WINDOW_DAYS));
                let rows = articles::scoring_rows(tx, &since)?;
                let engaged = articles::engaged_title_keys(tx, &since)?;
                let engaged_sets: Vec<(i64, std::collections::HashSet<&str>)> =
                    engaged.iter().map(|(id, k)| (*id, word_set(k))).collect();
                let aff: HashMap<(String, i64), f64> = interactions::all_affinities(tx)?;
                let now_s = fmt_ts(now);
                for r in rows {
                    let when = r
                        .published_at
                        .as_deref()
                        .or(Some(r.discovered_at.as_str()))
                        .and_then(parse_ts);
                    let age_hours = when.map(|w| (now - w).num_minutes() as f64 / 60.0).unwrap_or(0.0);
                    let ws = word_set(&r.title_key);
                    let max_sim = engaged_sets
                        .iter()
                        .filter(|(id, _)| *id != r.id)
                        .map(|(_, s)| jaccard(&ws, s))
                        .fold(0.0, f64::max);
                    let topic_aff = r
                        .primary_topic_id
                        .and_then(|t| aff.get(&("topic".to_string(), t)).copied())
                        .unwrap_or(0.0);
                    let source_aff = r
                        .source_ids
                        .iter()
                        .filter_map(|f| aff.get(&("source".to_string(), *f)).copied())
                        .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))))
                        .unwrap_or(0.0);
                    let b = ranking::score(
                        &RankInputs {
                            relevance: r.relevance,
                            age_hours,
                            hn_points: r.hn_points,
                            hn_comments: r.hn_comments,
                            source_weight: r.source_weight,
                            max_recent_similarity: max_sim,
                            topic_affinity: topic_aff,
                            source_affinity: source_aff,
                        },
                        &weights,
                    );
                    articles::update_score(tx, r.id, &b, &now_s)?;
                }

                // Lessons (P3): only learning material, 60-day window, no freshness.
                articles::clear_stale_lesson_scores(tx, LESSON_THRESHOLD)?;
                let lesson_since = fmt_ts(now - Duration::days(LESSON_WINDOW_DAYS));
                for r in articles::lesson_rows(tx, &lesson_since, LESSON_THRESHOLD)? {
                    let topic_aff = r
                        .primary_topic_id
                        .and_then(|t| aff.get(&("topic".to_string(), t)).copied())
                        .unwrap_or(0.0);
                    let source_aff = r
                        .source_ids
                        .iter()
                        .filter_map(|f| aff.get(&("source".to_string(), *f)).copied())
                        .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))))
                        .unwrap_or(0.0);
                    let s = ranking::lesson_score(&LessonInputs {
                        learn_relevance: r.learn_relevance,
                        learning_score: r.learning_score,
                        source_weight: r.source_weight,
                        hn_points: r.hn_points,
                        hn_comments: r.hn_comments,
                        topic_affinity: topic_aff,
                        source_affinity: source_aff,
                    });
                    articles::set_lesson_score(tx, r.id, s)?;
                }
                Ok(())
            })
            .await
    }

    /// A feed's Learning switch changed: recompute its articles' learning scores (60 days).
    pub async fn recompute_learning_for_feed(&self, feed_id: i64) -> AppResult<()> {
        let since = fmt_ts(self.clock.now() - Duration::days(LESSON_WINDOW_DAYS));
        let n = self
            .db
            .tx(move |tx| {
                let ids = articles::ids_for_feed_since(tx, feed_id, &since)?;
                for id in &ids {
                    ingest::update_learning(tx, *id)?;
                }
                Ok(ids.len())
            })
            .await?;
        tracing::debug!(feed = feed_id, articles = n, "learning scores recomputed");
        self.rescore().await?;
        events::emit(self.events.as_ref(), events::NEWS_UPDATED, &json!({ "newCount": 0 }));
        Ok(())
    }

    /// Give a learning score to articles that have none (stored before P3). Returns how many.
    pub async fn backfill_learning(&self) -> AppResult<usize> {
        let n = self
            .db
            .tx(|tx| {
                let ids = articles::ids_without_learning_score(tx)?;
                for id in &ids {
                    ingest::update_learning(tx, *id)?;
                }
                Ok(ids.len())
            })
            .await?;
        if n > 0 {
            tracing::info!(articles = n, "learning scores backfilled");
            self.rescore().await?;
            events::emit(self.events.as_ref(), events::NEWS_UPDATED, &json!({ "newCount": 0 }));
        }
        Ok(n)
    }

    /// Find the feeds of the site behind an example URL (SPEC §7.12).
    pub async fn discover_feeds(&self, url: &str) -> AppResult<discover::Discovery> {
        let existing: Vec<String> = self
            .db
            .call(|c| Ok(feeds::list(c)?.into_iter().map(|f| f.url).collect()))
            .await?;
        let (page, candidates) = discover::discover(self.http.as_ref(), url, &existing).await?;
        Ok(discover::Discovery { page, candidates })
    }

    /// Add the chosen feed and/or save the example article. The caller starts a fetch.
    pub async fn add_from_example(&self, input: AddFromExample) -> AppResult<AddedFromExample> {
        let feed_input = match input.feed_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            Some(u) => {
                let f = feeds::validate(&feeds::FeedInput {
                    id: None,
                    kind: "rss".into(),
                    name: input.name.clone(),
                    url: u.to_string(),
                    source_weight: EXAMPLE_SOURCE_WEIGHT,
                    enabled: true,
                    learning: input.learning,
                })?;
                let norm = normalize::normalize_url(&f.url)?;
                let dup = self
                    .db
                    .call(move |c| {
                        Ok(feeds::list(c)?
                            .iter()
                            .any(|x| normalize::normalize_url(&x.url).is_ok_and(|n| n == norm)))
                    })
                    .await?;
                if dup {
                    return Err(AppError::Invalid("This source is already in your list.".into()));
                }
                Some(f)
            }
            None => None,
        };
        if feed_input.is_none() && !input.save_article {
            return Err(AppError::Invalid("Choose a feed, or save the article".into()));
        }
        let page = if input.save_article {
            match discover::fetch_page(self.http.as_ref(), &input.url).await? {
                discover::Fetched::Page(info) => Some(info.page),
                discover::Fetched::Feed(..) => {
                    return Err(AppError::Invalid("This link is a feed, not an article".into()));
                }
            }
        } else {
            None
        };
        let now = self.clock.now();
        let topics = self.compiled_topics();
        let out = self
            .db
            .tx(move |tx| {
                let now_s = fmt_ts(now);
                let feed = feed_input.map(|f| feeds::upsert(tx, &f, &now_s)).transpose()?;
                let article = page
                    .map(|p| save_example_article(tx, &p, &topics, &now_s))
                    .transpose()?;
                Ok(AddedFromExample { feed, article })
            })
            .await?;
        tracing::info!(
            feed = out.feed.as_ref().map(|f| f.id),
            article = out.article.as_ref().map(|a| a.id),
            "added from example"
        );
        self.rescore().await?;
        events::emit(self.events.as_ref(), events::NEWS_UPDATED, &json!({ "newCount": 0 }));
        match out.article {
            Some(a) => {
                let id = a.id;
                let article = self.db.call(move |c| articles::get_item(c, id)).await?;
                Ok(AddedFromExample {
                    article: Some(article),
                    ..out
                })
            }
            None => Ok(out),
        }
    }

    /// Record a user interaction, apply affinity changes (SPEC §7.7) and rescore.
    pub async fn record_interaction(&self, article_id: i64, kind: InteractionKind) -> AppResult<()> {
        let now = fmt_ts(self.clock.now());
        self.db
            .tx(move |tx| {
                articles::get_item(tx, article_id)?; // 404 for unknown ids
                interactions::insert(tx, article_id, kind.as_str(), &now)?;
                match kind {
                    InteractionKind::Opened => articles::mark_opened(tx, article_id)?,
                    InteractionKind::Read => articles::mark_read(tx, article_id)?,
                    InteractionKind::Saved => articles::set_saved(tx, article_id, true)?,
                    InteractionKind::NotInterested => articles::set_hidden(tx, article_id)?,
                    _ => {}
                }
                if let Some((dt, ds)) = affinity_deltas(kind) {
                    if let Some((topic, rel)) = articles::topic_relevances(tx, article_id)?.first().copied() {
                        interactions::add_affinity(tx, "topic", topic, dt * rel, &now)?;
                    }
                    for feed in articles::source_feed_ids(tx, article_id)? {
                        interactions::add_affinity(tx, "source", feed, ds, &now)?;
                    }
                }
                Ok(())
            })
            .await?;
        self.rescore().await?;
        events::emit(self.events.as_ref(), events::NEWS_UPDATED, &json!({ "newCount": 0 }));
        Ok(())
    }

    pub async fn unsave(&self, article_id: i64) -> AppResult<()> {
        self.db.call(move |c| articles::set_saved(c, article_id, false)).await?;
        events::emit(self.events.as_ref(), events::NEWS_UPDATED, &json!({ "newCount": 0 }));
        Ok(())
    }

    pub async fn test_feed(&self, url: &str) -> AppResult<rss::FeedTestResult> {
        rss::test_feed(self.http.as_ref(), url).await
    }

    /// Fetch the article page for the frontend's Readability pass.
    pub async fn fetch_article_html(&self, article_id: i64) -> AppResult<body::FetchedHtml> {
        let item = self.db.call(move |c| articles::get_item(c, article_id)).await?;
        body::fetch_html(self.http.as_ref(), &item.url).await
    }

    /// Store the extracted body: status, difficulty, better topic match; wake the pick waiters.
    pub async fn save_article_body(&self, input: SaveBody) -> AppResult<articles::ArticleListItem> {
        let report = input.text.as_deref().map(difficulty::analyze);
        let word_count = report.as_ref().map(|r| r.word_count).unwrap_or(0);
        let id = input.article_id;
        let topics = self.compiled_topics();
        let item = self
            .db
            .tx(move |tx| {
                let current = articles::get_item(tx, id)?;
                let desc_len = current.description.as_deref().map(str::len).unwrap_or(0);
                let status = body::body_status(word_count, desc_len, input.paywall_hint, input.failed);
                let keep = status == "ok";
                articles::save_body(
                    tx,
                    id,
                    &articles::BodyUpdate {
                        status,
                        text: input.text.as_deref().filter(|_| keep),
                        html: input.html.as_deref().filter(|_| keep),
                        canonical_url: input.canonical_url.as_deref(),
                        word_count: keep.then_some(word_count),
                        difficulty: report.as_ref().filter(|_| keep).map(|r| r.level.as_str()),
                    },
                )?;
                if keep {
                    ingest::rematch(tx, id, &topics)?;
                } else {
                    articles::set_hidden(tx, id)?;
                }
                // The word count is known now (long bodies get a small bonus).
                ingest::update_learning(tx, id)?;
                Ok((articles::get_item(tx, id)?, !current.hidden && !keep))
            })
            .await?;
        let (item, newly_hidden) = item;
        tracing::debug!(article = id, status = %item.body_status, words = word_count, "body saved");
        self.body_waiters.wake(id, &item.body_status);
        // Lists drop the story only when it was visible before (the background check saves many bodies).
        if newly_hidden {
            events::emit(self.events.as_ref(), events::NEWS_UPDATED, &json!({ "newCount": 0 }));
        }
        events::emit(
            self.events.as_ref(),
            events::ARTICLE_BODY,
            &json!({ "articleId": id, "status": item.body_status }),
        );
        Ok(item)
    }
}

/// Store the example page as a saved article (no feed source), or mark an existing one saved.
fn save_example_article(
    conn: &rusqlite::Connection,
    p: &discover::ExamplePage,
    topics: &[CompiledTopic],
    now: &str,
) -> AppResult<articles::ArticleListItem> {
    let norm = normalize::normalize_url(&p.url)?;
    let id = match articles::find_id_by_normalized_url(conn, &norm)? {
        Some(id) => id,
        None => {
            let title = normalize::truncate_chars(&p.title, 300);
            let key = match normalize::title_key(&title, &p.site_name) {
                k if k.is_empty() => norm.clone(),
                k => k,
            };
            let id = articles::insert(
                conn,
                &articles::NewArticle {
                    url: p.url.clone(),
                    normalized_url: norm,
                    title,
                    title_key: key,
                    source_name: p.site_name.clone(),
                    author: None,
                    description: p.description.clone(),
                    published_at: p.published_at.clone(),
                    discovered_at: now.to_string(),
                    hn: None,
                },
            )?;
            ingest::rematch(conn, id, topics)?;
            ingest::update_learning(conn, id)?;
            id
        }
    };
    articles::set_saved(conn, id, true)?;
    articles::get_item(conn, id)
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddFromExample {
    pub url: String,
    pub feed_url: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub learning: bool,
    #[serde(default)]
    pub save_article: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedFromExample {
    pub feed: Option<Feed>,
    pub article: Option<articles::ArticleListItem>,
}

/// What the frontend sends after running Readability (P2 dev spec §3).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveBody {
    pub article_id: i64,
    pub text: Option<String>,
    pub html: Option<String>,
    pub canonical_url: Option<String>,
    #[serde(default)]
    pub paywall_hint: bool,
    #[serde(default)]
    pub failed: bool,
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    use crate::clock::FakeClock;
    use crate::db::repo::feeds::FeedInput;
    use crate::db::repo::topics::TopicInput;
    use crate::events::RecordingEventSink;
    use crate::http::ReqwestClient;

    pub struct Harness {
        pub db: Db,
        pub clock: Arc<FakeClock>,
        pub settings: Arc<SettingsStore>,
        pub events: Arc<RecordingEventSink>,
        pub mode: Arc<ModeManager>,
        pub news: Arc<NewsService>,
    }

    pub async fn harness(now: &str) -> Harness {
        let db = Db::open_in_memory().unwrap();
        let clock = Arc::new(FakeClock::at(now));
        let settings = SettingsStore::load(db.clone()).await.unwrap();
        settings.update(json!({ "onboardingDone": true })).await.unwrap();
        let events = Arc::new(RecordingEventSink::default());
        let mode = ModeManager::load(db.clone(), events.clone()).await.unwrap();
        let news = NewsService::new(
            db.clone(),
            Arc::new(ReqwestClient::new().unwrap()),
            clock.clone(),
            settings.clone(),
            events.clone(),
            mode.clone(),
            None,
        )
        .await
        .unwrap();
        Harness {
            db,
            clock,
            settings,
            events,
            mode,
            news,
        }
    }

    pub async fn add_topic(h: &Harness, name: &str, kws: &[&str]) -> i64 {
        add_topic_learn(h, name, kws, false).await
    }

    pub async fn add_topic_learn(h: &Harness, name: &str, kws: &[&str], learn: bool) -> i64 {
        let input = TopicInput {
            id: None,
            name: name.into(),
            keywords: kws.iter().map(|s| s.to_string()).collect(),
            excluded_keywords: vec![],
            priority: 3,
            enabled: true,
            notify: true,
            notify_threshold: None,
            learn,
        };
        let id =
            h.db.call(move |c| Ok(topics_repo::upsert(c, &input, "2026-01-01T00:00:00Z")?.id))
                .await
                .unwrap();
        h.news.reload_topics().await.unwrap();
        id
    }

    pub async fn add_feed(h: &Harness, kind: &str, url: &str) -> i64 {
        add_feed_learning(h, kind, url, false).await
    }

    pub async fn add_feed_learning(h: &Harness, kind: &str, url: &str, learning: bool) -> i64 {
        let input = FeedInput {
            id: None,
            kind: kind.into(),
            name: "Example Tech".into(),
            url: url.into(),
            source_weight: 0.5,
            enabled: true,
            learning,
        };
        h.db.call(move |c| Ok(feeds::upsert(c, &input, "2026-01-01T00:00:00Z")?.id))
            .await
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn due_rules() {
        let now = parse_ts("2026-09-29T12:00:00Z").unwrap();
        let mut f = Feed {
            id: 1,
            kind: "rss".into(),
            name: "x".into(),
            url: "https://x".into(),
            source_weight: 0.5,
            enabled: true,
            last_fetched_at: None,
            last_error: None,
            consecutive_failures: 0,
            learning: false,
            etag: None,
            last_modified: None,
        };
        assert!(is_due(&f, now, 20));
        f.last_fetched_at = Some("2026-09-29T11:45:00Z".into());
        assert!(!is_due(&f, now, 20));
        assert!(is_due(&f, now, 15));
        f.consecutive_failures = 1; // 40 min
        f.last_fetched_at = Some("2026-09-29T11:25:00Z".into());
        assert!(!is_due(&f, now, 20));
        f.consecutive_failures = 10; // capped at 6 h
        f.last_fetched_at = Some("2026-09-29T06:00:00Z".into());
        assert!(is_due(&f, now, 20));
    }

    #[tokio::test]
    async fn fetch_cycle_ingests_scores_and_backs_off() {
        let s = MockServer::start().await;
        Mock::given(path("/feed"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/rss2_basic.xml")),
            )
            .mount(&s)
            .await;
        let h = harness("2026-09-29T08:00:00Z").await;
        add_topic(&h, "AI", &["AI"]).await;
        add_feed(&h, "rss", &format!("{}/feed", s.uri())).await;

        assert_eq!(h.news.fetch_cycle(false).await.unwrap(), 2);
        let page =
            h.db.call(|c| articles::list_items(c, &Default::default(), None, 50))
                .await
                .unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(
            page.items[0].title, "New local AI model runs on Apple Silicon",
            "AI story ranks first"
        );
        assert!(page.items[0].score.unwrap() > page.items[1].score.unwrap());

        // Not due again 10 min later; due after 20 min.
        h.clock.advance(Duration::minutes(10));
        assert_eq!(h.news.fetch_cycle(false).await.unwrap(), 0);
        assert!(h.events.names().iter().filter(|n| *n == events::NEWS_UPDATED).count() >= 1);
        h.clock.advance(Duration::minutes(10));
        let before = h.events.names().len();
        h.news.fetch_cycle(false).await.unwrap();
        assert!(h.events.names().len() > before, "fetched again");
    }

    #[tokio::test]
    async fn hibernate_uses_longer_interval_and_catches_up_after_sleep() {
        let s = MockServer::start().await;
        Mock::given(path("/feed"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/rss2_basic.xml")),
            )
            .mount(&s)
            .await;
        let h = harness("2026-09-29T08:00:00Z").await;
        let fid = add_feed(&h, "rss", &format!("{}/feed", s.uri())).await;
        h.mode.set(Mode::Hibernate).await.unwrap();
        h.news.fetch_cycle(false).await.unwrap();
        let last = |h: &Harness| {
            let db = h.db.clone();
            async move { db.call(move |c| feeds::get(c, fid)).await.unwrap().last_fetched_at }
        };
        let first = last(&h).await;
        h.clock.advance(Duration::minutes(30));
        h.news.fetch_cycle(false).await.unwrap();
        assert_eq!(last(&h).await, first, "30 min < 45 min hibernate interval");
        // Mac asleep for 3 hours: first tick after wake fetches.
        h.clock.advance(Duration::hours(3));
        h.news.fetch_cycle(false).await.unwrap();
        assert_ne!(last(&h).await, first);
    }

    #[tokio::test]
    async fn offline_does_not_count_as_feed_failure() {
        let h = harness("2026-09-29T08:00:00Z").await;
        let fid = add_feed(&h, "rss", "http://127.0.0.1:9/feed").await; // nothing listens on port 9
        h.news.fetch_cycle(false).await.unwrap();
        let f = h.db.call(move |c| feeds::get(c, fid)).await.unwrap();
        assert_eq!(f.consecutive_failures, 0);
        assert!(f.last_fetched_at.is_none(), "still due, retried soon");
        // Retry is suppressed until the backoff passes.
        assert_eq!(h.news.fetch_cycle(false).await.unwrap(), 0);
    }

    const LEARNING_XML: &str = include_str!("../../tests/fixtures/rss_learning.xml");

    async fn serve(body: &str) -> MockServer {
        let s = MockServer::start().await;
        Mock::given(path("/feed"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&s)
            .await;
        s
    }

    async fn titles(h: &Harness) -> Vec<String> {
        let mut t: Vec<String> =
            h.db.call(|c| Ok(articles::list_items(c, &Default::default(), None, 50)?.items))
                .await
                .unwrap()
                .into_iter()
                .map(|a| a.title)
                .collect();
        t.sort();
        t
    }

    #[tokio::test]
    async fn learning_feeds_keep_older_items() {
        let s = serve(LEARNING_XML).await;
        let url = format!("{}/feed", s.uri());
        // Normal feed: 7 days.
        let h = harness("2026-09-29T09:00:00Z").await;
        add_feed(&h, "rss", &url).await;
        h.news.fetch_cycle(true).await.unwrap();
        assert_eq!(titles(&h).await.len(), 3, "{:?}", titles(&h).await);
        // Learning feed: up to lessonMaxAgeDays (60), so the 9- and 19-day-old tutorials stay.
        let h = harness("2026-09-29T09:00:00Z").await;
        add_feed_learning(&h, "rss", &url, true).await;
        h.news.fetch_cycle(true).await.unwrap();
        let t = titles(&h).await;
        assert_eq!(t.len(), 5, "{t:?}");
        assert!(t.iter().any(|x| x.starts_with("Kafka explained")));
        assert!(!t.iter().any(|x| x.contains("from 2025")), "120 days is too old");
    }

    async fn learning_score_of(h: &Harness, title: &'static str) -> f64 {
        h.db.call(move |c| {
            Ok(
                c.query_row("SELECT learning_score FROM articles WHERE title = ?1", [title], |r| {
                    r.get(0)
                })?,
            )
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn learning_scores_follow_the_feed_switch() {
        let s = serve(LEARNING_XML).await;
        let h = harness("2026-09-29T09:00:00Z").await;
        add_topic_learn(&h, "Data Engineering", &["data pipeline", "dbt", "Kafka"], true).await;
        let fid = add_feed(&h, "rss", &format!("{}/feed", s.uri())).await;
        h.news.fetch_cycle(true).await.unwrap();
        let dbt = "Understanding dbt tests";
        assert!((learning_score_of(&h, dbt).await - 0.5).abs() < 1e-9);
        assert_eq!(
            learning_score_of(&h, "Snowflake announces new data engineering features").await,
            0.0
        );

        h.db.call(move |c| Ok(c.execute("UPDATE feeds SET learning = 1 WHERE id = ?1", [fid])?))
            .await
            .unwrap();
        h.news.recompute_learning_for_feed(fid).await.unwrap();
        assert!((learning_score_of(&h, dbt).await - 0.85).abs() < 1e-9);
        let lesson: Option<f64> =
            h.db.call(move |c| {
                Ok(
                    c.query_row("SELECT lesson_score FROM articles WHERE title = ?1", [dbt], |r| {
                        r.get(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert!(
            lesson.is_some_and(|l| l > 50.0),
            "lesson score after rescore: {lesson:?}"
        );
    }

    #[tokio::test]
    async fn articles_from_before_the_upgrade_are_backfilled_once() {
        let h = harness("2026-09-29T09:00:00Z").await;
        h.db.call(|c| {
            Ok(c.execute(
                "INSERT INTO articles(url, normalized_url, title, title_key, source_name, discovered_at)
                 VALUES ('https://a/1', 'https://a/1', 'Iceberg explained: a beginner guide', 'k', 's', '2026-09-20T00:00:00Z')",
                [],
            )?)
        })
        .await
        .unwrap();
        assert_eq!(h.news.backfill_learning().await.unwrap(), 1);
        assert!((learning_score_of(&h, "Iceberg explained: a beginner guide").await - 0.75).abs() < 1e-9);
        assert_eq!(h.news.backfill_learning().await.unwrap(), 0, "only once");
    }

    #[tokio::test]
    async fn add_from_example_saves_article_and_feed() {
        let s = serve(LEARNING_XML).await;
        Mock::given(path("/blog/kafka-intro"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"<html><head><title>Kafka 101: a beginner's guide | Data Blog</title>
                   <meta property="og:site_name" content="Data Blog">
                   <meta name="description" content="Streaming with Kafka, step by step.">
                   <link rel="alternate" type="application/rss+xml" href="/feed"></head><body>x</body></html>"#,
                "text/html",
            ))
            .mount(&s)
            .await;
        let h = harness("2026-09-29T09:00:00Z").await;
        add_topic_learn(&h, "Data Engineering", &["Kafka", "streaming"], true).await;
        let page = format!("{}/blog/kafka-intro", s.uri());

        let found = h.news.discover_feeds(&page).await.unwrap();
        assert_eq!(found.candidates.len(), 1);
        assert_eq!(found.page.site_name, "Data Blog");

        let input = |feed: Option<String>, save: bool| AddFromExample {
            url: page.clone(),
            feed_url: feed,
            name: "Data Learning".into(),
            learning: true,
            save_article: save,
        };
        let out = h
            .news
            .add_from_example(input(Some(found.candidates[0].url.clone()), true))
            .await
            .unwrap();
        let feed = out.feed.unwrap();
        assert!(feed.learning);
        assert!((feed.source_weight - EXAMPLE_SOURCE_WEIGHT).abs() < 1e-9);
        let a = out.article.unwrap();
        assert!(a.saved);
        assert_eq!(a.source_name, "Data Blog");
        assert_eq!(a.topics, vec!["Data Engineering"]);
        // 101 + beginner + guide in the title, "step by step" in the description; no feed source.
        assert!((a.learning_score.unwrap() - 0.75).abs() < 1e-9);

        // Once the body is extracted, the long-article bonus applies.
        let text = "Kafka keeps streams of events in topics. ".repeat(200);
        h.news
            .save_article_body(SaveBody {
                article_id: a.id,
                text: Some(text),
                html: Some("<p>…</p>".into()),
                canonical_url: None,
                paywall_hint: false,
                failed: false,
            })
            .await
            .unwrap();
        let again = h.db.call(move |c| articles::get_item(c, a.id)).await.unwrap();
        assert!((again.learning_score.unwrap() - 0.85).abs() < 1e-9);

        let dup = h
            .news
            .add_from_example(input(Some(format!("{}/feed/", s.uri())), false))
            .await
            .unwrap_err();
        assert_eq!(dup.to_string(), "This source is already in your list.");
        let only_article = h.news.add_from_example(input(None, true)).await.unwrap();
        assert_eq!(only_article.article.unwrap().id, a.id, "same article, not a copy");
        assert!(h.news.add_from_example(input(None, false)).await.is_err());
    }

    #[tokio::test]
    async fn unreadable_articles_are_hidden() {
        let s = MockServer::start().await;
        Mock::given(path("/feed"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/rss2_basic.xml")),
            )
            .mount(&s)
            .await;
        let h = harness("2026-09-29T08:00:00Z").await;
        add_topic(&h, "Data", &["Kafka", "streaming", "AI"]).await;
        add_feed(&h, "rss", &format!("{}/feed", s.uri())).await;
        h.news.fetch_cycle(false).await.unwrap();
        assert!(
            h.events.names().iter().any(|n| n == events::CHECK_BODIES),
            "new articles get checked"
        );

        let list = || h.db.call(|c| articles::list_items(c, &Default::default(), None, 50));
        let before = list().await.unwrap().items;
        let id = before[0].id;
        let saved = h
            .news
            .save_article_body(SaveBody {
                article_id: id,
                text: None,
                html: None,
                canonical_url: None,
                paywall_hint: false,
                failed: true,
            })
            .await
            .unwrap();
        assert!(saved.hidden);
        let after = list().await.unwrap().items;
        assert_eq!(after.len(), before.len() - 1);
        assert!(
            after.iter().all(|a| a.id != id),
            "a story the app can't read is not shown"
        );
    }

    #[tokio::test]
    async fn not_interested_hides_and_lowers_affinity() {
        let s = MockServer::start().await;
        Mock::given(path("/feed"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/rss2_basic.xml")),
            )
            .mount(&s)
            .await;
        let h = harness("2026-09-29T08:00:00Z").await;
        let tid = add_topic(&h, "Data", &["Kafka", "streaming", "AI"]).await;
        add_feed(&h, "rss", &format!("{}/feed", s.uri())).await;
        h.news.fetch_cycle(false).await.unwrap();
        let page =
            h.db.call(|c| articles::list_items(c, &Default::default(), None, 50))
                .await
                .unwrap();
        let (a, b) = (page.items[0].clone(), page.items[1].clone());
        let before = b.breakdown.as_ref().unwrap().user_history;
        h.news
            .record_interaction(a.id, InteractionKind::NotInterested)
            .await
            .unwrap();
        let page =
            h.db.call(|c| articles::list_items(c, &Default::default(), None, 50))
                .await
                .unwrap();
        assert_eq!(page.items.len(), 1, "hidden article disappears");
        let after = page.items[0].breakdown.as_ref().unwrap().user_history;
        assert!(after < before, "affinity lowered: {before} → {after}");
        let aff = h.db.call(|c| interactions::all_affinities(c)).await.unwrap();
        assert!(aff[&("topic".to_string(), tid)] < 0.0);
    }
}
