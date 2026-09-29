//! macOS shell: tray menu, widget window placement/style, main window routing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::json;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry,
};
use tauri_plugin_autostart::ManagerExt as _;

use crate::error::AppResult;
use crate::events;
use crate::mode::Mode;
use crate::news::pick::DailyPick;
use crate::settings::{WidgetSettings, WidgetStyle};
use crate::state::AppState;

pub const WIDGET: &str = "widget";
pub const MAIN: &str = "main";
const CARD_SIZE: (f64, f64) = (340.0, 260.0);
const PILL_SIZE: (f64, f64) = (180.0, 36.0);
const EDGE_INSET: f64 = 12.0;

/// Menu items whose state changes at runtime.
pub struct TrayItems {
    pub pick: MenuItem<Wry>,
    pub widget: MenuItem<Wry>,
    pub standard: CheckMenuItem<Wry>,
    pub hibernate: CheckMenuItem<Wry>,
    pub autostart: CheckMenuItem<Wry>,
}

pub fn build_tray(app: &AppHandle, mode: Mode) -> AppResult<()> {
    let pick = MenuItem::with_id(app, "today_pick", "Today's pick: not ready yet", false, None::<&str>)?;
    let widget = MenuItem::with_id(app, "toggle_widget", "Hide widget", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open_app", "Open Tech English", true, None::<&str>)?;
    let standard = CheckMenuItem::with_id(
        app,
        "mode_standard",
        "Standard",
        true,
        mode == Mode::Standard,
        None::<&str>,
    )?;
    let hibernate = CheckMenuItem::with_id(
        app,
        "mode_hibernate",
        "Hibernate",
        true,
        mode == Mode::Hibernate,
        None::<&str>,
    )?;
    let mode_menu = Submenu::with_id_and_items(app, "mode", "Mode", true, &[&standard, &hibernate])?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh news now", true, None::<&str>)?;
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(app, "autostart", "Launch at login", true, autostart_on, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Tech English", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &widget,
            &open,
            &pick,
            &PredefinedMenuItem::separator(app)?,
            &mode_menu,
            &refresh,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    TrayIconBuilder::with_id("main")
        .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .icon_as_template(true)
        .tooltip("Tech English")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app)?;
    app.manage(TrayItems {
        pick,
        widget,
        standard,
        hibernate,
        autostart,
    });
    Ok(())
}

fn on_menu_event(app: &AppHandle, ev: MenuEvent) {
    let state = app.state::<AppState>();
    match ev.id().as_ref() {
        "toggle_widget" => toggle_widget(app),
        "open_app" | "today_pick" => show_main(app, "/today"),
        "mode_standard" | "mode_hibernate" => {
            let mode = if ev.id().as_ref() == "mode_standard" {
                Mode::Standard
            } else {
                Mode::Hibernate
            };
            let m = state.mode.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = m.set(mode).await {
                    tracing::warn!(error = %e, "set mode from tray failed");
                }
            });
            sync_mode_items(app, mode);
        }
        "refresh" => {
            let news = state.news.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = news.fetch_cycle(true).await {
                    tracing::warn!(error = %e, "manual refresh failed");
                }
            });
        }
        "autostart" => {
            let al = app.autolaunch();
            let on = al.is_enabled().unwrap_or(false);
            let r = if on { al.disable() } else { al.enable() };
            if let Err(e) = r {
                tracing::warn!(error = %e, "autostart toggle failed");
            }
            if let Some(items) = app.try_state::<TrayItems>() {
                let _ = items.autostart.set_checked(al.is_enabled().unwrap_or(false));
            }
        }
        "quit" => {
            state.quitting.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        _ => {}
    }
}

pub fn sync_mode_items(app: &AppHandle, mode: Mode) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.standard.set_checked(mode == Mode::Standard);
        let _ = items.hibernate.set_checked(mode == Mode::Hibernate);
    }
}

pub fn sync_pick_item(app: &AppHandle, pick: Option<&DailyPick>) {
    if let Some(items) = app.try_state::<TrayItems>() {
        match pick {
            Some(p) => {
                let mut title: String = p.article.title.chars().take(48).collect();
                if p.article.title.chars().count() > 48 {
                    title.push('…');
                }
                let _ = items.pick.set_text(format!("Today's pick: {title}"));
                let _ = items.pick.set_enabled(true);
            }
            None => {
                let _ = items.pick.set_text("Today's pick: not ready yet");
                let _ = items.pick.set_enabled(false);
            }
        }
    }
}

