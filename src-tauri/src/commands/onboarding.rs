use serde::Deserialize;
use serde_json::json;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use super::CmdResult;
use crate::clock::fmt_ts;
use crate::events;
use crate::seed::{self, OnboardingDefaults};
use crate::state::AppState;

#[tauri::command]
pub async fn get_onboarding_defaults() -> CmdResult<OnboardingDefaults> {
    Ok(seed::defaults())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingInput {
    pub topic_names: Vec<String>,
    pub feed_urls: Vec<String>,
    pub pick_time: String,
    pub notify_daily_pick: bool,
    pub notify_high_interest: bool,
    pub launch_at_login: bool,
}

#[tauri::command]
pub async fn complete_onboarding(app: AppHandle, state: State<'_, AppState>, input: OnboardingInput) -> CmdResult<()> {
    if input.topic_names.is_empty() {
        return Err(crate::error::AppError::Invalid("Choose at least one topic".into()));
    }
    if input.feed_urls.is_empty() {
        return Err(crate::error::AppError::Invalid(
            "Choose at least one news source".into(),
        ));
    }
    let now = fmt_ts(state.clock.now());
    let (topics, feeds) = (input.topic_names.clone(), input.feed_urls.clone());
    state.db.call(move |c| seed::apply(c, &topics, &feeds, &now)).await?;
    let s = state
        .settings
        .update(json!({
            "onboardingDone": true,
            "pickTime": input.pick_time,
            "notifyDailyPick": input.notify_daily_pick,
            "notifyHighInterest": input.notify_high_interest,
        }))
        .await?;
    events::emit(state.events.as_ref(), events::SETTINGS_CHANGED, &s);
    let al = app.autolaunch();
    let r = if input.launch_at_login {
        al.enable()
    } else {
        al.disable()
    };
    if let Err(e) = r {
        tracing::warn!(error = %e, "launch at login setting failed");
    }
    state.news.reload_topics().await?;
    let news = state.news.clone();
    let pick = state.pick.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = news.fetch_cycle(true).await {
            tracing::warn!(error = %e, "first fetch failed");
        }
        if let Err(e) = pick.ensure_today().await {
            tracing::warn!(error = %e, "first pick failed");
        }
    });
    Ok(())
}
