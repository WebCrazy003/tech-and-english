use serde_json::Value;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use super::CmdResult;
use crate::error::AppError;
use crate::events;
use crate::mode::Mode;
use crate::settings::Settings;
use crate::shell;
use crate::state::AppState;
use crate::voice::session::EndReason;

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    Ok(state.settings.get())
}

#[tauri::command]
pub async fn update_settings(app: AppHandle, state: State<'_, AppState>, patch: Value) -> CmdResult<Settings> {
    let before = state.settings.get();
    let s = state.settings.update(patch).await?;
    if s.widget.style != before.widget.style || s.widget.always_on_top != before.widget.always_on_top {
        shell::apply_widget_style(&app, &s.widget);
    }
    if s.ranking_weights != before.ranking_weights {
        state.news.rescore().await?;
    }
    events::emit(state.events.as_ref(), events::SETTINGS_CHANGED, &s);
    // A changed pick time may make today's pick due now.
    if s.pick_time != before.pick_time || s.onboarding_done != before.onboarding_done {
        state.pick.ensure_today().await?;
    }
    Ok(s)
}

#[tauri::command]
pub async fn get_mode(state: State<'_, AppState>) -> CmdResult<Mode> {
    Ok(state.mode.get())
}

/// Hibernate during a voice session needs `force` (the UI asked first); the session then ends
/// and its review stays pending.
#[tauri::command]
pub async fn set_mode(state: State<'_, AppState>, mode: Mode, force: Option<bool>) -> CmdResult<Mode> {
    if mode == Mode::Hibernate && state.voice.is_active() {
        if !force.unwrap_or(false) {
            return Err(AppError::SessionActive);
        }
        state.voice.end(EndReason::Hibernate, false).await?;
    }
    state.mode.set(mode).await
}

#[tauri::command]
pub async fn get_autostart(app: AppHandle) -> CmdResult<bool> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> CmdResult<bool> {
    let al = app.autolaunch();
    let r = if enabled { al.enable() } else { al.disable() };
    r.map_err(|e| AppError::Internal(format!("launch at login: {e}")))?;
    let on = al.is_enabled().unwrap_or(false);
    if let Some(items) = tauri::Manager::try_state::<shell::TrayItems>(&app) {
        let _ = items.autostart.set_checked(on);
    }
    Ok(on)
}
