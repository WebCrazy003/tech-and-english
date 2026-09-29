use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::clock::Clock;
use crate::db::Db;
use crate::events::EventSink;
use crate::mode::ModeManager;
use crate::news::NewsService;
use crate::news::pick::PickService;
use crate::notify::NotifyService;
use crate::settings::SettingsStore;

pub struct AppState {
    pub db: Db,
    pub clock: Arc<dyn Clock>,
    pub settings: Arc<SettingsStore>,
    pub events: Arc<dyn EventSink>,
    pub mode: Arc<ModeManager>,
    pub news: Arc<NewsService>,
    pub pick: Arc<PickService>,
    pub notify: Arc<NotifyService>,
    /// Set by the tray Quit item; any other exit request is prevented.
    pub quitting: AtomicBool,
    /// Route the main window should open when it (re)loads.
    pub pending_route: Mutex<Option<String>>,
}
