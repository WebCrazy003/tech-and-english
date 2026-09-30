//! The conversation engine (P5 dev spec §8): one voice session at a time, turn processing,
//! local intents, the repeat check, pronunciation drills, observations and the review.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::capture::{self, AutoStop, Clip, Recording};
use super::intents::{self, Intent};
use super::reply_stream::ReplyExtractor;
use super::review::{self, SessionReview, Suggestion};
use super::similarity;
use super::stt::{self, SttProvider};
use super::tutor::{self, Correction, CorrectionPolicy, Topic, TutorTurnOut};
use crate::ai::provider::{ChatMsg, LlmRequest};
use crate::ai::service::{AiService, DerivKind, Sink as AiSink};
use crate::clock::{Clock, fmt_ts};
use crate::db::Db;
use crate::db::repo::{articles, topics, voice as repo};
use crate::error::{AppError, AppResult};
use crate::events::{self, EventSink};
use crate::learning::dictionary::DictEntry;
use crate::learning::vocab::VocabService;
use crate::mode::{Mode, ModeManager};
use crate::settings::SettingsStore;
use crate::sidecar::{HoldGuard, SidecarManager};

pub const DIDNT_HEAR: &str = "I didn't hear anything. Hold the button while you speak.";
const NO_SIGNAL: &str = "I got no sound from the microphone. Check System Settings › Privacy & Security › Microphone.";
const SORRY: &str = "Sorry, I had a problem. Could you say that again?";
const DRILL_RATE: f64 = 0.6;
const DRILL_ATTEMPTS: usize = 3;
const SUMMARY_WORDS: usize = 400;
const LATENCY_KEEP: usize = 50;

// ---------------------------------------------------------------- types

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSettings {
    pub level: u8,
    /// Tutor speech rate (0.5–1.2).
    pub rate: f64,
    pub pause_ms: u32,
    pub correction: CorrectionPolicy,
    #[serde(default)]
    pub voice_uri: Option<String>,
}

