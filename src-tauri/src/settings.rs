use std::sync::{Arc, RwLock};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::clock::parse_hhmm;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::voice::tutor::CorrectionPolicy;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RankingWeights {
    pub topic_relevance: f64,
    pub freshness: f64,
    pub popularity: f64,
    pub source_preference: f64,
    pub novelty: f64,
    pub user_history: f64,
}

impl Default for RankingWeights {
    fn default() -> Self {
        Self {
            // Tuned on live feeds (2026-09-29): source quality counts more, freshness less,
            // so a strong article from this morning beats a minutes-old low-quality post.
            topic_relevance: 0.30,
            freshness: 0.15,
            popularity: 0.15,
            source_preference: 0.20,
            novelty: 0.10,
            user_history: 0.10,
        }
    }
}

impl RankingWeights {
    pub fn sum(&self) -> f64 {
        self.topic_relevance
            + self.freshness
            + self.popularity
            + self.source_preference
            + self.novelty
            + self.user_history
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum WidgetStyle {
    #[default]
    Card,
    Pill,
    Hidden,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct WidgetSettings {
    pub style: WidgetStyle,
    pub always_on_top: bool,
    /// Last physical position (x, y) of the widget window.
    pub position: Option<(i32, i32)>,
}

impl Default for WidgetSettings {
    fn default() -> Self {
        Self {
            style: WidgetStyle::Card,
            always_on_top: true,
            position: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AiSettings {
    /// Catalog id of the chat model; `None` = the catalog default.
    pub active_model: Option<String>,
    /// Unload the model after this many idle minutes (2–60).
    pub idle_timeout_min: u32,
    /// 4096 or 8192.
    pub context_size: u32,
    /// 1 = very easy, 2 = B1, 3 = natural.
    pub english_level: u8,
    /// AI-written "why" for the daily pick, only when the model is already loaded.
    pub llm_why: bool,
    /// Override for the llama-server binary.
    pub llama_server_path: Option<String>,
    /// Override for the whisper-server binary (P5).
    pub whisper_server_path: Option<String>,
    /// A GGUF file outside the catalog (development).
    pub custom_model_path: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            active_model: None,
            idle_timeout_min: 10,
            context_size: 8192,
            english_level: 2,
            llm_why: true,
            llama_server_path: None,
            whisper_server_path: None,
            custom_model_path: None,
        }
    }
}

/// Text-to-speech (P4). Voices come from macOS through the webview.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TtsSettings {
    /// `SpeechSynthesisVoice.voiceURI`; `None` = the best English voice found.
    pub voice_uri: Option<String>,
    /// Reading speed for Listen (0.5–1.2).
    pub rate: f64,
    /// Speed for single words in the popup and quizzes (0.5–1.2).
    pub word_rate: f64,
    pub volume: f64,
    /// Pause between sentences.
    pub pause_ms: u32,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            voice_uri: None,
            rate: 0.85,
            word_rate: 0.7,
            volume: 1.0,
            pause_ms: 400,
        }
    }
}

/// Practice (P4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LearningSettings {
    /// Items per quiz (5–30).
    pub quiz_size: u32,
    /// FSRS target recall (0.80–0.95).
    pub desired_retention: f64,
    /// Say the word when the answer is shown.
    pub auto_pronounce: bool,
}

impl Default for LearningSettings {
    fn default() -> Self {
        Self {
            quiz_size: 10,
            desired_retention: 0.9,
            auto_pronounce: true,
        }
    }
}

/// Voice tutor defaults (P5). The level is `ai.englishLevel`; the voice and the pause between
/// sentences come from `tts`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct VoiceSettings {
    /// Catalog id of the whisper model; `None` = the catalog default.
    pub stt_model: Option<String>,
    pub correction: CorrectionPolicy,
    /// Tutor speech rate (0.5–1.2); `None` = the level's preset.
    pub rate: Option<f64>,
    /// Delete conversations after this many days; 0 = keep forever.
    pub keep_transcripts_days: u32,
    /// Stop recording after 1.2 s of silence (push-to-talk stays the default).
    pub vad_auto_stop: bool,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            stt_model: None,
            correction: CorrectionPolicy::High,
            rate: None,
            keep_transcripts_days: 90,
            vad_auto_stop: false,
        }
    }
}

