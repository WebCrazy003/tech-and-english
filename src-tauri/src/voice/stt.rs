//! Speech to text (P5 dev spec §3, §5): the `whisper-server` sidecar and its client.

use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex::Regex;

use crate::ai::models;
use crate::error::{AppError, AppResult};
use crate::settings::Settings;
use crate::sidecar::{Prepared, SidecarManager, SidecarSpec, resolve_binary};

pub const STT_PID_KEY: &str = "stt_pid";
/// Unload whisper this long after the last voice session ends.
pub const STT_IDLE: Duration = Duration::from_secs(5 * 60);
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct WhisperSidecar;

impl SidecarSpec for WhisperSidecar {
    fn component(&self) -> &'static str {
        "stt"
    }
    fn binary(&self) -> &'static str {
        "whisper-server"
    }
    fn label(&self) -> &'static str {
        "The speech engine"
    }
    fn pid_key(&self) -> &'static str {
        STT_PID_KEY
    }

    fn prepare(&self, settings: &Settings, data_dir: &Path) -> AppResult<Prepared> {
        let m = models::resolve(data_dir, "stt", settings.voice.stt_model.as_deref()).ok_or_else(|| {
            AppError::ai(
                "no_stt_model",
                "The speech model is not downloaded yet. Download it in Settings › Voice.",
            )
        })?;
        let program = resolve_binary("whisper-server", settings.ai.whisper_server_path.as_deref(), data_dir)
            .ok_or_else(|| {
                AppError::ai(
                    "no_stt_engine",
                    "The speech engine (whisper-server) was not found. Install it with: brew install whisper.cpp",
                )
            })?;
        Ok(Prepared {
            program,
            model_path: models::models_dir(data_dir).join(&m.file),
            model_id: m.id,
            model_bytes: m.size_bytes,
        })
    }

    /// whisper-server has no API key: it listens on 127.0.0.1 and a random port only.
    fn args(&self, p: &Prepared, port: u16, _api_key: &str, _settings: &Settings) -> Vec<String> {
        vec![
            "-m".into(),
            p.model_path.to_string_lossy().into_owned(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string(),
            "-t".into(),
            "4".into(),
            "-l".into(),
            "en".into(),
        ]
    }

    fn memory_headroom(&self) -> u64 {
        300_000_000
    }

    fn idle_timeout(&self, _settings: &Settings) -> Duration {
        STT_IDLE
    }
}

impl SidecarManager {
    /// The whisper-server manager.
    pub fn whisper(
        db: crate::db::Db,
        data_dir: std::path::PathBuf,
        settings: Arc<crate::settings::SettingsStore>,
        mode: Arc<crate::mode::ModeManager>,
        events: Arc<dyn crate::events::EventSink>,
        launcher: Arc<dyn crate::sidecar::ProcessLauncher>,
    ) -> Arc<Self> {
        Self::new(Box::new(WhisperSidecar), db, data_dir, settings, mode, events, launcher)
    }
}

// ---------------------------------------------------------------- transcripts

#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub duration_ms: u64,
}

#[async_trait]
pub trait SttProvider: Send + Sync {
    /// Start the engine if needed (session start-up), without transcribing.
    async fn warm_up(&self) -> AppResult<()>;
    async fn transcribe(&self, wav: Vec<u8>) -> AppResult<Transcript>;
}

static BRACKETED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]|\([^)]*\)|\*[^*]*\*").unwrap());

/// Trim, collapse whitespace, remove non-speech tags such as `[BLANK_AUDIO]` or `(music)`.
pub fn clean(text: &str) -> String {
    let no_tags = BRACKETED.replace_all(text, " ");
    no_tags.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Phrases whisper tends to invent for silence or noise.
const HALLUCINATIONS: &[&str] = &[
    "thank you",
    "thanks for watching",
    "you",
    "bye",
    "thank you for watching",
];

/// Quiet clips below this level whose text is a known hallucination count as silence.
pub const HALLUCINATION_DBFS: f32 = -40.0;

pub fn dbfs(rms: f32) -> f32 {
    if rms <= 0.0 { -120.0 } else { 20.0 * rms.log10() }
}

/// `text` if it is real speech; "" if it is a typical silence hallucination on a quiet clip.
pub fn filter_hallucination(text: &str, rms: f32) -> String {
    if dbfs(rms) >= HALLUCINATION_DBFS {
        return text.to_string();
    }
    let bare: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if HALLUCINATIONS.contains(&bare.as_str()) || bare.starts_with("subtitles by") {
        String::new()
    } else {
        text.to_string()
    }
}

/// Posts WAV clips to whisper-server's `/inference`.
pub struct WhisperServerProvider {
    manager: Arc<SidecarManager>,
    http: reqwest::Client,
}

impl WhisperServerProvider {
    pub fn new(manager: Arc<SidecarManager>) -> Self {
        Self {
            manager,
            http: reqwest::Client::builder().no_proxy().build().expect("http client"),
        }
    }
}

#[async_trait]
impl SttProvider for WhisperServerProvider {
    async fn warm_up(&self) -> AppResult<()> {
        self.manager.ensure_ready(true).await.map(|_| ())
    }

    async fn transcribe(&self, wav: Vec<u8>) -> AppResult<Transcript> {
        let ep = self.manager.ensure_ready(true).await?;
        let _busy = self.manager.busy();
        let t0 = Instant::now();
        let file = reqwest::multipart::Part::bytes(wav)
            .file_name("clip.wav")
            .mime_str("audio/wav")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // No prompt: a prompt would bias whisper towards "fixing" the learner's grammar.
        let form = reqwest::multipart::Form::new()
            .part("file", file)
            .text("temperature", "0")
            .text("response_format", "json");
        let resp = self
            .http
            .post(format!("{}/inference", ep.base_url))
            .multipart(form)
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::ai("stt_error", format!("Speech recognition failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::ai(
                "stt_error",
                format!("Speech recognition failed (HTTP {}).", resp.status().as_u16()),
            ));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::ai("stt_error", format!("Speech recognition answer unreadable: {e}")))?;
        let text = clean(v["text"].as_str().unwrap_or(""));
        Ok(Transcript {
            text,
            duration_ms: t0.elapsed().as_millis() as u64,
        })
    }
}

