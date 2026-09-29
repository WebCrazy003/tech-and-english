use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

pub const NEWS_UPDATED: &str = "news://updated";
pub const PICK_CHANGED: &str = "pick://changed";
pub const MODE_CHANGED: &str = "mode://changed";
pub const SETTINGS_CHANGED: &str = "settings://changed";
pub const NAVIGATE: &str = "navigate";

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

/// Test sink that records every event.
#[derive(Default)]
pub struct RecordingEventSink {
    pub events: Mutex<Vec<(String, Value)>>,
}

impl RecordingEventSink {
    pub fn names(&self) -> Vec<String> {
        self.events.lock().unwrap().iter().map(|(n, _)| n.clone()).collect()
    }
}

impl EventSink for RecordingEventSink {
    fn emit_value(&self, event: &str, payload: Value) {
        self.events.lock().unwrap().push((event.to_string(), payload));
    }
}