/// Development switches.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct DebugSettings {
    /// Also write each recording to `debug-audio/` (P5 §4). Off by default.
    pub keep_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub onboarding_done: bool,
    pub pick_time: String,
    pub fetch_interval_standard_min: u32,
    pub fetch_interval_hibernate_min: u32,
    pub ingest_max_age_days: u32,
    /// Lessons (and items of learning feeds) may be up to this many days old (P3).
    pub lesson_max_age_days: u32,
    pub hn_include_new: bool,
    pub ranking_weights: RankingWeights,
    pub notify_daily_pick: bool,
    pub notify_high_interest: bool,
    pub notify_threshold: u8,
    pub notify_max_per_day: u8,
    pub notify_min_gap_min: u32,
    /// (start, end) as "HH:MM"; may wrap past midnight.
    pub quiet_hours: Option<(String, String)>,
    pub widget: WidgetSettings,
    pub show_dock_icon: bool,
    pub ai: AiSettings,
    /// Reader: the AI panel on the right is open.
    pub reader_ai_panel_open: bool,
    pub tts: TtsSettings,
    pub learning: LearningSettings,
    pub voice: VoiceSettings,
    pub debug: DebugSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            onboarding_done: false,
            pick_time: "08:00".into(),
            fetch_interval_standard_min: 20,
            fetch_interval_hibernate_min: 45,
            ingest_max_age_days: 7,
            lesson_max_age_days: 60,
            hn_include_new: false,
            ranking_weights: RankingWeights::default(),
            notify_daily_pick: true,
            notify_high_interest: true,
            notify_threshold: 85,
            notify_max_per_day: 3,
            notify_min_gap_min: 90,
            quiet_hours: Some(("22:00".into(), "08:00".into())),
            widget: WidgetSettings::default(),
            show_dock_icon: false,
            ai: AiSettings::default(),
            reader_ai_panel_open: true,
            tts: TtsSettings::default(),
            learning: LearningSettings::default(),
            voice: VoiceSettings::default(),
            debug: DebugSettings::default(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> AppResult<()> {
        let bad = |m: &str| Err(AppError::Invalid(m.to_string()));
        if parse_hhmm(&self.pick_time).is_none() {
            return bad("Pick time must be HH:MM");
        }
        if !(15..=30).contains(&self.fetch_interval_standard_min) {
            return bad("Standard fetch interval must be 15–30 minutes");
        }
        if !(30..=60).contains(&self.fetch_interval_hibernate_min) {
            return bad("Hibernate fetch interval must be 30–60 minutes");
        }
        if !(1..=30).contains(&self.ingest_max_age_days) {
            return bad("Maximum article age must be 1–30 days");
        }
        if !(14..=180).contains(&self.lesson_max_age_days) {
            return bad("Lesson age must be 14–180 days");
        }
        if (self.ranking_weights.sum() - 1.0).abs() > 0.001 {
            return bad("Ranking weights must add up to 1.0");
        }
        let w = &self.ranking_weights;
        if [
            w.topic_relevance,
            w.freshness,
            w.popularity,
            w.source_preference,
            w.novelty,
            w.user_history,
        ]
        .iter()
        .any(|x| *x < 0.0)
        {
            return bad("Ranking weights cannot be negative");
        }
        if self.notify_threshold > 100 {
            return bad("Notification threshold must be 0–100");
        }
        if !(2..=60).contains(&self.ai.idle_timeout_min) {
            return bad("AI idle timeout must be 2–60 minutes");
        }
        if ![4096, 8192].contains(&self.ai.context_size) {
            return bad("AI context size must be 4096 or 8192");
        }
        if !(1..=3).contains(&self.ai.english_level) {
            return bad("English level must be 1, 2 or 3");
        }
        if !(0.5..=1.2).contains(&self.tts.rate) || !(0.5..=1.2).contains(&self.tts.word_rate) {
            return bad("Speaking speed must be 0.5–1.2");
        }
        if !(0.0..=1.0).contains(&self.tts.volume) || self.tts.pause_ms > 2000 {
            return bad("Volume must be 0–1 and the pause at most 2 seconds");
        }
        if !(5..=30).contains(&self.learning.quiz_size) {
            return bad("Quiz size must be 5–30");
        }
        if !(0.8..=0.95).contains(&self.learning.desired_retention) {
            return bad("Target recall must be 80–95 %");
        }
        if self.voice.rate.is_some_and(|r| !(0.5..=1.2).contains(&r)) {
            return bad("Speaking speed must be 0.5–1.2");
        }
        if self.voice.keep_transcripts_days > 3650 {
            return bad("Keep conversations for at most 3650 days (0 = forever)");
        }
        if self.notify_max_per_day > 5 {
            return bad("At most 5 high-interest notifications per day");
        }
        if let Some((a, b)) = &self.quiet_hours
            && (parse_hhmm(a).is_none() || parse_hhmm(b).is_none())
        {
            return bad("Quiet hours must be HH:MM");
        }
        Ok(())
    }
}

/// Recursively merge `patch` into `base` (objects merge, everything else replaces).
fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                merge(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, p) => *b = p.clone(),
    }
}

