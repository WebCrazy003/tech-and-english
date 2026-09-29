//! Daily pick (SPEC §7.8).

use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use super::NewsService;
use super::model::InteractionKind;
use super::why;
use crate::clock::{Clock, fmt_ts, parse_hhmm};
use crate::db::Db;
use crate::db::repo::articles::{self, ArticleListItem};
use crate::db::repo::{interactions, picks};
use crate::error::AppResult;
use crate::events::{self, EventSink};
use crate::notify::NotifyService;
use crate::settings::SettingsStore;

pub const MIN_RELEVANCE: f64 = 0.3;
pub const REPEAT_EXCLUSION_DAYS: i64 = 14;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyPick {
    pub date: String,
    pub article: ArticleListItem,
    pub why: String,
}

/// Best eligible article at `now`: relevance ≥ 0.3, age ≤ `max_age_hours`, not hidden,
/// not paywalled, not picked in the last 14 days. Highest score wins; ties → newest.
pub fn best_candidate(
    conn: &Connection,
    now: DateTime<Utc>,
    today: NaiveDate,
    max_age_hours: i64,
) -> AppResult<Option<i64>> {
    let cutoff = fmt_ts(now - Duration::hours(max_age_hours));
    let since_date = (today - Duration::days(REPEAT_EXCLUSION_DAYS)).to_string();
    Ok(conn
        .query_row(
            "SELECT a.id FROM articles a
             WHERE a.hidden = 0 AND a.body_status != 'paywalled' AND a.score IS NOT NULL
               AND COALESCE((SELECT MAX(relevance) FROM article_topics WHERE article_id = a.id), 0) >= ?1
               AND COALESCE(a.published_at, a.discovered_at) >= ?2
               AND a.id NOT IN (SELECT article_id FROM daily_picks WHERE date >= ?3)
             ORDER BY a.score DESC, a.discovered_at DESC, a.id DESC LIMIT 1",
            rusqlite::params![MIN_RELEVANCE, cutoff, since_date],
            |r| r.get(0),
        )
        .optional()?)
}

/// 48 h window first, then relaxed to 72 h.
pub fn select(conn: &Connection, now: DateTime<Utc>, today: NaiveDate) -> AppResult<Option<i64>> {
    match best_candidate(conn, now, today, 48)? {
        Some(id) => Ok(Some(id)),
        None => best_candidate(conn, now, today, 72),
    }
}

