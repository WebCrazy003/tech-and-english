//! 60-second wall-clock tick (SPEC §6.1). After the Mac wakes, the first tick sees
//! overdue feeds and fetches them — no OS sleep/wake hooks needed.

use std::sync::Arc;
use std::time::Duration;

use crate::clock::{Clock, fmt_ts};
use crate::db::Db;
use crate::db::repo::app_state;
use crate::news::NewsService;
use crate::news::pick::PickService;
use crate::news::retention;
use crate::notify::NotifyService;

pub struct Scheduler {
    pub db: Db,
    pub clock: Arc<dyn Clock>,
    pub news: Arc<NewsService>,
    pub pick: Arc<PickService>,
    pub notify: Arc<NotifyService>,
    /// P2: AI "why" text for the pick, only while the model is already loaded.
    pub ai: Option<Arc<crate::ai::service::AiService>>,
}

impl Scheduler {
    pub async fn tick(&self) {
        if let Err(e) = self.news.fetch_cycle(false).await {
            tracing::warn!(error = %e, "fetch cycle failed");
        }
        if let Err(e) = self.pick.ensure_today().await {
            tracing::warn!(error = %e, "daily pick failed");
        }
        if let Err(e) = self.notify.maybe_high_interest().await {
            tracing::warn!(error = %e, "high-interest check failed");
        }
        if let Some(ai) = &self.ai
            && let Err(e) = ai.background_why(self.clock.today_local().to_string()).await
        {
            tracing::debug!(error = %e, "AI why skipped");
        }
        if let Err(e) = self.daily_jobs().await {
            tracing::warn!(error = %e, "daily jobs failed");
        }
    }

    /// Once per local date: retention + "skipped" marking for yesterday's pick.
    async fn daily_jobs(&self) -> crate::error::AppResult<()> {
        let today = self.clock.today_local().to_string();
        let now = self.clock.now();
        let t = today.clone();
        let due = self
            .db
            .call(move |c| Ok(app_state::get(c, app_state::LAST_DAILY_JOB_DATE)?.as_deref() != Some(t.as_str())))
            .await?;
        if !due {
            return Ok(());
        }
        self.db.call(move |c| retention::run(c, now)).await?;
        self.pick.mark_yesterday_skipped().await?;
        self.db
            .call(move |c| app_state::set(c, app_state::LAST_DAILY_JOB_DATE, &today))
            .await?;
        tracing::debug!(at = fmt_ts(now), "daily jobs done");
        Ok(())
    }

    pub fn spawn(self: Arc<Self>) {
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                self.tick().await;
            }
        });
    }
}