pub fn apply_patch(current: &Settings, patch: &Value) -> AppResult<Settings> {
    if !patch.is_object() {
        return Err(AppError::Invalid("settings patch must be an object".into()));
    }
    let mut v = serde_json::to_value(current)?;
    merge(&mut v, patch);
    let next: Settings = serde_json::from_value(v).map_err(|e| AppError::Invalid(format!("invalid settings: {e}")))?;
    next.validate()?;
    Ok(next)
}

pub fn load(conn: &Connection) -> AppResult<Settings> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = 'app'", [], |r| r.get(0))
        .optional()?;
    Ok(match raw {
        Some(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "stored settings unreadable; using defaults");
            Settings::default()
        }),
        None => Settings::default(),
    })
}

pub fn save(conn: &Connection, s: &Settings) -> AppResult<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES('app', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![serde_json::to_string(s)?],
    )?;
    Ok(())
}

/// In-memory copy of the settings, persisted on every change.
pub struct SettingsStore {
    db: Db,
    current: RwLock<Settings>,
}

impl SettingsStore {
    pub async fn load(db: Db) -> AppResult<Arc<Self>> {
        let s = db.call(|c| load(c)).await?;
        Ok(Arc::new(Self {
            db,
            current: RwLock::new(s),
        }))
    }

    pub fn get(&self) -> Settings {
        self.current.read().unwrap().clone()
    }

    pub async fn update(&self, patch: Value) -> AppResult<Settings> {
        let next = apply_patch(&self.get(), &patch)?;
        let to_save = next.clone();
        self.db.call(move |c| save(c, &to_save)).await?;
        *self.current.write().unwrap() = next.clone();
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_valid() {
        Settings::default().validate().unwrap();
    }

    #[test]
    fn patch_merges_nested() {
        let s = apply_patch(
            &Settings::default(),
            &json!({"widget": {"style": "pill"}, "pickTime": "07:30"}),
        )
        .unwrap();
        assert_eq!(s.widget.style, WidgetStyle::Pill);
        assert!(s.widget.always_on_top, "untouched nested field kept");
        assert_eq!(s.pick_time, "07:30");
    }

    #[test]
    fn rejects_weights_not_summing_to_one() {
        let e = apply_patch(&Settings::default(), &json!({"rankingWeights": {"novelty": 0.5}})).unwrap_err();
        assert!(e.to_string().contains("add up"));
    }

    #[test]
    fn rejects_out_of_range_interval() {
        assert!(apply_patch(&Settings::default(), &json!({"fetchIntervalStandardMin": 5})).is_err());
        assert!(apply_patch(&Settings::default(), &json!({"fetchIntervalHibernateMin": 61})).is_err());
    }

    #[test]
    fn p4_ranges() {
        let d = Settings::default();
        assert_eq!((d.learning.quiz_size, d.tts.rate), (10, 0.85));
        assert!(apply_patch(&d, &json!({"learning": {"quizSize": 31}})).is_err());
        assert!(apply_patch(&d, &json!({"learning": {"desiredRetention": 0.97}})).is_err());
        assert!(apply_patch(&d, &json!({"tts": {"rate": 0.4}})).is_err());
        let s = apply_patch(&d, &json!({"tts": {"voiceUri": "com.apple.voice.premium.en-US.Zoe"}})).unwrap();
        assert_eq!(s.tts.rate, 0.85, "untouched nested field kept");
    }

    #[test]
    fn voice_defaults_and_ranges() {
        let d = Settings::default();
        assert_eq!(
            (d.voice.correction, d.voice.keep_transcripts_days),
            (CorrectionPolicy::High, 90)
        );
        assert!(!d.debug.keep_audio && !d.voice.vad_auto_stop);
        assert!(apply_patch(&d, &json!({"voice": {"rate": 1.5}})).is_err());
        let s = apply_patch(&d, &json!({"voice": {"correction": "low", "rate": 0.7}})).unwrap();
        assert_eq!((s.voice.correction, s.voice.rate), (CorrectionPolicy::Low, Some(0.7)));
        assert!(apply_patch(&d, &json!({"voice": {"correction": "sometimes"}})).is_err());
    }

    #[test]
    fn lesson_age_range() {
        assert_eq!(Settings::default().lesson_max_age_days, 60);
        assert!(apply_patch(&Settings::default(), &json!({"lessonMaxAgeDays": 13})).is_err());
        assert!(apply_patch(&Settings::default(), &json!({"lessonMaxAgeDays": 180})).is_ok());
    }

    #[test]
    fn rejects_bad_types() {
        assert!(apply_patch(&Settings::default(), &json!({"notifyThreshold": "high"})).is_err());
    }

    #[tokio::test]
    async fn store_persists() {
        let db = Db::open_in_memory().unwrap();
        let store = SettingsStore::load(db.clone()).await.unwrap();
        store.update(json!({"onboardingDone": true})).await.unwrap();
        let again = SettingsStore::load(db).await.unwrap();
        assert!(again.get().onboarding_done);
    }
}
