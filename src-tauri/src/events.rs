use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

pub const NEWS_UPDATED: &str = "news://updated";
pub const PICK_CHANGED: &str = "pick://changed";
pub const MODE_CHANGED: &str = "mode://changed";
pub const SETTINGS_CHANGED: &str = "settings://changed";
pub const NAVIGATE: &str = "navigate";
pub const ARTICLE_BODY: &str = "article://body";
pub const NEEDS_BODY: &str = "article://needs-body";
/// Low-priority list of new articles to fetch; unreadable ones get hidden.
pub const CHECK_BODIES: &str = "article://check-bodies";
pub const AI_STATUS: &str = "ai://status";
pub const AI_DOWNLOAD: &str = "ai://download";
pub const VOCAB_CHANGED: &str = "vocab://changed";
pub const VOICE_ACTIVE: &str = "voice://active";
pub const VOICE_LEVEL: &str = "voice://level";
pub const VOICE_AUTOSTOP: &str = "voice://autostop";

pub trait EventSink: Send + Sync {
    fn emit_value(&self, event: &str, payload: Value);
}

pub fn emit<T: Serialize>(sink: &dyn EventSink, event: &str, payload: &T) {
    match serde_json::to_value(payload) {
        Ok(v) => sink.emit_value(event, v),
        Err(e) => tracing::warn!(event, error = %e, "could not serialize event payload"),
    }
}

pub struct TauriEventSink(pub AppHandle);

impl EventSink for TauriEventSink {
    fn emit_value(&self, event: &str, payload: Value) {
        if let Err(e) = self.0.emit(event, payload) {
            tracing::warn!(event, error = %e, "emit failed");
        }
    }
}

type Hook = Box<dyn Fn(&str, &Value) + Send + Sync>;

/// Test sink that records every event. An optional hook can act like the frontend.
#[derive(Default)]
pub struct RecordingEventSink {
    pub events: Mutex<Vec<(String, Value)>>,
    pub hook: Mutex<Option<Hook>>,
}

impl RecordingEventSink {
    pub fn names(&self) -> Vec<String> {
        self.events.lock().unwrap().iter().map(|(n, _)| n.clone()).collect()
    }
}

impl EventSink for RecordingEventSink {
    fn emit_value(&self, event: &str, payload: Value) {
        if let Some(hook) = self.hook.lock().unwrap().as_ref() {
            hook(event, &payload);
        }
        self.events.lock().unwrap().push((event.to_string(), payload));
    }
}