pub fn level_rate(level: u8) -> f64 {
    match level {
        1 => 0.7,
        3 => 1.0,
        _ => 0.85,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Discuss,
    AwaitRepeat {
        target: String,
        attempts: u8,
    },
    Drill {
        word: String,
        hint: Option<String>,
        heard: Vec<String>,
    },
}

impl Phase {
    fn name(&self) -> &'static str {
        match self {
            Phase::Discuss => "discuss",
            Phase::AwaitRepeat { .. } => "awaitRepeat",
            Phase::Drill { .. } => "drill",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrillInfo {
    pub word: String,
    pub hint: Option<String>,
    /// 1-based attempt the learner is on now.
    pub attempt: usize,
    pub max_attempts: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VoiceEvent {
    #[serde(rename_all = "camelCase")]
    Loading {
        component: String,
    },
    #[serde(rename_all = "camelCase")]
    Ready {
        conversation_id: i64,
        settings: SessionSettings,
    },
    Transcript {
        text: String,
    },
    Notice {
        text: String,
    },
    /// One sentence to speak. `turn` lets the UI drop sentences of a reply it interrupted.
    #[serde(rename_all = "camelCase")]
    TutorSentence {
        turn: u64,
        text: String,
        /// Relative to the session rate (e.g. −0.15 for "Listen again").
        rate_delta: Option<f64>,
        /// Absolute rate (drill words at 0.6).
        rate: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    TutorDone {
        turn: u64,
        turn_id: i64,
        text: String,
        correction: Option<Correction>,
        phase: String,
        repeat_target: Option<String>,
        drill: Option<DrillInfo>,
        local: bool,
    },
    #[serde(rename_all = "camelCase")]
    LocalAction {
        action: String,
        rate: Option<f64>,
    },
    Error {
        code: String,
        message: String,
    },
}

pub type Sink = Arc<dyn Fn(VoiceEvent) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Voice,
    Typed,
}

/// Per-turn timings (P5 dev spec §10), in ms after the end of speech.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnTiming {
    pub turn: u64,
    pub typed: bool,
    pub stt_ms: Option<u64>,
    pub first_token_ms: Option<u64>,
    pub first_sentence_ms: Option<u64>,
    pub llm_done_ms: Option<u64>,
    /// Reported by the UI: end of speech → first tutor audio.
    pub tts_start_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Percentiles {
    pub count: usize,
    pub p50: Option<u64>,
    pub p90: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyReport {
    pub total: Percentiles,
    pub stt: Percentiles,
    pub first_token: Percentiles,
    pub first_sentence: Percentiles,
    pub llm_done: Percentiles,
    pub recent: Vec<TurnTiming>,
}

pub fn percentiles(mut v: Vec<u64>) -> Percentiles {
    v.sort_unstable();
    let at = |q: f64| -> Option<u64> {
        if v.is_empty() {
            return None;
        }
        let i = ((v.len() as f64 * q).ceil() as usize).clamp(1, v.len()) - 1;
        Some(v[i])
    };
    Percentiles {
        count: v.len(),
        p50: at(0.5),
        p90: at(0.9),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSession {
    pub conversation_id: i64,
    pub article_id: Option<i64>,
    pub article_title: Option<String>,
    pub settings: SessionSettings,
    pub recording: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndReason {
    User,
    Hibernate,
    Quit,
}

struct Session {
    id: i64,
    article_title: Option<String>,
    settings: SessionSettings,
    phase: Phase,
    topic: TopicText,
    /// Fixed for the whole session (prompt cache).
    system: String,
    /// Sent exactly as before so llama-server reuses its cache.
    history: Vec<ChatMsg>,
    mistakes: Vec<String>,
    speaking: Duration,
    last_reply: String,
    max_exchanges: usize,
    _holds: Vec<HoldGuard>,
}

pub type DictionaryFn = Arc<dyn Fn(&str) -> Option<DictEntry> + Send + Sync>;

/// What the engine needs; tests pass fakes and no sidecar managers.
pub struct VoiceDeps {
    pub db: Db,
    pub clock: Arc<dyn Clock>,
    pub settings: Arc<SettingsStore>,
    pub events: Arc<dyn EventSink>,
    pub mode: Arc<ModeManager>,
    pub ai: Arc<AiService>,
    pub stt: Arc<dyn SttProvider>,
    pub vocab: Arc<VocabService>,
    pub llm_manager: Option<Arc<SidecarManager>>,
    pub stt_manager: Option<Arc<SidecarManager>>,
    pub data_dir: PathBuf,
    /// The macOS dictionary (blocking); replaced in tests.
    pub dictionary: DictionaryFn,
}

pub struct VoiceEngine {
    d: VoiceDeps,
    active: tokio::sync::Mutex<Option<Session>>,
    /// The LLM turn in progress (cancelled when the session ends).
    turn_token: Mutex<Option<CancellationToken>>,
    recording: Mutex<Option<Recording>>,
    rec_stopped: Mutex<Option<Instant>>,
    next_turn: AtomicU64,
    timings: Mutex<VecDeque<TurnTiming>>,
    reviews: Mutex<HashMap<i64, SessionReview>>,
    /// Summary of the active session for the widget and a reloaded window.
    info: Mutex<Option<ActiveSession>>,
}

fn strip_markdown(s: &str) -> String {
    let text = s.replace("**", "").replace("__", "").replace('#', "");
    let text = text.replace("_Based on the short description only._", "");
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = words[..words.len().min(SUMMARY_WORDS)].join(" ");
    if words.len() > SUMMARY_WORDS {
        out.push('…');
    }
    out
}

impl VoiceEngine {
    pub fn new(d: VoiceDeps) -> Arc<Self> {
        Arc::new(Self {
            d,
            active: tokio::sync::Mutex::new(None),
            turn_token: Mutex::new(None),
            recording: Mutex::new(None),
            rec_stopped: Mutex::new(None),
            next_turn: AtomicU64::new(1),
            timings: Mutex::new(VecDeque::new()),
            reviews: Mutex::new(HashMap::new()),
            info: Mutex::new(None),
        })
    }

    pub fn active(&self) -> Option<ActiveSession> {
        self.info.lock().unwrap().clone().map(|mut a| {
            a.recording = self.recording.lock().unwrap().is_some();
            a
        })
    }

    pub fn is_active(&self) -> bool {
        self.info.lock().unwrap().is_some()
    }

    fn now(&self) -> String {
        fmt_ts(self.d.clock.now())
    }

    fn publish_active(&self, a: Option<ActiveSession>) {
        *self.info.lock().unwrap() = a.clone();
        events::emit(
            self.d.events.as_ref(),
            events::VOICE_ACTIVE,
            &json!({ "active": a.is_some(), "conversationId": a.map(|x| x.conversation_id) }),
        );
    }

    /// Default session settings from Settings (the setup form starts from these).
    pub fn default_settings(&self) -> SessionSettings {
        let s = self.d.settings.get();
        let level = s.ai.english_level;
        SessionSettings {
            level,
            rate: s.voice.rate.unwrap_or_else(|| level_rate(level)),
            pause_ms: s.tts.pause_ms.min(1500),
            correction: s.voice.correction,
            voice_uri: s.tts.voice_uri,
        }
    }

    // ------------------------------------------------------------ start / end

    pub async fn start(
        self: &Arc<Self>,
        article_id: Option<i64>,
        settings: SessionSettings,
        force: bool,
        sink: Sink,
    ) -> AppResult<i64> {
        if self.d.mode.get() == Mode::Hibernate {
            return Err(AppError::Hibernating);
        }
        if !(1..=3).contains(&settings.level) || !(0.5..=1.2).contains(&settings.rate) || settings.pause_ms > 1500 {
            return Err(AppError::Invalid("Check the session settings".into()));
        }
        if self.is_active() {
            self.end(EndReason::User, false).await?;
        }
        let mut holds = Vec::new();
        for m in [&self.d.llm_manager, &self.d.stt_manager].into_iter().flatten() {
            holds.push(m.hold());
        }
        self.d.ai.set_quiet(true);
        *self.rec_stopped.lock().unwrap() = None;
        let prepared = self.prepare(article_id, force, &sink).await;
        let (title, topic_text) = match prepared {
            Ok(x) => x,
            Err(e) => {
                self.d.ai.set_quiet(false);
                return Err(e);
            }
        };
        let system = system_for(&settings, title.as_deref(), &topic_text);
        let (now, settings_json) = (self.now(), serde_json::to_value(&settings)?);
        let id = self
            .d
            .db
            .call(move |c| repo::create(c, article_id, &settings_json, &now))
            .await?;
        let ctx = self.d.settings.get().ai.context_size;
        let session = Session {
            id,
            article_title: title.clone(),
            settings: settings.clone(),
            phase: Phase::Discuss,
            topic: topic_text,
            system,
            history: Vec::new(),
            mistakes: Vec::new(),
            speaking: Duration::ZERO,
            last_reply: String::new(),
            max_exchanges: if ctx >= 8192 { 12 } else { 5 },
            _holds: holds,
        };
        tracing::info!(conversation = id, article = ?article_id, level = settings.level, "voice session started");
        let mut guard = self.active.lock().await;
        *guard = Some(session);
        self.publish_active(Some(ActiveSession {
            conversation_id: id,
            article_id,
            article_title: title,
            settings: settings.clone(),
            recording: false,
        }));
        sink(VoiceEvent::Ready {
            conversation_id: id,
            settings,
        });
        let s = guard.as_mut().expect("just set");
        let opening = if article_id.is_some() {
            tutor::OPENING_INSTRUCTION
        } else {
            tutor::OPENING_INSTRUCTION_FREE
        };
        self.llm_turn(s, opening, "", None, &sink).await;
        Ok(id)
    }

    /// Start both engines in parallel and get the article summary.
    async fn prepare(
        &self,
        article_id: Option<i64>,
        force: bool,
        sink: &Sink,
    ) -> AppResult<(Option<String>, TopicText)> {
        let llm = async {
            if let Some(m) = &self.d.llm_manager
                && !m.is_ready()
            {
                sink(VoiceEvent::Loading {
                    component: "llm".into(),
                });
                m.ensure_ready(force).await?;
            }
            Ok::<_, AppError>(())
        };
        let stt = async {
            if let Some(m) = &self.d.stt_manager
                && !m.is_ready()
            {
                sink(VoiceEvent::Loading {
                    component: "stt".into(),
                });
            }
            self.d.stt.warm_up().await
        };
        let (a, b) = tokio::join!(llm, stt);
        a?;
        b?;
        let Some(article_id) = article_id else {
            let names = self
                .d
                .db
                .call(|c| {
                    Ok(topics::list(c)?
                        .into_iter()
                        .filter(|t| t.enabled)
                        .map(|t| t.name)
                        .take(8)
                        .collect::<Vec<_>>())
                })
                .await?;
            return Ok((None, TopicText::Free(names)));
        };
        let title = self
            .d
            .db
            .call(move |c| Ok(articles::get_item(c, article_id)?.title))
            .await?;
        let cached = self
            .d
            .db
            .call(move |c| crate::db::repo::ai::get_derivative(c, article_id, DerivKind::SummaryB1.as_str()))
            .await?;
        if cached.is_none() {
            sink(VoiceEvent::Loading {
                component: "summary".into(),
            });
        }
        let noop: AiSink = Arc::new(|_| {});
        let summary = self
            .d
            .ai
            .derivative(
                article_id,
                DerivKind::SummaryB1,
                false,
                force,
                &CancellationToken::new(),
                &noop,
            )
            .await?
            .unwrap_or_default();
        Ok((Some(title), TopicText::Article(strip_markdown(&summary))))
    }

    /// End the active session. `review` builds the review now (the user pressed End).
    pub async fn end(&self, reason: EndReason, review: bool) -> AppResult<Option<SessionReview>> {
        if let Some(t) = self.turn_token.lock().unwrap().take() {
            t.cancel();
        }
        drop(self.recording.lock().unwrap().take());
        let session = self.active.lock().await.take();
        let Some(s) = session else {
            return Ok(None);
        };
        let (id, secs, now) = (s.id, s.speaking.as_secs() as i64, self.now());
        self.d.db.call(move |c| repo::end(c, id, &now, secs)).await?;
        drop(s); // releases the sidecar holds: whisper unloads 5 min later
        self.d.ai.set_quiet(false);
        self.publish_active(None);
        tracing::info!(conversation = id, reason = ?reason, speaking_secs = secs, "voice session ended");
        if review && reason == EndReason::User {
            return self.review(id).await.map(Some);
        }
        Ok(None)
    }

    pub async fn update_settings(
        &self,
        rate: Option<f64>,
        level: Option<u8>,
        correction: Option<CorrectionPolicy>,
    ) -> AppResult<SessionSettings> {
        let mut guard = self.active.lock().await;
        let s = guard
            .as_mut()
            .ok_or_else(|| AppError::Invalid("No conversation is running".into()))?;
        if let Some(r) = rate {
            s.settings.rate = r.clamp(0.5, 1.2);
        }
        // Level and correction change the system prompt; they apply from the next turn and
        // cost one re-read of the history.
        if level.is_some() || correction.is_some() {
            if let Some(l) = level.filter(|l| (1..=3).contains(l)) {
                s.settings.level = l;
            }
            if let Some(c) = correction {
                s.settings.correction = c;
            }
            s.system = system_for(&s.settings, s.article_title.as_deref(), &s.topic);
        }
        let out = s.settings.clone();
        if let Some(a) = self.info.lock().unwrap().as_mut() {
            a.settings = out.clone();
        }
        Ok(out)
    }

    // ------------------------------------------------------------ recording

    pub fn start_recording(self: &Arc<Self>) -> AppResult<()> {
        if !self.is_active() {
            return Err(AppError::Invalid("No conversation is running".into()));
        }
        if self.d.mode.get() == Mode::Hibernate {
            return Err(AppError::Hibernating);
        }
        if self.recording.lock().unwrap().is_some() {
            return Ok(());
        }
        capture::ensure_mic_permission()?;
        let ev = self.d.events.clone();
        let ev2 = self.d.events.clone();
        let rec = Recording::start(
            self.d.settings.get().voice.vad_auto_stop,
            Box::new(move |rms| events::emit(ev.as_ref(), events::VOICE_LEVEL, &json!({ "rms": rms }))),
            Box::new(move |reason: AutoStop| {
                events::emit(
                    ev2.as_ref(),
                    events::VOICE_AUTOSTOP,
                    &json!({ "reason": reason.as_str() }),
                );
            }),
        )?;
        *self.recording.lock().unwrap() = Some(rec);
        Ok(())
    }

    /// Close the mic, transcribe and answer.
    pub async fn stop_recording(self: &Arc<Self>, sink: Sink) -> AppResult<()> {
        let rec = self.recording.lock().unwrap().take();
        let Some(rec) = rec else {
            return Ok(());
        };
        let stopped = Instant::now();
        let clip = tokio::task::spawn_blocking(move || rec.stop()).await??;
        self.handle_clip(clip, stopped, sink).await
    }

    pub async fn handle_clip(self: &Arc<Self>, clip: Clip, stopped: Instant, sink: Sink) -> AppResult<()> {
        *self.rec_stopped.lock().unwrap() = Some(stopped);
        if clip.duration >= capture::MIN_LENGTH && clip.peak == 0.0 {
            sink(VoiceEvent::Notice { text: NO_SIGNAL.into() });
            return Ok(());
        }
        if clip.is_silence() {
            sink(VoiceEvent::Notice {
                text: DIDNT_HEAR.into(),
            });
            return Ok(());
        }
        let wav = capture::encode_wav(&clip.samples_16k_mono)?;
        if self.d.settings.get().debug.keep_audio {
            self.keep_debug_audio(&wav).await;
            sink(VoiceEvent::Notice {
                text: "Recording saved (debug)".into(),
            });
        }
        let t = self.d.stt.transcribe(wav).await?;
        let stt_ms = stopped.elapsed().as_millis() as u64;
        let text = stt::filter_hallucination(&t.text, clip.rms);
        if let Some(s) = self.active.lock().await.as_mut() {
            s.speaking += clip.duration;
        }
        self.input(text, Source::Voice, Some(stt_ms), sink).await
    }

    async fn keep_debug_audio(&self, wav: &[u8]) {
        let dir = self.d.data_dir.join("debug-audio");
        let conv = self
            .info
            .lock()
            .unwrap()
            .as_ref()
            .map(|a| a.conversation_id)
            .unwrap_or(0);
        let path = dir.join(format!("{conv}-{}.wav", self.next_turn.load(Ordering::SeqCst)));
        let wav = wav.to_vec();
        let _ =
            tokio::task::spawn_blocking(move || std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(path, wav)))
                .await;
    }

    pub async fn send_text(self: &Arc<Self>, text: String, sink: Sink) -> AppResult<()> {
        *self.rec_stopped.lock().unwrap() = Some(Instant::now());
        self.input(stt::clean(&text), Source::Typed, None, sink).await
    }

    // ------------------------------------------------------------ turn processing

    async fn input(self: &Arc<Self>, text: String, source: Source, stt_ms: Option<u64>, sink: Sink) -> AppResult<()> {
        let mut guard = self.active.lock().await;
        let s = guard
            .as_mut()
            .ok_or_else(|| AppError::Invalid("No conversation is running".into()))?;
        if text.trim().is_empty() {
            sink(VoiceEvent::Notice {
                text: DIDNT_HEAR.into(),
            });
            return Ok(());
        }
        sink(VoiceEvent::Transcript { text: text.clone() });
        let (id, t, now) = (s.id, text.clone(), self.now());
        self.d
            .db
            .call(move |c| repo::add_turn(c, id, "user", &t, None, &now).map(|_| ()))
            .await?;
        let timing = TurnTiming {
            typed: source == Source::Typed,
            stt_ms,
            ..Default::default()
        };
        match intents::detect(&text) {
            Some(Intent::Repeat) => {
                sink(VoiceEvent::LocalAction {
                    action: "replay".into(),
                    rate: None,
                });
                return Ok(());
            }
            Some(i @ (Intent::Slower | Intent::Faster)) => {
                let slower = i == Intent::Slower;
                let delta = if slower { -0.1 } else { 0.1 };
                s.settings.rate = ((s.settings.rate + delta) * 100.0).round() / 100.0;
                s.settings.rate = s.settings.rate.clamp(0.5, 1.2);
                if let Some(a) = self.info.lock().unwrap().as_mut() {
                    a.settings.rate = s.settings.rate;
                }
                sink(VoiceEvent::LocalAction {
                    action: if slower { "slower" } else { "faster" }.into(),
                    rate: Some(s.settings.rate),
                });
                return Ok(());
            }
            Some(Intent::End) => {
                sink(VoiceEvent::LocalAction {
                    action: "end".into(),
                    rate: None,
                });
                return Ok(());
            }
            Some(Intent::Pronounce(word)) => {
                self.begin_drill(s, word, &sink).await;
                return Ok(());
            }
            Some(Intent::Define(word)) => {
                if matches!(s.phase, Phase::Discuss) {
                    let kind = if word.contains(' ') {
                        "unknown_phrase"
                    } else {
                        "unknown_word"
                    };
                    self.observe(s.id, kind, &word, json!({ "context": text })).await;
                }
            }
            None => {}
        }
        match s.phase.clone() {
            Phase::AwaitRepeat { target, attempts } => {
                if similarity::repeat_passes(&text, &target) {
                    s.phase = Phase::Discuss;
                    self.local_reply(s, &[("Good. Let's continue.", None, None)], &sink)
                        .await;
                    self.llm_turn(s, tutor::REPEATED_INSTRUCTION, &text, Some(timing), &sink)
                        .await;
                } else if attempts == 0 {
                    s.phase = Phase::AwaitRepeat {
                        target: target.clone(),
                        attempts: 1,
                    };
                    self.local_reply(
                        s,
                        &[("Almost. Listen again:", None, None), (&target, Some(-0.15), None)],
                        &sink,
                    )
                    .await;
                } else {
                    s.phase = Phase::Discuss;
                    self.local_reply(s, &[("Good try. Let's continue.", None, None)], &sink)
                        .await;
                    self.llm_turn(s, tutor::TRIED_INSTRUCTION, &text, Some(timing), &sink)
                        .await;
                }
            }
            Phase::Drill { word, hint, mut heard } => {
                heard.push(text.clone());
                let passed = similarity::contains_word(&text, &word);
                if passed || heard.len() >= DRILL_ATTEMPTS {
                    let detail = json!({
                        "target": word,
                        "attempts": heard.iter().map(|h| json!({ "heard": h })).collect::<Vec<_>>(),
                        "passed": passed,
                        "hint": hint,
                    });
                    self.observe(s.id, "pronunciation", &word, detail).await;
                    s.phase = Phase::Discuss;
                    let msg = if passed { "Good!" } else { "Good try. Let's continue." };
                    self.local_reply(s, &[(msg, None, None)], &sink).await;
                    let instruction =
                        format!("The learner practised saying the word \"{word}\". Continue the conversation.");
                    self.llm_turn(s, &instruction, &text, Some(timing), &sink).await;
                } else {
                    let first = format!("I heard \"{text}\". Listen again:");
                    s.phase = Phase::Drill {
                        word: word.clone(),
                        hint,
                        heard,
                    };
                    self.local_reply(s, &[(&first, None, None), (&word, None, Some(DRILL_RATE))], &sink)
                        .await;
                }
            }
            Phase::Discuss => {
                self.llm_turn(s, "", &text, Some(timing), &sink).await;
            }
        }
        Ok(())
    }

    fn turn_no(&self) -> u64 {
        self.next_turn.fetch_add(1, Ordering::SeqCst)
    }

    fn drill_info(phase: &Phase) -> Option<DrillInfo> {
        match phase {
            Phase::Drill { word, hint, heard } => Some(DrillInfo {
                word: word.clone(),
                hint: hint.clone(),
                attempt: heard.len() + 1,
                max_attempts: DRILL_ATTEMPTS,
            }),
            _ => None,
        }
    }

    /// Short replies made without the LLM ("Good. Let's continue."): (text, rate delta, rate).
    async fn local_reply(&self, s: &mut Session, parts: &[(&str, Option<f64>, Option<f64>)], sink: &Sink) {
        let turn = self.turn_no();
        for (text, rate_delta, rate) in parts {
            sink(VoiceEvent::TutorSentence {
                turn,
                text: text.to_string(),
                rate_delta: *rate_delta,
                rate: *rate,
            });
        }
        let text = parts.iter().map(|p| p.0).collect::<Vec<_>>().join(" ");
        let (id, t, now) = (s.id, text.clone(), self.now());
        let turn_id = self
            .d
            .db
            .call(move |c| repo::add_turn(c, id, "tutor", &t, Some(&json!({ "local": true })), &now).map(|t| t.id))
            .await
            .unwrap_or(0);
        s.last_reply = text.clone();
        let repeat_target = match &s.phase {
            Phase::AwaitRepeat { target, .. } => Some(target.clone()),
            _ => None,
        };
        sink(VoiceEvent::TutorDone {
            turn,
            turn_id,
            text,
            correction: None,
            phase: s.phase.name().into(),
            repeat_target,
            drill: Self::drill_info(&s.phase),
            local: true,
        });
    }

    async fn observe(&self, conversation: i64, kind: &'static str, text: &str, detail: Value) {
        let (text, now) = (text.trim().to_string(), self.now());
        if text.is_empty() {
            return;
        }
        let r = self
            .d
            .db
            .call(move |c| repo::add_observation(c, conversation, kind, &text, Some(&detail), &now))
            .await;
        if let Err(e) = r {
            tracing::warn!(error = %e, kind, "could not save an observation");
        }
    }

    fn trim_history(s: &mut Session) {
        // Dropping old turns makes llama-server re-read the rest, so drop many at once.
        if s.history.len() > s.max_exchanges * 2 {
            let keep = 3 * 2;
            let cut = s.history.len() - keep;
            s.history.drain(..cut);
        }
    }

    /// One streamed tutor turn. Errors become the spoken "Sorry…" reply; the phase stays.
    async fn llm_turn(
        &self,
        s: &mut Session,
        instruction: &str,
        transcript: &str,
        timing: Option<TurnTiming>,
        sink: &Sink,
    ) {
        let turn = self.turn_no();
        let started = self.rec_stopped.lock().unwrap().unwrap_or_else(Instant::now);
        let measured = timing.is_some();
        let mut timing = timing.unwrap_or_default();
        timing.turn = turn;
        let user = tutor::user_message(instruction, transcript, &s.mistakes);
        let mut messages = vec![ChatMsg::system(s.system.clone())];
        messages.extend(s.history.iter().cloned());
        messages.push(user.clone());
        let mut req = LlmRequest::new(messages, 450);
        req.temperature = tutor::TEMPERATURE;
        req.json_schema = Some(tutor::schema());
        let token = CancellationToken::new();
        *self.turn_token.lock().unwrap() = Some(token.clone());

        let mut extractor = ReplyExtractor::new();
        let mut spoken: Vec<String> = Vec::new();
        let mut raw = String::new();
        let result: AppResult<bool> = async {
            let mut stream = self.d.ai.interactive_stream(req, true).await?;
            loop {
                tokio::select! {
                    biased;
                    _ = token.cancelled() => return Ok(false),
                    next = stream.next() => match next {
                        None => return Ok(true),
                        Some(Err(e)) => return Err(e),
                        Some(Ok(chunk)) => {
                            if timing.first_token_ms.is_none() {
                                timing.first_token_ms = Some(started.elapsed().as_millis() as u64);
                            }
                            raw.push_str(&chunk);
                            for sentence in extractor.feed(&chunk) {
                                if timing.first_sentence_ms.is_none() {
                                    timing.first_sentence_ms = Some(started.elapsed().as_millis() as u64);
                                }
                                sink(VoiceEvent::TutorSentence { turn, text: sentence.clone(), rate_delta: None, rate: None });
                                spoken.push(sentence);
                            }
                        }
                    }
                }
            }
        }
        .await;
        self.turn_token.lock().unwrap().take();
        timing.llm_done_ms = Some(started.elapsed().as_millis() as u64);

        match result {
            Ok(false) => return, // the session ended
            Ok(true) => {}
            Err(e) => {
                tracing::warn!(code = e.code(), conversation = s.id, "tutor turn failed");
                if matches!(e, AppError::Hibernating) {
                    sink(VoiceEvent::Error {
                        code: e.code().into(),
                        message: e.to_string(),
                    });
                    return;
                }
                if spoken.is_empty() {
                    self.local_reply(s, &[(SORRY, None, None)], sink).await;
                    return;
                }
            }
        }
        let (rest, parsed) = extractor.finish();
        for sentence in rest {
            if timing.first_sentence_ms.is_none() {
                timing.first_sentence_ms = Some(started.elapsed().as_millis() as u64);
            }
            sink(VoiceEvent::TutorSentence {
                turn,
                text: sentence.clone(),
                rate_delta: None,
                rate: None,
            });
            spoken.push(sentence);
        }
        let mut out = match parsed {
            Ok(o) => o,
            Err(_) if !extractor.reply().trim().is_empty() => TutorTurnOut {
                reply: extractor.reply().trim().to_string(),
                ..Default::default()
            },
            Err(_) => {
                tracing::warn!(conversation = s.id, "tutor answer unreadable");
                self.local_reply(s, &[(SORRY, None, None)], sink).await;
                return;
            }
        };
        tutor::reconcile(&mut out, transcript);
        // The model does not always end with "Please say: …" when it asks for a repeat.
        if let Some(c) = out.correction.as_ref().filter(|c| c.ask_repeat)
            && !out.reply.to_lowercase().contains("please say")
        {
            let extra = format!("Please say: {}", c.corrected);
            sink(VoiceEvent::TutorSentence {
                turn,
                text: extra.clone(),
                rate_delta: None,
                rate: None,
            });
            out.reply = format!("{} {extra}", out.reply);
        }

        let meta = serde_json::to_value(&out).unwrap_or(Value::Null);
        let (id, reply, now) = (s.id, out.reply.clone(), self.now());
        let turn_id = self
            .d
            .db
            .call(move |c| repo::add_turn(c, id, "tutor", &reply, Some(&meta), &now).map(|t| t.id))
            .await
            .unwrap_or(0);
        s.history.push(user);
        s.history.push(ChatMsg::assistant(raw));
        Self::trim_history(s);
        s.last_reply = out.reply.clone();

        let context_user = transcript.to_string();
        if let Some(c) = &out.correction {
            let detail = json!({
                "original": c.original, "corrected": c.corrected, "explanation": c.explanation,
                "context": if context_user.is_empty() { c.original.clone() } else { context_user.clone() },
            });
            self.observe(s.id, "grammar", &c.corrected, detail).await;
            let label = tutor::mistake_label(c);
            if !s.mistakes.contains(&label) {
                s.mistakes.push(label);
            }
            if c.ask_repeat {
                s.phase = Phase::AwaitRepeat {
                    target: c.corrected.clone(),
                    attempts: 0,
                };
            }
        }
        for t in &out.unknown_terms {
            let kind = if t.text.contains(' ') {
                "unknown_phrase"
            } else {
                "unknown_word"
            };
            let detail = json!({ "explanation": t.explanation, "context": out.reply });
            self.observe(s.id, kind, &t.text, detail).await;
        }
        for p in &out.useful_phrases {
            self.observe(s.id, "useful_sentence", p, json!({ "context": out.reply }))
                .await;
        }
        if measured {
            self.record_timing(timing);
        }
        let repeat_target = match &s.phase {
            Phase::AwaitRepeat { target, .. } => Some(target.clone()),
            _ => None,
        };
        sink(VoiceEvent::TutorDone {
            turn,
            turn_id,
            text: out.reply.clone(),
            correction: out.correction.clone(),
            phase: s.phase.name().into(),
            repeat_target,
            drill: None,
            local: false,
        });
    }

    // ------------------------------------------------------------ pronunciation drill

    /// From the "How do I say X?" intent or the UI (tap a word → Practise saying).
    pub async fn start_drill(self: &Arc<Self>, word: String, sink: Sink) -> AppResult<()> {
        let mut guard = self.active.lock().await;
        let s = guard
            .as_mut()
            .ok_or_else(|| AppError::Invalid("No conversation is running".into()))?;
        let word = intents::clean(&word);
        if word.is_empty() {
            return Err(AppError::Invalid("Choose a word to practise".into()));
        }
        self.begin_drill(s, word, &sink).await;
        Ok(())
    }

    async fn begin_drill(&self, s: &mut Session, word: String, sink: &Sink) {
        let hint = self.drill_hint(&word).await;
        s.phase = Phase::Drill {
            word: word.clone(),
            hint,
            heard: Vec::new(),
        };
        self.local_reply(s, &[("Listen:", None, None), (&word, None, Some(DRILL_RATE))], sink)
            .await;
    }

    /// Syllables (and stress) for the drill card: the "Explain simply" cache, then the macOS
    /// dictionary, then one `define_term` call.
    async fn drill_hint(&self, word: &str) -> Option<String> {
        if let Some(d) = self.d.ai.cached_term(word)
            && let Some(syl) = d.syllables
        {
            return Some(syl);
        }
        let dict = self.d.dictionary.clone();
        let w = word.to_string();
        let entry = tokio::task::spawn_blocking(move || dict(&w)).await.ok().flatten();
        if let Some(e) = entry
            && (e.syllables.is_some() || e.pronunciation.is_some())
        {
            return Some(match (e.syllables, e.pronunciation) {
                (Some(s), Some(p)) => format!("{s}  /{p}/"),
                (Some(s), None) => s,
                (None, Some(p)) => format!("/{p}/"),
                (None, None) => unreachable!(),
            });
        }
        match self.d.ai.define_term(word, None, None, true).await {
            Ok(d) => d.syllables,
            Err(e) => {
                tracing::debug!(code = e.code(), "no drill hint");
                None
            }
        }
    }

    // ------------------------------------------------------------ review

    pub async fn review(&self, conversation_id: i64) -> AppResult<SessionReview> {
        if let Some(r) = self.reviews.lock().unwrap().get(&conversation_id) {
            return Ok(r.clone());
        }
        let (conv, obs, turns) = self
            .d
            .db
            .call(move |c| {
                Ok((
                    repo::get(c, conversation_id)?,
                    repo::observations(c, conversation_id)?,
                    repo::turns(c, conversation_id)?,
                ))
            })
            .await?;
        let level = conv.settings.get("level").and_then(Value::as_u64).unwrap_or(2) as u8;
        let mut source = "llm";
        let mut suggestions = Vec::new();
        if !obs.is_empty() && self.d.mode.get() == Mode::Standard {
            let mut ok = false;
            for attempt in 0..2 {
                let mut req = LlmRequest::new(review::prompt(level, conv.article_title.as_deref(), &obs, &turns), 900);
                req.temperature = 0.2;
                req.json_schema = Some(review::schema());
                match self
                    .d
                    .ai
                    .complete(req, true)
                    .await
                    .and_then(|t| review::parse(&t, &obs))
                {
                    Ok(s) => {
                        suggestions = s;
                        ok = true;
                        break;
                    }
                    Err(e) => tracing::warn!(code = e.code(), attempt, "session review failed"),
                }
            }
            if !ok {
                source = "fallback";
                suggestions = review::fallback(&obs);
            }
        } else {
            source = "fallback";
            suggestions = review::fallback(&obs);
        }
        let r = SessionReview {
            conversation_id,
            article_title: conv.article_title.clone(),
            started_at: conv.started_at.clone(),
            review_status: conv.review_status.clone(),
            stats: review::stats(&conv, &obs),
            suggestions,
            source: source.into(),
        };
        self.reviews.lock().unwrap().insert(conversation_id, r.clone());
        Ok(r)
    }

    /// Add the chosen suggestions to the Word Book (an empty choice = Skip).
    pub async fn apply_review(&self, conversation_id: i64, selected: Vec<Suggestion>) -> AppResult<usize> {
        let (conv, obs) = self
            .d
            .db
            .call(move |c| Ok((repo::get(c, conversation_id)?, repo::observations(c, conversation_id)?)))
            .await?;
        if selected.is_empty() {
            self.d
                .db
                .call(move |c| repo::set_review_status(c, conversation_id, "skipped"))
                .await?;
            self.reviews.lock().unwrap().remove(&conversation_id);
            return Ok(0);
        }
        let mut added = 0;
        for (sug, item) in review::to_new_items(&selected, &obs, conversation_id, conv.article_id) {
            let r = self.d.vocab.add(item).await?;
            added += 1;
            let (ids, item_id) = (sug.observation_ids.clone(), r.item.id);
            self.d
                .db
                .call(move |c| {
                    for i in ids {
                        repo::set_observation_saved(c, i, item_id)?;
                    }
                    Ok(())
                })
                .await?;
        }
        self.d
            .db
            .call(move |c| repo::set_review_status(c, conversation_id, "done"))
            .await?;
        self.reviews.lock().unwrap().remove(&conversation_id);
        tracing::info!(conversation = conversation_id, added, "session review applied");
        Ok(added)
    }

    // ------------------------------------------------------------ latency

    fn record_timing(&self, t: TurnTiming) {
        if let (Some(total), false) = (t.first_sentence_ms, t.typed) {
            tracing::info!(
                turn = t.turn,
                stt_ms = t.stt_ms,
                first_token_ms = t.first_token_ms,
                first_sentence_ms = total,
                llm_done_ms = t.llm_done_ms,
                "voice turn timing"
            );
        }
        let mut q = self.timings.lock().unwrap();
        q.push_back(t);
        while q.len() > LATENCY_KEEP {
            q.pop_front();
        }
    }

    /// The UI reports when the first sentence started playing.
    pub fn report_tts_start(&self, turn: u64, tts_start_ms: u64) {
        if let Some(t) = self.timings.lock().unwrap().iter_mut().find(|t| t.turn == turn) {
            t.tts_start_ms = Some(tts_start_ms);
            tracing::info!(turn, tts_start_ms, "voice turn: first audio");
        }
    }

    pub fn latency(&self) -> LatencyReport {
        let q = self.timings.lock().unwrap();
        let voice: Vec<&TurnTiming> = q.iter().filter(|t| !t.typed).collect();
        let pick = |f: fn(&TurnTiming) -> Option<u64>| percentiles(voice.iter().filter_map(|t| f(t)).collect());
        LatencyReport {
            total: pick(|t| t.tts_start_ms),
            stt: pick(|t| t.stt_ms),
            first_token: pick(|t| t.first_token_ms),
            first_sentence: pick(|t| t.first_sentence_ms),
            llm_done: pick(|t| t.llm_done_ms),
            recent: q.iter().rev().take(10).cloned().collect(),
        }
    }
}

enum TopicText {
    Article(String),
    Free(Vec<String>),
}

fn system_for(settings: &SessionSettings, title: Option<&str>, topic: &TopicText) -> String {
    let topic = match topic {
        TopicText::Article(summary) => Topic::Article {
            title: title.unwrap_or(""),
            summary,
        },
        TopicText::Free(t) => Topic::Free { topics: t },
    };
    tutor::system_prompt(settings.level, settings.correction, &topic)
}

#[cfg(test)]
mod tests;
