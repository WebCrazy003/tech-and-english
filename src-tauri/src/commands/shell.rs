use serde::Deserialize;
use serde_json::json;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, State};

use super::CmdResult;
use crate::events;
use crate::settings::{WidgetSettings, WidgetStyle};
use crate::shell;
use crate::state::AppState;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetStyleArgs {
    pub style: Option<WidgetStyle>,
    pub always_on_top: Option<bool>,
}

#[tauri::command]
pub async fn set_widget_style(
    app: AppHandle,
    state: State<'_, AppState>,
    args: WidgetStyleArgs,
) -> CmdResult<WidgetSettings> {
    let mut patch = serde_json::Map::new();
    if let Some(s) = args.style {
        patch.insert("style".into(), json!(s));
    }
    if let Some(a) = args.always_on_top {
        patch.insert("alwaysOnTop".into(), json!(a));
    }
    let s = state.settings.update(json!({ "widget": patch })).await?;
    shell::apply_widget_style(&app, &s.widget);
    events::emit(state.events.as_ref(), events::SETTINGS_CHANGED, &s);
    Ok(s.widget)
}

#[tauri::command]
pub async fn show_main(app: AppHandle, route: Option<String>) -> CmdResult<()> {
    shell::show_main(&app, route.as_deref().unwrap_or("/today"));
    Ok(())
}

/// The main window calls this on load to open the route requested before it was ready.
#[tauri::command]
pub async fn take_pending_route(state: State<'_, AppState>) -> CmdResult<Option<String>> {
    Ok(state.pending_route.lock().unwrap().take())
}

#[tauri::command]
pub async fn quit_app(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.quitting.store(true, Ordering::SeqCst);
    app.exit(0);
    Ok(())
}
