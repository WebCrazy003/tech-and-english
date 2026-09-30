//! The `llama-server` sidecar (P2 dev spec §8). The lifecycle is shared with whisper in
//! [`crate::sidecar`]; this file only says how to find and start llama-server.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::models;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::events::EventSink;
use crate::mode::ModeManager;
use crate::settings::{Settings, SettingsStore};
pub use crate::sidecar::{
    AiStatus, BusyGuard, ChildHandle, Endpoint, HoldGuard, LaunchSpec, Prepared, ProcessLauncher, RealLauncher,
    SidecarManager, SidecarSpec, available_memory, resolve_binary, terminate,
};

pub const LLM_PID_KEY: &str = "llm_pid";
/// Free RAM needed on top of the model file.
const MEMORY_HEADROOM: u64 = 1_500_000_000;

/// The chat model's manager.
pub type AiManager = SidecarManager;

pub struct LlmSidecar;

impl SidecarSpec for LlmSidecar {
    fn component(&self) -> &'static str {
        "llm"
    }
    fn binary(&self) -> &'static str {
        "llama-server"
    }
    fn label(&self) -> &'static str {
        "The AI engine"
    }
    fn pid_key(&self) -> &'static str {
        LLM_PID_KEY
    }

    fn prepare(&self, settings: &Settings, data_dir: &Path) -> AppResult<Prepared> {
        let (model_path, model_id, model_bytes) = match settings.ai.custom_model_path.as_deref().map(PathBuf::from) {
            Some(p) if p.is_file() => {
                let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                (p, "custom".to_string(), size)
            }
            _ => {
                let m = models::resolve_active(data_dir, settings.ai.active_model.as_deref()).ok_or_else(|| {
                    AppError::ai(
                        "no_model",
                        "No AI model is downloaded yet. Download one in Settings › AI.",
                    )
                })?;
                (models::models_dir(data_dir).join(&m.file), m.id, m.size_bytes)
            }
        };
        let program =
            resolve_binary("llama-server", settings.ai.llama_server_path.as_deref(), data_dir).ok_or_else(|| {
                AppError::ai(
                    "no_engine",
                    "The AI engine (llama-server) was not found. See Settings › AI.",
                )
            })?;
        Ok(Prepared {
            program,
            model_path,
            model_id,
            model_bytes,
        })
    }

    fn args(&self, p: &Prepared, port: u16, api_key: &str, settings: &Settings) -> Vec<String> {
        vec![
            "-m".into(),
            p.model_path.to_string_lossy().into_owned(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string(),
            "--api-key".into(),
            api_key.into(),
            "-c".into(),
            settings.ai.context_size.to_string(),
            "-ngl".into(),
            "99".into(),
            "-np".into(),
            "1".into(),
            "--no-webui".into(),
        ]
    }

    fn memory_headroom(&self) -> u64 {
        MEMORY_HEADROOM
    }

    fn idle_timeout(&self, settings: &Settings) -> Duration {
        Duration::from_secs(settings.ai.idle_timeout_min as u64 * 60)
    }
}

impl SidecarManager {
    /// The llama-server manager.
    pub fn llm(
        db: Db,
        data_dir: PathBuf,
        settings: Arc<SettingsStore>,
        mode: Arc<ModeManager>,
        events: Arc<dyn EventSink>,
        launcher: Arc<dyn ProcessLauncher>,
    ) -> Arc<Self> {
        Self::new(Box::new(LlmSidecar), db, data_dir, settings, mode, events, launcher)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub use crate::sidecar::tests::Rig;
    use crate::sidecar::tests::{Kind, rig_for};

    /// An LLM manager with a fake engine and a tiny custom "model" file.
    pub async fn rig(healthy: bool) -> Rig {
        rig_for(Kind::Llm, healthy).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn launches_llama_server_with_a_key_and_the_context_size() {
        let r = rig(true).await;
        let ep = r.manager.ensure_ready(true).await.unwrap();
        let spec = r.launcher.last.lock().unwrap().clone().unwrap();
        let args = spec.args.join(" ");
        assert!(args.contains(&format!("--api-key {}", ep.api_key)) && ep.api_key.len() == 32);
        assert!(args.contains("-c 8192") && args.contains("--host 127.0.0.1"));
        assert_eq!(ep.model_id, "custom");
        assert!(spec.log_file.unwrap().ends_with("llama-server.log"));
    }

    #[tokio::test]
    async fn missing_model_or_engine_are_typed_errors() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::default();
        let e = LlmSidecar.prepare(&s, dir.path()).unwrap_err();
        assert_eq!(e.code(), "no_model");
    }
}