fn sync_widget_item(app: &AppHandle, visible: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items
            .widget
            .set_text(if visible { "Hide widget" } else { "Show widget" });
    }
}

fn toggle_widget(app: &AppHandle) {
    let Some(w) = app.get_webview_window(WIDGET) else {
        return;
    };
    let visible = w.is_visible().unwrap_or(false);
    let style = if visible {
        WidgetStyle::Hidden
    } else {
        WidgetStyle::Card
    };
    let state = app.state::<AppState>();
    let settings = state.settings.clone();
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        match settings.update(json!({ "widget": { "style": style } })).await {
            Ok(s) => {
                apply_widget_style(&app2, &s.widget);
                if let Some(state) = app2.try_state::<AppState>() {
                    events::emit(state.events.as_ref(), events::SETTINGS_CHANGED, &s);
                }
            }
            Err(e) => tracing::warn!(error = %e, "toggle widget failed"),
        }
    });
}

/// Resize/show/hide the widget, keeping its right edge in place.
pub fn apply_widget_style(app: &AppHandle, ws: &WidgetSettings) {
    let Some(w) = app.get_webview_window(WIDGET) else {
        return;
    };
    let _ = w.set_always_on_top(ws.always_on_top);
    let target = match ws.style {
        WidgetStyle::Hidden => {
            let _ = w.hide();
            sync_widget_item(app, false);
            return;
        }
        WidgetStyle::Card => CARD_SIZE,
        WidgetStyle::Pill => PILL_SIZE,
    };
    if let (Ok(pos), Ok(size), Ok(scale)) = (w.outer_position(), w.outer_size(), w.scale_factor()) {
        let new_w = (target.0 * scale) as i32;
        let right = pos.x + size.width as i32;
        let _ = w.set_size(LogicalSize::new(target.0, target.1));
        if size.width as i32 != new_w && w.is_visible().unwrap_or(false) {
            let _ = w.set_position(PhysicalPosition::new(right - new_w, pos.y));
        }
    } else {
        let _ = w.set_size(LogicalSize::new(target.0, target.1));
    }
    let _ = w.show();
    sync_widget_item(app, true);
}

/// Put the widget at its saved position if that is still on a screen, else top-right.
pub fn place_widget(w: &WebviewWindow, ws: &WidgetSettings) {
    let monitors = w.available_monitors().unwrap_or_default();
    if let Some((x, y)) = ws.position {
        let on_screen = monitors.iter().any(|m| {
            let (p, s) = (m.position(), m.size());
            x + 20 >= p.x && y + 20 >= p.y && x + 20 < p.x + s.width as i32 && y + 20 < p.y + s.height as i32
        });
        if on_screen {
            let _ = w.set_position(PhysicalPosition::new(x, y));
            return;
        }
    }
    let monitor = w
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.into_iter().next());
    if let Some(m) = monitor {
        let wa = m.work_area();
        let scale = m.scale_factor();
        let logical_width = if ws.style == WidgetStyle::Pill {
            PILL_SIZE.0
        } else {
            CARD_SIZE.0
        };
        let width = (logical_width * scale) as i32;
        let inset = (EDGE_INSET * scale) as i32;
        let x = wa.position.x + wa.size.width as i32 - width - inset;
        let y = wa.position.y + inset;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

static MOVE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Save the widget position 500 ms after the last move event.
pub fn on_widget_moved(app: &AppHandle, pos: PhysicalPosition<i32>) {
    let generation = MOVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if MOVE_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(state) = app.try_state::<AppState>()
            && let Err(e) = state
                .settings
                .update(json!({ "widget": { "position": [pos.x, pos.y] } }))
                .await
        {
            tracing::warn!(error = %e, "saving widget position failed");
        }
    });
}

/// The main window is created on demand and destroyed when closed, so an idle app keeps
/// only the widget's WebKit process (saves ~70 MB).
fn ensure_main(app: &AppHandle) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(MAIN) {
        return Some(w);
    }
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Tech English")
        .inner_size(1000.0, 700.0)
        .min_inner_size(760.0, 520.0)
        .visible(false)
        .build()
        .map_err(|e| tracing::warn!(error = %e, "could not create main window"))
        .ok()
}

pub fn show_main(app: &AppHandle, route: &str) {
    if let Some(state) = app.try_state::<AppState>() {
        *state.pending_route.lock().unwrap() = Some(route.to_string());
    }
    if let Some(w) = ensure_main(app) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        let _ = app.emit_to(MAIN, events::NAVIGATE, json!({ "route": route }));
    }
}
