//! Words (P4): dictionary, "Explain simply", Word Book and quizzes.

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use super::CmdResult;
use crate::ai::service::DefineTermOut;
use crate::db::repo::vocab::{VocabFilter, VocabItem, VocabPage};
use crate::learning::dictionary::{self, DictEntry};
use crate::learning::srs::Grade;
use crate::learning::vocab::{AddResult, DueCount, GradeResult, ItemDetail, ItemPatch, NewItem, QuizResult, QuizStart};
use crate::state::AppState;

/// macOS dictionary, offline. Works in Hibernate too.
#[tauri::command]
pub async fn dictionary_lookup(term: String) -> CmdResult<Option<DictEntry>> {
    let t = std::time::Instant::now();
    let e = tokio::task::spawn_blocking(move || dictionary::lookup(&term)).await?;
    tracing::debug!(
        found = e.is_some(),
        ms = t.elapsed().as_millis() as u64,
        "dictionary lookup"
    );
    Ok(e)
}

#[tauri::command]
pub async fn define_term(
    state: State<'_, AppState>,
    term: String,
    sentence: Option<String>,
    article_id: Option<i64>,
    force: Option<bool>,
) -> CmdResult<DefineTermOut> {
    state
        .ai_service
        .define_term(&term, sentence.as_deref(), article_id, force.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn add_vocab_item(state: State<'_, AppState>, item: NewItem) -> CmdResult<AddResult> {
    state.vocab.add(item).await
}

#[tauri::command]
pub async fn update_vocab_item(state: State<'_, AppState>, id: i64, patch: ItemPatch) -> CmdResult<VocabItem> {
    state.vocab.update(id, patch).await
}

#[tauri::command]
pub async fn delete_vocab_item(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.vocab.delete(id).await
}

#[tauri::command]
pub async fn list_vocab(
    state: State<'_, AppState>,
    filter: Option<VocabFilter>,
    cursor: Option<i64>,
    limit: Option<u32>,
) -> CmdResult<VocabPage> {
    state
        .vocab
        .list(filter.unwrap_or_default(), cursor, limit.unwrap_or(100))
        .await
}

#[tauri::command]
pub async fn get_vocab_item(state: State<'_, AppState>, id: i64) -> CmdResult<ItemDetail> {
    state.vocab.detail(id).await
}

#[tauri::command]
pub async fn list_vocab_keys(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    state.vocab.keys().await
}

#[tauri::command]
pub async fn due_count(state: State<'_, AppState>) -> CmdResult<DueCount> {
    state.vocab.due_count().await
}

#[derive(Serialize)]
pub struct Exported {
    pub path: String,
}

/// Write the CSV to ~/Downloads and show it in Finder.
#[tauri::command]
pub async fn export_vocab_csv(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Exported> {
    let dir = app.path().download_dir()?;
    let path = state.vocab.export_csv(&dir).await?;
    if let Err(e) = app.opener().reveal_item_in_dir(&path) {
        tracing::warn!(error = %e, "could not reveal the CSV in Finder");
    }
    Ok(Exported {
        path: path.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn start_quiz(
    state: State<'_, AppState>,
    size: Option<u32>,
    item_ids: Option<Vec<i64>>,
) -> CmdResult<QuizStart> {
    state.vocab.start_quiz(size, item_ids).await
}

#[tauri::command]
pub async fn grade_quiz_item(
    state: State<'_, AppState>,
    session_id: i64,
    item_id: i64,
    grade: Grade,
) -> CmdResult<GradeResult> {
    state.vocab.grade(session_id, item_id, grade).await
}

#[tauri::command]
pub async fn finish_quiz(state: State<'_, AppState>, session_id: i64) -> CmdResult<QuizResult> {
    state.vocab.finish(session_id).await
}

#[tauri::command]
pub async fn list_quiz_history(state: State<'_, AppState>, limit: Option<u32>) -> CmdResult<Vec<QuizResult>> {
    state.vocab.history(limit.unwrap_or(5)).await
}
