use std::sync::Arc;

use serde::Serialize;
use serde_json::json;
use tauri::State;
use tauri::ipc::Channel;

use super::CmdResult;
use crate::ai::manager::{AiStatus, resolve_binary};
use crate::ai::models::{self, ModelInfo};
use crate::ai::prompts::QuickAction;
use crate::ai::service::{DerivKind, Sink, StreamEvent};
use crate::db::repo::ai::{self as ai_repo, ChatMessage};
use crate::error::AppError;
use crate::events;
use crate::state::AppState;

fn channel_sink(ch: Channel<StreamEvent>) -> Sink {
    Arc::new(move |e| {
        let _ = ch.send(e);
    })
}

/// Sent when the user pressed Stop, so the UI knows the stream is over.
fn cancelled_event() -> StreamEvent {
    StreamEvent::Error {
        code: "cancelled".into(),
        message: "Stopped".into(),
    }
}

fn error_event(e: &AppError) -> StreamEvent {
    StreamEvent::Error {
        code: e.code().to_string(),
        message: e.to_string(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOverview {
    pub status: AiStatus,
    /// Path of the llama-server binary found, if any.
    pub engine_path: Option<String>,
    pub models: Vec<ModelInfo>,
    pub available_memory_gb: f64,
}

#[tauri::command]
pub async fn ai_overview(state: State<'_, AppState>) -> CmdResult<AiOverview> {
    let s = state.settings.get();
    let dir = state.ai_manager.data_dir().to_path_buf();
    Ok(AiOverview {
        status: state.ai_manager.status(),
        engine_path: resolve_binary(s.ai.llama_server_path.as_deref(), &dir).map(|p| p.to_string_lossy().into_owned()),
        models: models::list(
            &dir,
            s.ai.active_model.as_deref(),
            s.voice.stt_model.as_deref(),
            &state.downloader,
        ),
        available_memory_gb: crate::ai::manager::available_memory() as f64 / 1e9,
    })
}

#[tauri::command]
pub async fn download_model(state: State<'_, AppState>, model_id: String) -> CmdResult<()> {
    let entry = models::catalog()
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| AppError::NotFound(format!("model {model_id}")))?;
    let dir = models::models_dir(state.ai_manager.data_dir());
    let (downloader, events) = (state.downloader.clone(), state.events.clone());
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
        let ev = events.clone();
        let id = entry.id.clone();
        let result = downloader
            .download(&client, &entry, &dir, move |p| {
                events::emit(
                    ev.as_ref(),
                    events::AI_DOWNLOAD,
                    &json!({ "modelId": id, "bytes": p.bytes, "total": p.total }),
                );
            })
            .await;
        let payload = match &result {
            Ok(_) => json!({ "modelId": entry.id, "done": true }),
            Err(e) => json!({ "modelId": entry.id, "error": e.to_string() }),
        };
        events::emit(events.as_ref(), events::AI_DOWNLOAD, &payload);
        if let Err(e) = result {
            tracing::warn!(model = %entry.id, error = %e, "model download failed");
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn cancel_download(state: State<'_, AppState>, model_id: String) -> CmdResult<()> {
    state.downloader.cancel(&model_id);
    Ok(())
}

#[tauri::command]
pub async fn delete_model(state: State<'_, AppState>, model_id: String) -> CmdResult<()> {
    let entry = models::catalog()
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| AppError::NotFound(format!("model {model_id}")))?;
    if state.ai_manager.status().model_id.as_deref() == Some(&entry.id) {
        state.ai_manager.shutdown().await;
    }
    let dir = models::models_dir(state.ai_manager.data_dir());
    for f in [dir.join(&entry.file), dir.join(format!("{}.part", entry.file))] {
        let _ = std::fs::remove_file(f);
    }
    Ok(())
}

#[tauri::command]
pub async fn set_active_model(state: State<'_, AppState>, model_id: String) -> CmdResult<()> {
    state
        .settings
        .update(json!({ "ai": { "activeModel": model_id } }))
        .await?;
    // The next request starts the new model.
    state.ai_manager.shutdown().await;
    Ok(())
}

/// Start the model now (e.g. "Start anyway" after a low-memory warning).
#[tauri::command]
pub async fn start_ai(state: State<'_, AppState>, force: bool) -> CmdResult<AiStatus> {
    state.ai_manager.ensure_ready(force).await?;
    Ok(state.ai_manager.status())
}

#[tauri::command]
pub async fn unload_ai(state: State<'_, AppState>) -> CmdResult<()> {
    state.ai_manager.shutdown().await;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStarted {
    pub job_id: u64,
}

/// A cached B1 summary / Easy English text, without generating one (the Listen button, P4).
#[tauri::command]
pub async fn get_cached_derivative(
    state: State<'_, AppState>,
    article_id: i64,
    kind: DerivKind,
) -> CmdResult<Option<String>> {
    let k = kind.as_str();
    Ok(state
        .db
        .call(move |c| ai_repo::get_derivative(c, article_id, k))
        .await?
        .map(|d| d.text))
}

#[tauri::command]
pub async fn get_derivative(
    state: State<'_, AppState>,
    article_id: i64,
    kind: DerivKind,
    regenerate: Option<bool>,
    force: Option<bool>,
    channel: Channel<StreamEvent>,
) -> CmdResult<JobStarted> {
    let svc = state.ai_service.clone();
    let (job_id, token) = svc.new_job();
    let sink = channel_sink(channel);
    tauri::async_runtime::spawn(async move {
        match svc
            .derivative(
                article_id,
                kind,
                regenerate.unwrap_or(false),
                force.unwrap_or(false),
                &token,
                &sink,
            )
            .await
        {
            Err(e) => sink(error_event(&e)),
            Ok(_) if token.is_cancelled() => sink(cancelled_event()),
            Ok(_) => {}
        }
        svc.finish_job(job_id);
    });
    Ok(JobStarted { job_id })
}

#[tauri::command]
pub async fn list_article_chat(state: State<'_, AppState>, article_id: i64) -> CmdResult<Vec<ChatMessage>> {
    state.db.call(move |c| ai_repo::list_chat(c, article_id)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStarted {
    pub job_id: u64,
    /// `None` for a retry (the question is already saved).
    pub user_message: Option<ChatMessage>,
}

#[tauri::command]
pub async fn send_article_chat(
    state: State<'_, AppState>,
    article_id: i64,
    text: Option<String>,
    action: Option<QuickAction>,
    retry: Option<bool>,
    force: Option<bool>,
    channel: Channel<StreamEvent>,
) -> CmdResult<ChatStarted> {
    let svc = state.ai_service.clone();
    // Retry (after "Start anyway" or an error) answers the last question again.
    let user_message = if retry.unwrap_or(false) {
        None
    } else {
        let question = match (action, text) {
            (Some(a), _) => a.message().to_string(),
            (None, Some(t)) => t,
            (None, None) => return Err(AppError::Invalid("Type a question first".into())),
        };
        Some(svc.add_user_message(article_id, question).await?)
    };
    let (job_id, token) = svc.new_job();
    let sink = channel_sink(channel);
    tauri::async_runtime::spawn(async move {
        match svc
            .answer_chat(article_id, action, force.unwrap_or(false), &token, &sink)
            .await
        {
            Err(e) => sink(error_event(&e)),
            Ok(()) if token.is_cancelled() => sink(cancelled_event()),
            Ok(()) => {}
        }
        svc.finish_job(job_id);
    });
    Ok(ChatStarted { job_id, user_message })
}

#[tauri::command]
pub async fn clear_article_chat(state: State<'_, AppState>, article_id: i64) -> CmdResult<()> {
    state.db.call(move |c| ai_repo::clear_chat(c, article_id)).await
}

#[tauri::command]
pub async fn cancel_job(state: State<'_, AppState>, job_id: u64) -> CmdResult<()> {
    state.ai_service.cancel(job_id);
    Ok(())
}