/// Returns scripted transcripts (tests).
#[derive(Default)]
pub struct FakeStt {
    pub texts: Mutex<std::collections::VecDeque<String>>,
    pub calls: std::sync::atomic::AtomicUsize,
}

impl FakeStt {
    pub fn with(texts: &[&str]) -> Self {
        let f = Self::default();
        f.texts.lock().unwrap().extend(texts.iter().map(|s| s.to_string()));
        f
    }
}

#[async_trait]
impl SttProvider for FakeStt {
    async fn warm_up(&self) -> AppResult<()> {
        Ok(())
    }
    async fn transcribe(&self, _wav: Vec<u8>) -> AppResult<Transcript> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let text = self.texts.lock().unwrap().pop_front().unwrap_or_default();
        Ok(Transcript {
            text: clean(&text),
            duration_ms: 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::tests::{Kind, rig_for};
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, ResponseTemplate};

    #[test]
    fn cleaning_removes_tags_and_spaces() {
        assert_eq!(clean(" [BLANK_AUDIO]\n"), "");
        assert_eq!(
            clean(" In my job I work.\n Every morning (music) we load [inaudible] files.\n"),
            "In my job I work. Every morning we load files."
        );
    }

    #[test]
    fn hallucinations_only_on_quiet_clips() {
        let quiet = 0.005; // ≈ −46 dBFS
        let loud = 0.1; // −20 dBFS
        assert_eq!(filter_hallucination("Thank you.", quiet), "");
        assert_eq!(filter_hallucination("Thanks for watching!", quiet), "");
        assert_eq!(filter_hallucination("Subtitles by the Amara.org community", quiet), "");
        assert_eq!(filter_hallucination("Thank you.", loud), "Thank you.");
        assert_eq!(filter_hallucination("I like Rust.", quiet), "I like Rust.");
        assert!((dbfs(0.01) + 40.0).abs() < 0.01);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn posts_multipart_and_cleans_the_text() {
        let r = rig_for(Kind::Whisper, true).await;
        Mock::given(method("POST"))
            .and(path("/inference"))
            .and(body_string_contains("name=\"temperature\""))
            .and(body_string_contains("name=\"response_format\""))
            .and(body_string_contains("RIFF"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "text": " Yesterday I deploy the application.\n"
            })))
            .mount(&r.server)
            .await;
        let p = WhisperServerProvider::new(r.manager.clone());
        let t = p.transcribe(b"RIFF....WAVE".to_vec()).await.unwrap();
        assert_eq!(t.text, "Yesterday I deploy the application.");
        let args = r.launcher.last.lock().unwrap().clone().unwrap().args.join(" ");
        assert!(args.contains("-l en") && args.contains("-t 4") && args.contains("ggml-base.en.bin"));
        assert!(!args.contains("--api-key"));
        assert_eq!(r.manager.status().state, "ready", "busy cleared");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn server_errors_are_stt_errors() {
        let r = rig_for(Kind::Whisper, true).await;
        Mock::given(path("/inference"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&r.server)
            .await;
        let e = WhisperServerProvider::new(r.manager.clone())
            .transcribe(vec![0; 10])
            .await
            .unwrap_err();
        assert_eq!(e.code(), "stt_error");
    }

    #[test]
    fn missing_model_is_a_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let e = WhisperSidecar.prepare(&Settings::default(), dir.path()).unwrap_err();
        assert_eq!(e.code(), "no_stt_model");
    }

    #[tokio::test]
    async fn fake_stt_scripts() {
        let f = FakeStt::with(&["[BLANK_AUDIO]", "Hi"]);
        assert_eq!(f.transcribe(vec![]).await.unwrap().text, "");
        assert_eq!(f.transcribe(vec![]).await.unwrap().text, "Hi");
        assert_eq!(f.transcribe(vec![]).await.unwrap().text, "");
    }
}