fn why_for(conn: &Connection, article_id: i64, hn_points: Option<i64>) -> AppResult<String> {
    let mut st = conn.prepare(
        "SELECT t.name, at.relevance FROM article_topics at JOIN topics t ON t.id = at.topic_id WHERE at.article_id = ?1",
    )?;
    let topics = st
        .query_map([article_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let aff = interactions::all_affinities(conn)?;
    let source_aff = articles::source_feed_ids(conn, article_id)?
        .iter()
        .filter_map(|f| aff.get(&("source".to_string(), *f)).copied())
        .fold(0.0, f64::max);
    Ok(why::template(&topics, hn_points, source_aff))
}

/// Today's pick, unless the user marked it "not interested" (then the UI shows the next best story).
fn load_pick(conn: &Connection, date: &str) -> AppResult<Option<DailyPick>> {
    let Some(row) = picks::get(conn, date)? else {
        return Ok(None);
    };
    if articles::get_item(conn, row.article_id)?.hidden {
        return Ok(None);
    }
    Ok(Some(DailyPick {
        date: row.date,
        article: articles::get_item(conn, row.article_id)?,
        why: row.why,
    }))
}

pub struct PickService {
    db: Db,
    clock: Arc<dyn Clock>,
    settings: Arc<SettingsStore>,
    events: Arc<dyn EventSink>,
    news: Arc<NewsService>,
    notify: Arc<NotifyService>,
}

impl PickService {
    pub fn new(
        db: Db,
        clock: Arc<dyn Clock>,
        settings: Arc<SettingsStore>,
        events: Arc<dyn EventSink>,
        news: Arc<NewsService>,
        notify: Arc<NotifyService>,
    ) -> Self {
        Self {
            db,
            clock,
            settings,
            events,
            news,
            notify,
        }
    }

    pub async fn today(&self) -> AppResult<Option<DailyPick>> {
        let date = self.clock.today_local().to_string();
        self.db.call(move |c| load_pick(c, &date)).await
    }

    /// Best candidate right now, without saving it (shown before pick time).
    pub async fn preview(&self) -> AppResult<Option<ArticleListItem>> {
        let (now, today) = (self.clock.now(), self.clock.today_local());
        self.db
            .call(move |c| match select(c, now, today)? {
                Some(id) => Ok(Some(articles::get_item(c, id)?)),
                None => Ok(None),
            })
            .await
    }

    /// Make today's pick if it's time and none exists. Returns the pick when newly created.
    pub async fn ensure_today(&self) -> AppResult<Option<DailyPick>> {
        let settings = self.settings.get();
        if !settings.onboarding_done {
            return Ok(None);
        }
        let now_local = self.clock.now_local();
        let pick_time = parse_hhmm(&settings.pick_time).unwrap_or_default();
        if now_local.time() < pick_time {
            return Ok(None);
        }
        let (now, today) = (self.clock.now(), now_local.date_naive());
        let date = today.to_string();
        let created = self
            .db
            .tx(move |tx| {
                if picks::get(tx, &date)?.is_some() {
                    return Ok(None);
                }
                let Some(id) = select(tx, now, today)? else {
                    return Ok(None);
                };
                // P2: body extraction / paywall retry / difficulty happen here (post_select).
                let item = articles::get_item(tx, id)?;
                let why = why_for(tx, id, item.hn_points)?;
                picks::insert(tx, &date, id, &why, &fmt_ts(now))?;
                Ok(Some(DailyPick {
                    date,
                    article: item,
                    why,
                }))
            })
            .await?;
        if let Some(p) = &created {
            tracing::info!(article = p.article.id, "daily pick created");
            events::emit(self.events.as_ref(), events::PICK_CHANGED, p);
            if let Err(e) = self.notify.daily_pick(&p.article).await {
                tracing::warn!(error = %e, "daily pick notification failed");
            }
        }
        Ok(created)
    }

    /// Record "skipped" (once) if yesterday's pick was never opened.
    pub async fn mark_yesterday_skipped(&self) -> AppResult<()> {
        let yesterday = (self.clock.today_local() - Duration::days(1)).to_string();
        let target = self
            .db
            .call(move |c| {
                let Some(p) = picks::get(c, &yesterday)? else {
                    return Ok(None);
                };
                let opened = interactions::has(c, p.article_id, "opened")?;
                let skipped = interactions::has(c, p.article_id, "skipped")?;
                Ok((!opened && !skipped).then_some(p.article_id))
            })
            .await?;
        if let Some(id) = target {
            self.news.record_interaction(id, InteractionKind::Skipped).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news::testutil::*;
    use crate::notify::RecordingNotifier;
    use serde_json::json;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn setup(now: &str) -> (Harness, PickService, Arc<RecordingNotifier>, MockServer) {
        let s = MockServer::start().await;
        Mock::given(path("/feed"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/rss2_basic.xml")),
            )
            .mount(&s)
            .await;
        let h = harness(now).await;
        add_topic(&h, "AI", &["AI"]).await;
        add_feed(&h, "rss", &format!("{}/feed", s.uri())).await;
        let notifier = Arc::new(RecordingNotifier::default());
        let notify = Arc::new(NotifyService::new(
            h.db.clone(),
            h.clock.clone(),
            h.settings.clone(),
            notifier.clone(),
        ));
        let pick = PickService::new(
            h.db.clone(),
            h.clock.clone(),
            h.settings.clone(),
            h.events.clone(),
            h.news.clone(),
            notify,
        );
        (h, pick, notifier, s)
    }

    #[tokio::test]
    async fn waits_for_pick_time_then_picks_once() {
        let (h, pick, notifier, _s) = setup("2026-09-29T07:30:00Z").await;
        h.news.fetch_cycle(true).await.unwrap();
        assert!(pick.ensure_today().await.unwrap().is_none(), "before 08:00");
        assert!(pick.preview().await.unwrap().is_some(), "preview available");
        h.clock.advance(Duration::minutes(31));
        let p = pick.ensure_today().await.unwrap().expect("picked");
        assert_eq!(p.article.title, "New local AI model runs on Apple Silicon");
        assert!(p.why.starts_with("Matches your topics: AI"));
        assert!(pick.ensure_today().await.unwrap().is_none(), "only once per day");
        assert_eq!(notifier.sent.lock().unwrap().len(), 1);
        assert_eq!(pick.today().await.unwrap().unwrap().article.id, p.article.id);
    }

    #[tokio::test]
    async fn relaxes_to_72h_then_gives_up() {
        // Fixture AI item published 2026-09-29T06:00Z.
        let (h, pick, _n, _s) = setup("2026-09-29T09:00:00Z").await;
        h.news.fetch_cycle(true).await.unwrap();
        h.clock.set(crate::clock::parse_ts("2026-10-01T10:00:00Z").unwrap()); // 52 h later
        assert!(pick.ensure_today().await.unwrap().is_some(), "72 h fallback");
        h.clock.set(crate::clock::parse_ts("2026-10-02T10:00:00Z").unwrap()); // 76 h later
        assert!(pick.ensure_today().await.unwrap().is_none(), "nothing eligible");
    }

    #[tokio::test]
    async fn hidden_and_recent_picks_are_excluded() {
        let (h, pick, _n, _s) = setup("2026-09-29T09:00:00Z").await;
        h.news.fetch_cycle(true).await.unwrap();
        let first = pick.ensure_today().await.unwrap().unwrap();
        // Next day: the same article can't be picked again (14-day rule).
        h.clock.advance(Duration::days(1));
        let second = pick.ensure_today().await.unwrap();
        assert!(second.is_none_or(|p| p.article.id != first.article.id));
    }

    #[tokio::test]
    async fn not_interested_article_is_never_picked() {
        let (h, pick, _n, _s) = setup("2026-09-29T09:00:00Z").await;
        h.news.fetch_cycle(true).await.unwrap();
        let preview = pick.preview().await.unwrap().unwrap();
        h.news
            .record_interaction(preview.id, InteractionKind::NotInterested)
            .await
            .unwrap();
        assert!(
            pick.ensure_today()
                .await
                .unwrap()
                .is_none_or(|p| p.article.id != preview.id)
        );
    }

    #[tokio::test]
    async fn hidden_pick_is_not_returned() {
        let (h, pick, _n, _s) = setup("2026-09-29T09:00:00Z").await;
        h.news.fetch_cycle(true).await.unwrap();
        let p = pick.ensure_today().await.unwrap().unwrap();
        h.news
            .record_interaction(p.article.id, InteractionKind::NotInterested)
            .await
            .unwrap();
        assert!(pick.today().await.unwrap().is_none());
        assert!(
            pick.ensure_today().await.unwrap().is_none(),
            "no second pick the same day"
        );
    }

    #[tokio::test]
    async fn skipped_marked_once_when_not_opened() {
        let (h, pick, _n, _s) = setup("2026-09-29T09:00:00Z").await;
        h.news.fetch_cycle(true).await.unwrap();
        let p = pick.ensure_today().await.unwrap().unwrap();
        h.clock.advance(Duration::days(1));
        pick.mark_yesterday_skipped().await.unwrap();
        pick.mark_yesterday_skipped().await.unwrap();
        let id = p.article.id;
        let n: i64 =
            h.db.call(move |c| {
                Ok(c.query_row(
                    "SELECT count(*) FROM article_interactions WHERE article_id = ?1 AND kind = 'skipped'",
                    [id],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn no_pick_before_onboarding() {
        let (h, pick, _n, _s) = setup("2026-09-29T09:00:00Z").await;
        h.news.fetch_cycle(true).await.unwrap();
        h.settings.update(json!({ "onboardingDone": false })).await.unwrap();
        assert!(pick.ensure_today().await.unwrap().is_none());
    }
}
