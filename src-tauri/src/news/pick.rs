//! Daily pick (SPEC §7.8): today's story, and from P3 today's lesson (SPEC §7.11).

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use super::NewsService;
use super::body::BodyWaiters;
use super::learning::{LESSON_THRESHOLD, LearningInput, lesson_kind};
use super::model::InteractionKind;
use super::why;
use crate::clock::{Clock, fmt_ts, parse_hhmm};
use crate::db::Db;
use crate::db::repo::articles::{self, ArticleListItem};
use crate::db::repo::interactions;
use crate::db::repo::picks::{self, LESSON, STORY};
use crate::error::AppResult;
use crate::events::{self, EventSink};
use crate::notify::NotifyService;
use crate::settings::SettingsStore;

pub const MIN_RELEVANCE: f64 = 0.3;
pub const REPEAT_EXCLUSION_DAYS: i64 = 14;
/// A lesson is never an article picked (as story or lesson) in the last 60 days.
pub const LESSON_REPEAT_EXCLUSION_DAYS: i64 = 60;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyPick {
    pub date: String,
    /// "story" | "lesson"
    pub kind: String,
    pub article: ArticleListItem,
    pub why: String,
}

/// Candidates tried in order when the pick is made (the next one is used if a story is paywalled).
pub const PICK_ATTEMPTS: usize = 3;
/// How long the pick waits for the frontend to extract a body.
pub const BODY_WAIT: StdDuration = StdDuration::from_secs(30);

/// Best eligible articles at `now`: relevance ≥ 0.3, age ≤ `max_age_hours`, not hidden,
/// not paywalled, not picked in the last 14 days. Highest score first; ties → newest.
pub fn best_candidates(
    conn: &Connection,
    now: DateTime<Utc>,
    today: NaiveDate,
    max_age_hours: i64,
    limit: usize,
) -> AppResult<Vec<i64>> {
    let cutoff = fmt_ts(now - Duration::hours(max_age_hours));
    let since_date = (today - Duration::days(REPEAT_EXCLUSION_DAYS)).to_string();
    let mut st = conn.prepare(
        "SELECT a.id FROM articles a
         WHERE a.hidden = 0 AND a.body_status != 'paywalled' AND a.score IS NOT NULL
           AND COALESCE((SELECT MAX(relevance) FROM article_topics WHERE article_id = a.id), 0) >= ?1
           AND COALESCE(a.published_at, a.discovered_at) >= ?2
           AND a.id NOT IN (SELECT article_id FROM daily_picks WHERE date >= ?3)
         ORDER BY a.score DESC, a.discovered_at DESC, a.id DESC LIMIT ?4",
    )?;
    let rows = st
        .query_map(
            rusqlite::params![MIN_RELEVANCE, cutoff, since_date, limit as i64],
            |r| r.get(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 48 h window first, then relaxed to 72 h.
pub fn candidates(conn: &Connection, now: DateTime<Utc>, today: NaiveDate, limit: usize) -> AppResult<Vec<i64>> {
    let first = best_candidates(conn, now, today, 48, limit)?;
    if first.is_empty() {
        best_candidates(conn, now, today, 72, limit)
    } else {
        Ok(first)
    }
}

pub fn select(conn: &Connection, now: DateTime<Utc>, today: NaiveDate) -> AppResult<Option<i64>> {
    Ok(candidates(conn, now, today, 1)?.into_iter().next())
}

pub fn has_learn_topics(conn: &Connection) -> AppResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM topics WHERE learn = 1 AND enabled = 1)",
        [],
        |r| r.get(0),
    )?)
}

