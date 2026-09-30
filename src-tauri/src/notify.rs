//! Notification rules (SPEC §7.9). The policy is a pure function; sending goes through [`Notifier`].

use std::sync::Arc;

use chrono::{DateTime, Duration, FixedOffset, NaiveTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::clock::{Clock, fmt_ts, parse_hhmm, parse_ts};
use crate::db::Db;
use crate::db::repo::articles::{self, ArticleListItem};
use crate::db::repo::{app_state, notifications};
use crate::error::{AppError, AppResult};
use crate::settings::{Settings, SettingsStore};

pub trait Notifier: Send + Sync {
    fn send(&self, title: &str, body: &str) -> AppResult<()>;
}

pub struct TauriNotifier(pub AppHandle);

impl Notifier for TauriNotifier {
    fn send(&self, title: &str, body: &str) -> AppResult<()> {
        self.0
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| AppError::Internal(format!("notification: {e}")))
    }
}

/// Records notifications instead of showing them (tests).
#[derive(Default)]
pub struct RecordingNotifier {
    pub sent: std::sync::Mutex<Vec<(String, String)>>,
}

impl Notifier for RecordingNotifier {
    fn send(&self, title: &str, body: &str) -> AppResult<()> {
        self.sent.lock().unwrap().push((title.to_string(), body.to_string()));
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Send,
    Skip(&'static str),
}

pub struct HiCtx<'a> {
    pub now: DateTime<Utc>,
    pub now_local: DateTime<FixedOffset>,
    pub settings: &'a Settings,
    pub score: f64,
    /// (topic.notify, topic.notify_threshold) for every matching enabled topic.
    pub topics: Vec<(bool, Option<u8>)>,
    pub sent_today: u32,
    /// Last notification of any kind.
    pub last_sent_at: Option<DateTime<Utc>>,
    pub already_notified: bool,
    pub from_first_fetch: bool,
}

pub fn in_quiet_hours(t: NaiveTime, start: NaiveTime, end: NaiveTime) -> bool {
    if start <= end {
        t >= start && t < end
    } else {
        t >= start || t < end
    }
}

pub fn high_interest_decision(c: &HiCtx) -> Decision {
    let s = c.settings;
    if !s.notify_high_interest {
        return Decision::Skip("disabled");
    }
    if c.already_notified {
        return Decision::Skip("already notified");
    }
    if c.from_first_fetch {
        return Decision::Skip("first fetch");
    }
    let thresholds: Vec<u8> = c
        .topics
        .iter()
        .filter(|(notify, _)| *notify)
        .map(|(_, t)| t.unwrap_or(s.notify_threshold))
        .collect();
    let Some(threshold) = thresholds.into_iter().min() else {
        return Decision::Skip("no topic with notifications on");
    };
    if c.score < threshold as f64 {
        return Decision::Skip("below threshold");
    }
    if c.sent_today >= s.notify_max_per_day as u32 {
        return Decision::Skip("daily cap");
    }
    if c.last_sent_at
        .is_some_and(|l| c.now - l < Duration::minutes(s.notify_min_gap_min as i64))
    {
        return Decision::Skip("too soon");
    }
    if let Some((a, b)) = &s.quiet_hours
        && let (Some(a), Some(b)) = (parse_hhmm(a), parse_hhmm(b))
        && in_quiet_hours(c.now_local.time(), a, b)
    {
        return Decision::Skip("quiet hours");
    }
    Decision::Send
}

pub struct NotifyService {
    db: Db,
    clock: Arc<dyn Clock>,
    settings: Arc<SettingsStore>,
    notifier: Arc<dyn Notifier>,
}

fn local_midnight_utc(now_local: DateTime<FixedOffset>) -> String {
    let midnight = now_local.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let dt = midnight
        .and_local_timezone(*now_local.offset())
        .single()
        .unwrap_or(now_local);
    fmt_ts(dt.with_timezone(&Utc))
}

fn last_sent_any(conn: &Connection) -> AppResult<Option<DateTime<Utc>>> {
    let s: Option<String> = conn
        .query_row("SELECT max(sent_at) FROM notifications_log", [], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(s.and_then(|s| parse_ts(&s)))
}

impl NotifyService {
    pub fn new(db: Db, clock: Arc<dyn Clock>, settings: Arc<SettingsStore>, notifier: Arc<dyn Notifier>) -> Self {
        Self {
            db,
            clock,
            settings,
            notifier,
        }
    }

    /// Exactly one per day; called by the pick service when the story is made.
    /// It mentions today's lesson too, if there is one (P3).
    pub async fn daily_pick(&self, item: &ArticleListItem, lesson: Option<&ArticleListItem>) -> AppResult<()> {
        if !self.settings.get().notify_daily_pick {
            return Ok(());
        }
        let meta = [item.primary_topic.as_deref(), Some(item.source_name.as_str())]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        let (title, body) = match lesson {
            Some(l) => (
                "Today's story and lesson are ready.",
                format!("\"{}\"\n{}\nLesson: \"{}\"", item.title, meta, l.title),
            ),
            None => ("Today's tech story is ready.", format!("\"{}\"\n{}", item.title, meta)),
        };
        self.notifier.send(title, &body)?;
        let (id, now) = (item.id, fmt_ts(self.clock.now()));
        self.db
            .call(move |c| notifications::insert(c, "daily_pick", Some(id), &now))
            .await
    }

    /// Consider the best recent article not yet notified. Returns the article id if sent.
    pub async fn maybe_high_interest(&self) -> AppResult<Option<i64>> {
        let settings = self.settings.get();
        if !settings.onboarding_done || !settings.notify_high_interest {
            return Ok(None);
        }
        let now = self.clock.now();
        let now_local = self.clock.now_local();
        let candidate = self
            .db
            .call(move |c| {
                let since = fmt_ts(now - Duration::hours(24));
                let row: Option<(i64, f64, String)> = c
                    .query_row(
                        "SELECT a.id, a.score, a.discovered_at FROM articles a
                         WHERE a.hidden = 0 AND a.score IS NOT NULL AND a.discovered_at >= ?1
                           AND NOT EXISTS (SELECT 1 FROM notifications_log n WHERE n.article_id = a.id)
                         ORDER BY a.score DESC LIMIT 1",
                        [&since],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((id, score, discovered)) = row else {
                    return Ok(None);
                };
                let mut st = c.prepare(
                    "SELECT t.notify, t.notify_threshold FROM article_topics at JOIN topics t ON t.id = at.topic_id
                     WHERE at.article_id = ?1 AND t.enabled = 1",
                )?;
                let topics = st
                    .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<Vec<(bool, Option<u8>)>, _>>()?;
                let first = app_state::get(c, app_state::FIRST_FETCH_COMPLETED_AT)?;
                let from_first_fetch = first.as_deref().is_none_or(|f| discovered.as_str() <= f);
                let sent_today = notifications::count_since(c, "high_interest", &local_midnight_utc(now_local))?;
                let last = last_sent_any(c)?;
                let item = articles::get_item(c, id)?;
                Ok(Some((score, topics, from_first_fetch, sent_today, last, item)))
            })
            .await?;
        let Some((score, topics, from_first_fetch, sent_today, last_sent_at, item)) = candidate else {
            return Ok(None);
        };
        let decision = high_interest_decision(&HiCtx {
            now,
            now_local,
            settings: &settings,
            score,
            topics,
            sent_today,
            last_sent_at,
            already_notified: false,
            from_first_fetch,
        });
        if decision != Decision::Send {
            return Ok(None);
        }
        let heading = match &item.primary_topic {
            Some(t) => format!("Interesting for you: {t}"),
            None => "A story you may like".to_string(),
        };
        self.notifier
            .send(&heading, &format!("\"{}\"\n{}", item.title, item.source_name))?;
        let (id, ts) = (item.id, fmt_ts(now));
        self.db
            .call(move |c| notifications::insert(c, "high_interest", Some(id), &ts))
            .await?;
        tracing::info!(article = id, score, "high-interest notification sent");
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ctx(settings: &Settings) -> HiCtx<'_> {
        let now = Utc.with_ymd_and_hms(2026, 9, 29, 12, 0, 0).unwrap();
        HiCtx {
            now,
            now_local: now.with_timezone(&FixedOffset::east_opt(0).unwrap()),
            settings,
            score: 90.0,
            topics: vec![(true, None)],
            sent_today: 0,
            last_sent_at: None,
            already_notified: false,
            from_first_fetch: false,
        }
    }

    #[test]
    fn sends_when_all_rules_pass() {
        let s = Settings::default();
        assert_eq!(high_interest_decision(&ctx(&s)), Decision::Send);
    }

    #[test]
    fn each_rule_can_block() {
        let s = Settings::default();
        let mut c = ctx(&s);
        c.score = 84.9;
        assert_eq!(high_interest_decision(&c), Decision::Skip("below threshold"));

        let mut c = ctx(&s);
        c.topics = vec![(false, None)];
        assert_eq!(
            high_interest_decision(&c),
            Decision::Skip("no topic with notifications on")
        );

        let mut c = ctx(&s);
        c.sent_today = 3;
        assert_eq!(high_interest_decision(&c), Decision::Skip("daily cap"));

        let mut c = ctx(&s);
        c.last_sent_at = Some(c.now - Duration::minutes(89));
        assert_eq!(high_interest_decision(&c), Decision::Skip("too soon"));
        c.last_sent_at = Some(c.now - Duration::minutes(90));
        assert_eq!(high_interest_decision(&c), Decision::Send);

        let mut c = ctx(&s);
        c.from_first_fetch = true;
        assert_eq!(high_interest_decision(&c), Decision::Skip("first fetch"));

        let mut c = ctx(&s);
        c.already_notified = true;
        assert_eq!(high_interest_decision(&c), Decision::Skip("already notified"));

        let off = Settings {
            notify_high_interest: false,
            ..Settings::default()
        };
        assert_eq!(high_interest_decision(&ctx(&off)), Decision::Skip("disabled"));
    }

    #[test]
    fn lowest_topic_threshold_wins() {
        let s = Settings::default();
        let mut c = ctx(&s);
        c.score = 60.0;
        c.topics = vec![(true, Some(90)), (true, Some(55)), (false, Some(10))];
        assert_eq!(high_interest_decision(&c), Decision::Send);
    }

    #[test]
    fn quiet_hours_wrap_midnight() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        assert!(in_quiet_hours(t(23, 0), t(22, 0), t(8, 0)));
        assert!(in_quiet_hours(t(7, 59), t(22, 0), t(8, 0)));
        assert!(!in_quiet_hours(t(8, 0), t(22, 0), t(8, 0)));
        assert!(in_quiet_hours(t(13, 0), t(12, 0), t(14, 0)));

        let s = Settings::default();
        let mut c = ctx(&s);
        c.now_local = Utc
            .with_ymd_and_hms(2026, 9, 29, 23, 30, 0)
            .unwrap()
            .with_timezone(&FixedOffset::east_opt(0).unwrap());
        assert_eq!(high_interest_decision(&c), Decision::Skip("quiet hours"));
    }
}
