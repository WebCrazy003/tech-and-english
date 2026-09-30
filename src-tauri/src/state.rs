use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::ai::manager::AiManager;
use crate::ai::models::Downloader;
use crate::ai::service::AiService;
use crate::clock::Clock;
use crate::db::Db;
use crate::events::EventSink;
use crate::learning::vocab::VocabService;
use crate::mode::ModeManager;
use crate::news::NewsService;
use crate::news::pick::PickService;
use crate::notify::NotifyService;
use crate::settings::SettingsStore;
use crate::voice::session::VoiceEngine;

pub struct AppState {
    pub db: Db,
    pub clock: Arc<dyn Clock>,
    pub settings: Arc<SettingsStore>,
    pub events: Arc<dyn EventSink>,
    pub mode: Arc<ModeManager>,
    pub news: Arc<NewsService>,
    pub pick: Arc<PickService>,
    pub notify: Arc<NotifyService>,
    pub ai_manager: Arc<AiManager>,
    /// whisper-server (P5).
    pub stt_manager: Arc<AiManager>,
    pub ai_service: Arc<AiService>,
    pub vocab: Arc<VocabService>,
    /// Voice tutor (P5).
    pub voice: Arc<VoiceEngine>,
    pub downloader: Arc<Downloader>,
    /// Set by the tray Quit item; any other exit request is prevented.
    pub quitting: AtomicBool,
    /// Route the main window should open when it (re)loads.
    pub pending_route: Mutex<Option<String>>,
}