/// Lesson candidates (SPEC §7.11): learning material that matches a Learn topic, at most
/// `max_age_days` old, not hidden or paywalled, not `exclude` (today's story), and not a pick
/// of either kind in the last 60 days. Best lesson score first; ties → newest.
pub fn lesson_candidates(
    conn: &Connection,
    now: DateTime<Utc>,
    today: NaiveDate,
    max_age_days: u32,
    exclude: Option<i64>,
    limit: usize,
) -> AppResult<Vec<i64>> {
    let cutoff = fmt_ts(now - Duration::days(max_age_days as i64));
    let since_date = (today - Duration::days(LESSON_REPEAT_EXCLUSION_DAYS)).to_string();
    let mut st = conn.prepare(
        "SELECT a.id FROM articles a
         WHERE a.hidden = 0 AND a.body_status != 'paywalled' AND a.lesson_score IS NOT NULL
           AND a.learning_score >= ?1
           AND EXISTS (SELECT 1 FROM article_topics at JOIN topics t ON t.id = at.topic_id
                       WHERE at.article_id = a.id AND t.learn = 1 AND t.enabled = 1 AND at.relevance >= ?2)
           AND COALESCE(a.published_at, a.discovered_at) >= ?3
           AND a.id IS NOT ?4
           AND a.id NOT IN (SELECT article_id FROM daily_picks WHERE date >= ?5)
         ORDER BY a.lesson_score DESC, a.discovered_at DESC, a.id DESC LIMIT ?6",
    )?;
    let rows = st
        .query_map(
            rusqlite::params![
                LESSON_THRESHOLD,
                MIN_RELEVANCE,
                cutoff,
                exclude,
                since_date,
                limit as i64
            ],
            |r| r.get(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
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

/// "Tutorial · Data Engineering · from a learning source"
fn lesson_why_for(conn: &Connection, article_id: i64) -> AppResult<String> {
    let (title, description, word_count, feed_learning) = articles::learning_inputs(conn, article_id)?;
    let kind = lesson_kind(&LearningInput {
        title: &title,
        description: description.as_deref(),
        feed_learning,
        word_count,
    });
    let topic: Option<String> = conn
        .query_row(
            "SELECT t.name FROM article_topics at JOIN topics t ON t.id = at.topic_id
             WHERE at.article_id = ?1 AND t.learn = 1 AND t.enabled = 1 ORDER BY at.relevance DESC LIMIT 1",
            [article_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(why::lesson(kind.label(), topic.as_deref(), feed_learning))
}

/// Today's pick of `kind`, unless the user marked it "not interested"
/// (then the UI shows the next best story, or no lesson).
fn load_pick(conn: &Connection, date: &str, kind: &str) -> AppResult<Option<DailyPick>> {
    let Some(row) = picks::get(conn, date, kind)? else {
        return Ok(None);
    };
    let article = articles::get_item(conn, row.article_id)?;
    if article.hidden {
        return Ok(None);
    }
    Ok(Some(DailyPick {
        date: row.date,
        kind: row.kind,
        article,
        why: row.why,
    }))
}

/// Insert a pick unless one of this kind exists already (another tick won the race).
fn insert_pick(conn: &Connection, date: &str, kind: &str, article_id: i64, now: &str) -> AppResult<Option<DailyPick>> {
    if picks::get(conn, date, kind)?.is_some() {
        return Ok(None);
    }
    let item = articles::get_item(conn, article_id)?;
    let why = if kind == LESSON {
        lesson_why_for(conn, article_id)?
    } else {
        why_for(conn, article_id, item.hn_points)?
    };
    picks::insert(conn, date, kind, article_id, &why, now)?;
    Ok(Some(DailyPick {
        date: date.to_string(),
        kind: kind.to_string(),
        article: item,
        why,
    }))
}

pub struct PickService {
    db: Db,
    clock: Arc<dyn Clock>,
    settings: Arc<SettingsStore>,
    events: Arc<dyn EventSink>,
    news: Arc<NewsService>,
    notify: Arc<NotifyService>,
    /// Stops two ticks (or a tick and a manual refresh) from picking at the same time.
    pick_lock: tokio::sync::Mutex<()>,
    body_wait: StdDuration,
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
            pick_lock: tokio::sync::Mutex::new(()),
            body_wait: BODY_WAIT,
        }
    }

    /// Shorter body wait (tests).
    pub fn with_body_wait(mut self, d: StdDuration) -> Self {
        self.body_wait = d;
        self
    }

    /// Body status of an article, asking the frontend to extract it if needed.
    /// `None` if the frontend didn't answer in time.
    async fn ensure_body(&self, article_id: i64) -> AppResult<Option<String>> {
        let status = self
            .db
            .call(move |c| Ok(articles::get_item(c, article_id)?.body_status))
            .await?;
        if status != "none" {
            return Ok(Some(status));
        }
        let rx = self.news.body_waiters.register(article_id);
        events::emit(
            self.events.as_ref(),
            events::NEEDS_BODY,
            &serde_json::json!({ "articleId": article_id }),
        );
        Ok(BodyWaiters::wait(rx, self.body_wait).await)
    }

    /// P2: make sure the body is extracted; skip paywalled candidates.
    async fn first_readable(&self, cands: &[i64]) -> AppResult<i64> {
        for id in cands {
            if self.ensure_body(*id).await?.as_deref() != Some("paywalled") {
                return Ok(*id);
            }
        }
        Ok(cands[0])
    }

    pub async fn today(&self) -> AppResult<Option<DailyPick>> {
        let date = self.clock.today_local().to_string();
        self.db.call(move |c| load_pick(c, &date, STORY)).await
    }

    pub async fn today_lesson(&self) -> AppResult<Option<DailyPick>> {
        let date = self.clock.today_local().to_string();
        self.db.call(move |c| load_pick(c, &date, LESSON)).await
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

    /// Make today's story and lesson if it's time and they don't exist yet.
    /// Returns the story when it is newly created. One notification per day, sent with the story.
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
        let _guard = self.pick_lock.lock().await;
        let (now, today) = (self.clock.now(), now_local.date_naive());
        let date = today.to_string();

        let story = self.ensure_story(now, today, &date).await?;
        let lesson = self
            .ensure_lesson(now, today, &date, settings.lesson_max_age_days)
            .await?;

        if let Some(p) = &story {
            tracing::info!(article = p.article.id, "daily pick created");
            events::emit(self.events.as_ref(), events::PICK_CHANGED, p);
        }
        if let Some(l) = &lesson {
            tracing::info!(article = l.article.id, "daily lesson created");
            events::emit(self.events.as_ref(), events::PICK_CHANGED, l);
        }
        if let Some(p) = &story {
            let d = date.clone();
            let todays_lesson = self.db.call(move |c| load_pick(c, &d, LESSON)).await?;
            if let Err(e) = self
                .notify
                .daily_pick(&p.article, todays_lesson.as_ref().map(|l| &l.article))
                .await
            {
                tracing::warn!(error = %e, "daily pick notification failed");
            }
        }
        Ok(story)
    }

    async fn ensure_story(&self, now: DateTime<Utc>, today: NaiveDate, date: &str) -> AppResult<Option<DailyPick>> {
        let d = date.to_string();
        let (exists, cands) = self
            .db
            .call(move |c| {
                Ok((
                    picks::get(c, &d, STORY)?.is_some(),
                    candidates(c, now, today, PICK_ATTEMPTS)?,
                ))
            })
            .await?;
        if exists || cands.is_empty() {
            return Ok(None);
        }
        let id = self.first_readable(&cands).await?;
        let d = date.to_string();
        self.db.tx(move |tx| insert_pick(tx, &d, STORY, id, &fmt_ts(now))).await
    }

    async fn ensure_lesson(
        &self,
        now: DateTime<Utc>,
        today: NaiveDate,
        date: &str,
        max_age_days: u32,
    ) -> AppResult<Option<DailyPick>> {
        let d = date.to_string();
        let cands = self
            .db
            .call(move |c| {
                if picks::get(c, &d, LESSON)?.is_some() || !has_learn_topics(c)? {
                    return Ok(Vec::new());
                }
                let story = picks::get(c, &d, STORY)?.map(|p| p.article_id);
                lesson_candidates(c, now, today, max_age_days, story, PICK_ATTEMPTS)
            })
            .await?;
        if cands.is_empty() {
            return Ok(None);
        }
        let id = self.first_readable(&cands).await?;
        let d = date.to_string();
        self.db
            .tx(move |tx| insert_pick(tx, &d, LESSON, id, &fmt_ts(now)))
            .await
    }

    /// Record "skipped" (once) for yesterday's story and lesson if they were never opened.
    pub async fn mark_yesterday_skipped(&self) -> AppResult<()> {
        let yesterday = (self.clock.today_local() - Duration::days(1)).to_string();
        let targets = self
            .db
            .call(move |c| {
                let mut out = Vec::new();
                for kind in [STORY, LESSON] {
                    let Some(p) = picks::get(c, &yesterday, kind)? else {
                        continue;
                    };
                    let opened = interactions::has(c, p.article_id, "opened")?;
                    let skipped = interactions::has(c, p.article_id, "skipped")?;
                    if !opened && !skipped {
                        out.push(p.article_id);
                    }
                }
                Ok(out)
            })
            .await?;
        for id in targets {
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
        )
        .with_body_wait(std::time::Duration::from_millis(30));
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

    /// Acts like the widget: on `article://needs-body`, "extracts" a body. The first article
    /// asked for is paywalled, the others are fine.
    fn fake_frontend(h: &Harness) {
        let news = h.news.clone();
        let asked = Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
        *h.events.hook.lock().unwrap() = Some(Box::new(move |event, payload| {
            if event != events::NEEDS_BODY {
                return;
            }
            let id = payload["articleId"].as_i64().unwrap();
            let first = {
                let mut a = asked.lock().unwrap();
                a.push(id);
                a.len() == 1
            };
            let news = news.clone();
            tokio::spawn(async move {
                let words = if first { 300 } else { 900 };
                let text = "Local models run fast on laptops today. ".repeat(words / 7);
                news.save_article_body(crate::news::SaveBody {
                    article_id: id,
                    text: Some(text),
                    html: Some("<p>…</p>".into()),
                    canonical_url: None,
                    paywall_hint: first,
                    failed: false,
                })
                .await
                .unwrap();
            });
        }));
    }

    #[tokio::test]
    async fn paywalled_candidate_is_skipped_and_difficulty_filled() {
        let (h, pick, _n, _s) = setup("2026-09-29T09:00:00Z").await;
        h.settings
            .update(
                json!({ "rankingWeights": { "topicRelevance": 0.0, "freshness": 0.45, "popularity": 0.15,
                "sourcePreference": 0.2, "novelty": 0.1, "userHistory": 0.1 } }),
            )
            .await
            .unwrap();
        // Make both fixture stories eligible (AI + Kafka topics).
        add_topic(&h, "Data", &["Kafka"]).await;
        h.news.fetch_cycle(true).await.unwrap();
        fake_frontend(&h);
        let pick = pick.with_body_wait(std::time::Duration::from_secs(5));
        let p = pick.ensure_today().await.unwrap().expect("picked");
        assert_eq!(p.article.body_status, "ok", "second candidate chosen");
        assert!(p.article.difficulty.is_some());
        assert!(p.article.reading_minutes.unwrap() >= 5);
        let paywalled: i64 =
            h.db.call(|c| {
                Ok(c.query_row(
                    "SELECT count(*) FROM articles WHERE body_status = 'paywalled'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(paywalled, 1);
    }

    #[tokio::test]
    async fn body_timeout_still_picks() {
        let (h, pick, _n, _s) = setup("2026-09-29T09:00:00Z").await;
        h.news.fetch_cycle(true).await.unwrap();
        let p = pick.ensure_today().await.unwrap().expect("picked without a body");
        assert_eq!(p.article.body_status, "none");
        assert!(h.events.names().contains(&events::NEEDS_BODY.to_string()));
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

    // ------------------------------------------------------------ lessons (P3)

    const DE: &[&str] = &[
        "data pipeline",
        "Kafka",
        "dbt",
        "Airflow",
        "data engineering",
        "streaming",
    ];

    /// Story feed + a learning feed with tutorials, news and spam (tests/fixtures/rss_learning.xml).
    async fn setup_lessons(learn: bool) -> (Harness, PickService, Arc<RecordingNotifier>, MockServer, i64) {
        let (h, pick, notifier, s) = setup("2026-09-29T09:00:00Z").await;
        Mock::given(path("/learn"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/rss_learning.xml")),
            )
            .mount(&s)
            .await;
        let de = add_topic_learn(&h, "Data Engineering", DE, learn).await;
        add_feed_learning(&h, "rss", &format!("{}/learn", s.uri()), true).await;
        h.news.fetch_cycle(true).await.unwrap();
        (h, pick, notifier, s, de)
    }

    const TUTORIALS: &[&str] = &[
        "How to build a data pipeline with Airflow: a step-by-step tutorial",
        "Kafka explained: a beginner's guide to streaming",
        "Understanding dbt tests",
    ];

    #[tokio::test]
    async fn lesson_is_chosen_and_never_the_story() {
        let (_h, pick, notifier, _s, _) = setup_lessons(true).await;
        let story = pick.ensure_today().await.unwrap().expect("story");
        let lesson = pick.today_lesson().await.unwrap().expect("lesson");
        assert_eq!((story.kind.as_str(), lesson.kind.as_str()), ("story", "lesson"));
        assert_ne!(story.article.id, lesson.article.id);
        assert!(
            TUTORIALS.contains(&lesson.article.title.as_str()),
            "{}",
            lesson.article.title
        );
        assert!(
            lesson.why.ends_with("· Data Engineering · from a learning source"),
            "{}",
            lesson.why
        );
        let sent = notifier.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "one daily notification");
        assert_eq!(sent[0].0, "Today's story and lesson are ready.");
        assert!(sent[0].1.contains(&format!("Lesson: \"{}\"", lesson.article.title)));
        assert!(pick.ensure_today().await.unwrap().is_none());
        assert_eq!(notifier.sent.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn news_and_spam_are_never_lessons_and_lessons_do_not_repeat() {
        let (h, pick, _n, _s, _) = setup_lessons(true).await;
        let mut seen = Vec::new();
        for _ in 0..6 {
            pick.ensure_today().await.unwrap();
            if let Some(l) = pick.today_lesson().await.unwrap() {
                assert!(TUTORIALS.contains(&l.article.title.as_str()), "{}", l.article.title);
                assert!(!seen.contains(&l.article.id), "repeated within 60 days");
                seen.push(l.article.id);
            }
            h.clock.advance(Duration::days(1));
        }
        assert!(seen.len() >= 2, "{seen:?}");
        assert!(pick.today_lesson().await.unwrap().is_none(), "pool used up");
    }

    #[tokio::test]
    async fn no_learn_topic_means_no_lesson() {
        let (_h, pick, notifier, _s, _) = setup_lessons(false).await;
        pick.ensure_today().await.unwrap().expect("story");
        assert!(pick.today_lesson().await.unwrap().is_none());
        assert_eq!(notifier.sent.lock().unwrap()[0].0, "Today's tech story is ready.");
    }

    #[tokio::test]
    async fn a_lesson_found_later_is_added_quietly() {
        let (h, pick, notifier, _s, de) = setup_lessons(false).await;
        pick.ensure_today().await.unwrap().expect("story");
        h.db.call(move |c| Ok(c.execute("UPDATE topics SET learn = 1 WHERE id = ?1", [de])?))
            .await
            .unwrap();
        h.news.reload_topics().await.unwrap();
        assert!(pick.ensure_today().await.unwrap().is_none(), "no second story");
        let lesson = pick.today_lesson().await.unwrap().expect("lesson added later");
        assert_eq!(notifier.sent.lock().unwrap().len(), 1, "no second notification");
        let lesson_events = h
            .events
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, p)| n == events::PICK_CHANGED && p["kind"] == "lesson")
            .count();
        assert_eq!(lesson_events, 1, "the UI hears about it");
        assert!(TUTORIALS.contains(&lesson.article.title.as_str()));
    }

    #[tokio::test]
    async fn unopened_lesson_is_marked_skipped() {
        let (h, pick, _n, _s, _) = setup_lessons(true).await;
        pick.ensure_today().await.unwrap();
        let id = pick.today_lesson().await.unwrap().unwrap().article.id;
        h.clock.advance(Duration::days(1));
        pick.mark_yesterday_skipped().await.unwrap();
        let skipped = h.db.call(move |c| interactions::has(c, id, "skipped")).await.unwrap();
        assert!(skipped);
    }

    #[tokio::test]
    async fn no_pick_before_onboarding() {
        let (h, pick, _n, _s) = setup("2026-09-29T09:00:00Z").await;
        h.news.fetch_cycle(true).await.unwrap();
        h.settings.update(json!({ "onboardingDone": false })).await.unwrap();
        assert!(pick.ensure_today().await.unwrap().is_none());
    }
}
