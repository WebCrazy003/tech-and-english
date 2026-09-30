//! Voice tutor (P5): sessions, push-to-talk, review and history.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

use super::CmdResult;
use crate::ai::models;
use crate::db::repo::voice::{self as repo, Conversation, Turn};
use crate::error::AppError;
use crate::sidecar::resolve_binary;
use crate::state::AppState;
use crate::voice::capture::{self, MicPermission};
use crate::voice::review::{SessionReview, Suggestion};
use crate::voice::session::{ActiveSession, EndReason, LatencyReport, SessionSettings, Sink, VoiceEvent};
use crate::voice::tutor::CorrectionPolicy;

fn channel_sink(ch: Channel<VoiceEvent>) -> Sink {
    Arc::new(move |e| {
        let _ = ch.send(e);
    })
}

/// Errors go to the channel too, so the UI's turn state always finishes.
fn report(sink: &Sink, e: &AppError) {
    sink(VoiceEvent::Error {
        code: e.code().into(),
        message: e.to_string(),
    });
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSetup {
    pub settings: SessionSettings,
    pub active: Option<ActiveSession>,
    /// The speech model is downloaded.
    pub stt_model: bool,
    /// whisper-server was found.
    pub stt_engine: Option<String>,
    /// granted | denied | notDetermined
    pub mic: &'static str,
}

#[tauri::command]
pub async fn voice_setup(state: State<'_, AppState>) -> CmdResult<VoiceSetup> {
    let s = state.settings.get();
    let dir = state.stt_manager.data_dir().to_path_buf();
    let mic = tokio::task::spawn_blocking(capture::mic_permission).await?;
    Ok(VoiceSetup {
        settings: state.voice.default_settings(),
        active: state.voice.active(),
        stt_model: models::resolve(&dir, "stt", s.voice.stt_model.as_deref()).is_some(),
        stt_engine: resolve_binary("whisper-server", s.ai.whisper_server_path.as_deref(), &dir)
            .map(|p| p.to_string_lossy().into_owned()),
        mic: match mic {
            MicPermission::Granted => "granted",
            MicPermission::Denied => "denied",
            MicPermission::NotDetermined => "notDetermined",
        },
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartArgs {
    pub article_id: Option<i64>,
    pub settings: SessionSettings,
    #[serde(default)]
    pub force: bool,
}

/// Loads both engines, prepares the summary and streams the opening turn.
#[tauri::command]
pub async fn start_voice_session(
    state: State<'_, AppState>,
    args: StartArgs,
    channel: Channel<VoiceEvent>,
) -> CmdResult<i64> {
    let sink = channel_sink(channel);
    let r = state
        .voice
        .start(args.article_id, args.settings, args.force, sink.clone())
        .await;
    if let Err(e) = &r {
        report(&sink, e);
    }
    r
}

#[tauri::command]
pub async fn start_recording(state: State<'_, AppState>) -> CmdResult<()> {
    let voice = state.voice.clone();
    tokio::task::spawn_blocking(move || voice.start_recording()).await?
}

#[tauri::command]
pub async fn stop_recording(state: State<'_, AppState>, channel: Channel<VoiceEvent>) -> CmdResult<()> {
    let sink = channel_sink(channel);
    let r = state.voice.stop_recording(sink.clone()).await;
    if let Err(e) = &r {
        report(&sink, e);
    }
    r
}

#[tauri::command]
pub async fn send_text_turn(state: State<'_, AppState>, text: String, channel: Channel<VoiceEvent>) -> CmdResult<()> {
    let sink = channel_sink(channel);
    let r = state.voice.send_text(text, sink.clone()).await;
    if let Err(e) = &r {
        report(&sink, e);
    }
    r
}

#[tauri::command]
pub async fn start_drill(state: State<'_, AppState>, word: String, channel: Channel<VoiceEvent>) -> CmdResult<()> {
    let sink = channel_sink(channel);
    let r = state.voice.start_drill(word, sink.clone()).await;
    if let Err(e) = &r {
        report(&sink, e);
    }
    r
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPatch {
    pub rate: Option<f64>,
    pub level: Option<u8>,
    pub correction: Option<CorrectionPolicy>,
}

#[tauri::command]
pub async fn update_session_settings(state: State<'_, AppState>, patch: SessionPatch) -> CmdResult<SessionSettings> {
    state
        .voice
        .update_settings(patch.rate, patch.level, patch.correction)
        .await
}

#[tauri::command]
pub async fn get_active_session(state: State<'_, AppState>) -> CmdResult<Option<ActiveSession>> {
    Ok(state.voice.active())
}

/// `user` → the review is built now; `hibernate` / `quit` → it stays pending in History.
#[tauri::command]
pub async fn end_voice_session(state: State<'_, AppState>, reason: EndReason) -> CmdResult<Option<SessionReview>> {
    state.voice.end(reason, reason == EndReason::User).await
}

#[tauri::command]
pub async fn get_session_review(state: State<'_, AppState>, conversation_id: i64) -> CmdResult<SessionReview> {
    state.voice.review(conversation_id).await
}

/// The chosen suggestions become Word Book items; an empty list = Skip.
#[tauri::command]
pub async fn apply_session_review(
    state: State<'_, AppState>,
    conversation_id: i64,
    selected: Vec<Suggestion>,
) -> CmdResult<usize> {
    state.voice.apply_review(conversation_id, selected).await
}

#[tauri::command]
pub async fn list_conversations(state: State<'_, AppState>) -> CmdResult<Vec<Conversation>> {
    state.db.call(|c| repo::list(c, 200)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDetail {
    pub conversation: Conversation,
    pub turns: Vec<Turn>,
}

#[tauri::command]
pub async fn get_conversation(state: State<'_, AppState>, id: i64) -> CmdResult<ConversationDetail> {
    state
        .db
        .call(move |c| {
            Ok(ConversationDetail {
                conversation: repo::get(c, id)?,
                turns: repo::turns(c, id)?,
            })
        })
        .await
}

/// Settings › Data: delete every conversation (Word Book items stay).
#[tauri::command]
pub async fn delete_conversations(state: State<'_, AppState>) -> CmdResult<usize> {
    if state.voice.is_active() {
        return Err(AppError::SessionActive);
    }
    state.db.call(|c| repo::delete_all(c)).await
}

#[tauri::command]
pub async fn report_latency(state: State<'_, AppState>, turn: u64, tts_start_ms: u64) -> CmdResult<()> {
    state.voice.report_tts_start(turn, tts_start_ms);
    Ok(())
}

#[tauri::command]
pub async fn voice_latency(state: State<'_, AppState>) -> CmdResult<LatencyReport> {
    Ok(state.voice.latency())
}
