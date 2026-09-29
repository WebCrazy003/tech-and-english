pub mod hackernews;
pub mod ingest;
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
use model::InteractionKind;
use normalize::{jaccard, word_set};
use ranking::{RankInputs, affinity_deltas};
use source::FetchCtx;
use topics::CompiledTopic;

/// Articles discovered within this window are rescored every cycle (freshness changes).
pub const RESCORE_WINDOW_DAYS: i64 = 7;
const MAX_BACKOFF: Duration = Duration::hours(6);
const OFFLINE_RETRY_SECS: [i64; 3] = [30, 60, 120];

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
        }))
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    fn compiled_topics(&self) -> Arc<Vec<CompiledTopic>> {
        self.topics.read().unwrap().clone()
    }

    /// Recompile topics after an edit, re-match recent articles and rescore.
    pub async fn reload_topics(&self) -> AppResult<()> {
        let list = self.db.call(|c| topics_repo::list(c)).await?;
        let compiled = Arc::new(topics::compile(&list));
        *self.topics.write().unwrap() = compiled.clone();
        let since = fmt_ts(self.clock.now() - Duration::days(RESCORE_WINDOW_DAYS));
        self.db
            .tx(move |tx| {
                let ids = articles::ids_discovered_since(tx, &since)?;
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
        let max_age = settings.ingest_max_age_days;
        let (new_count, ok_count) = self
            .db
            .tx(move |tx| {
                let now_s = fmt_ts(now);
                let mut recent = RecentTitles::load(tx, now)?;
                let (mut new, mut ok) = (0u32, 0u32);
                for (feed, r) in results {
                    match r {
                        Ok(res) => {
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
                Ok(())
            })
            .await
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
        let input = TopicInput {
            id: None,
            name: name.into(),
            keywords: kws.iter().map(|s| s.to_string()).collect(),
            excluded_keywords: vec![],
            priority: 3,
            enabled: true,
            notify: true,
            notify_threshold: None,
        };
        let id =
            h.db.call(move |c| Ok(topics_repo::upsert(c, &input, "2026-01-01T00:00:00Z")?.id))
                .await
                .unwrap();
        h.news.reload_topics().await.unwrap();
        id
    }

    pub async fn add_feed(h: &Harness, kind: &str, url: &str) -> i64 {
        let input = FeedInput {
            id: None,
            kind: kind.into(),
            name: "Example Tech".into(),
            url: url.into(),
            source_weight: 0.5,
            enabled: true,
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
