//! About, licenses, diagnostics and the crash notice (P6). Everything stays on this Mac.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;
use tauri::State;

use super::CmdResult;
use crate::ai::models;
use crate::db::repo::{app_state, feeds};
use crate::diagnostics::{self, BUILD_DATE, COMMIT, CRASH_SEEN_KEY, VERSION};
use crate::error::AppError;
use crate::sidecar::resolve_binary;
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    pub version: &'static str,
    pub commit: &'static str,
    pub build_date: &'static str,
    pub system: String,
    /// Downloaded models with their licenses.
    pub models: Vec<ModelLicense>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLicense {
    pub name: String,
    pub license: String,
    pub license_url: String,
}

#[tauri::command]
pub async fn about_info(state: State<'_, AppState>) -> CmdResult<AboutInfo> {
    let dir = models::models_dir(state.ai_manager.data_dir());
    let models = models::catalog()
        .into_iter()
        .filter(|m| dir.join(&m.file).is_file())
        .map(|m| ModelLicense {
            name: m.display_name,
            license: m.license,
            license_url: m.license_url,
        })
        .collect();
    Ok(AboutInfo {
        version: VERSION,
        commit: COMMIT,
        build_date: BUILD_DATE,
        system: tokio::task::spawn_blocking(diagnostics::system_line).await?,
        models,
    })
}

/// License notices: "rust", "js" or "notices" (engines, word list).
#[tauri::command]
pub async fn third_party_licenses(kind: String) -> CmdResult<&'static str> {
    Ok(match kind.as_str() {
        "rust" => include_str!("../../resources/THIRD_PARTY_RUST.txt"),
        "js" => include_str!("../../resources/THIRD_PARTY_JS.txt"),
        "notices" => include_str!("../../resources/NOTICES.txt"),
        _ => return Err(AppError::Invalid("unknown license list".into())),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineCheck {
    pub path: Option<String>,
    /// The binary started and answered `--version` / `--help`.
    pub ok: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Engines {
    pub llama: EngineCheck,
    pub whisper: EngineCheck,
    pub free_disk_gb: Option<f64>,
}

fn runs(path: &std::path::Path, arg: &str) -> bool {
    let Ok(mut child) = Command::new(path)
        .arg(arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // The first start of a Metal binary can take a while (shader compilation).
    for _ in 0..600 {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

/// First-run AI setup: are both engines present and do they start?
#[tauri::command]
pub async fn check_engines(state: State<'_, AppState>) -> CmdResult<Engines> {
    let s = state.settings.get();
    let dir = state.ai_manager.data_dir().to_path_buf();
    Ok(tokio::task::spawn_blocking(move || {
        let check = |name: &str, over: Option<&str>, arg: &str| {
            let path = resolve_binary(name, over, &dir);
            EngineCheck {
                ok: path.as_deref().is_some_and(|p| runs(p, arg)),
                path: path.map(|p| p.to_string_lossy().into_owned()),
            }
        };
        Engines {
            llama: check("llama-server", s.ai.llama_server_path.as_deref(), "--version"),
            whisper: check("whisper-server", s.ai.whisper_server_path.as_deref(), "--help"),
            free_disk_gb: models::free_disk_bytes(&dir).map(|b| b as f64 / 1e9),
        }
    })
    .await?)
}

/// Version, system, mode, engines, models, feed errors and the last log lines. No user content.
#[tauri::command]
pub async fn get_diagnostics(state: State<'_, AppState>) -> CmdResult<String> {
    let s = state.settings.get();
    let dir = state.ai_manager.data_dir().to_path_buf();
    let (feed_count, feed_errors) = state
        .db
        .call(|c| {
            let f = feeds::list(c)?;
            Ok((f.len(), f.iter().filter(|x| x.last_error.is_some()).count()))
        })
        .await?;
    let (llm, stt) = (state.ai_manager.status(), state.stt_manager.status());
    let log_dir = state.log_dir.clone();
    let (system, logs, llama_path, whisper_path) = tokio::task::spawn_blocking(move || {
        let p = |name: &str, over: Option<&str>| {
            resolve_binary(name, over, &dir)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "not found".into())
        };
        (
            diagnostics::system_line(),
            diagnostics::last_log_lines(&log_dir),
            p("llama-server", s.ai.llama_server_path.as_deref()),
            p("whisper-server", s.ai.whisper_server_path.as_deref()),
        )
    })
    .await?;
    let home = std::env::var("HOME").unwrap_or_default();
    let text = format!(
        "Tech English {VERSION} ({COMMIT}, built {BUILD_DATE})\n{system}\nmode: {}\n\
         llama-server: {} ({}) at {llama_path}\nwhisper-server: {} ({}) at {whisper_path}\n\
         feeds: {feed_count}, with errors: {feed_errors}\n\n--- last log lines ---\n{}\n",
        state.mode.get().as_str(),
        llm.state,
        llm.model_id.as_deref().unwrap_or("no model"),
        stt.state,
        stt.model_id.as_deref().unwrap_or("no model"),
        logs.join("\n"),
    );
    // Paths show the account name: write ~ instead.
    Ok(if home.is_empty() {
        text
    } else {
        text.replace(&home, "~")
    })
}

/// Put text on the clipboard (the webview's clipboard API needs a user gesture in focus).
#[tauri::command]
pub async fn copy_text(text: String) -> CmdResult<()> {
    tokio::task::spawn_blocking(move || {
        let mut child = Command::new("pbcopy").stdin(Stdio::piped()).spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        child.wait()?;
        Ok::<_, std::io::Error>(())
    })
    .await??;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashNotice {
    pub file: String,
    pub text: String,
}

/// The crash of the last run, shown once.
#[tauri::command]
pub async fn take_crash_notice(state: State<'_, AppState>) -> CmdResult<Option<CrashNotice>> {
    let log_dir = state.log_dir.clone();
    let Some((file, text)) = tokio::task::spawn_blocking(move || diagnostics::latest_crash(&log_dir)).await? else {
        return Ok(None);
    };
    let f = file.clone();
    let seen = state
        .db
        .call(move |c| {
            let seen = app_state::get(c, CRASH_SEEN_KEY)?.as_deref() == Some(f.as_str());
            app_state::set(c, CRASH_SEEN_KEY, &f)?;
            Ok(seen)
        })
        .await?;
    Ok((!seen).then_some(CrashNotice { file, text }))
}

/// Development only: crash on purpose to test the crash notice.
#[tauri::command]
pub async fn debug_panic() -> CmdResult<()> {
    if cfg!(debug_assertions) || std::env::var_os("TECH_ENGLISH_DEBUG").is_some() {
        std::thread::spawn(|| panic!("debug panic (test of the crash notice)"))
            .join()
            .ok();
        std::process::abort();
    }
    Err(AppError::Invalid("only in development builds".into()))
}
