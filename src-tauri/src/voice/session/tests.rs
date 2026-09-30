use super::*;
use crate::ai::provider::MockProvider;
use crate::clock::FakeClock;
use crate::events::RecordingEventSink;
use crate::voice::stt::FakeStt;

struct T {
    engine: Arc<VoiceEngine>,
    mock: Arc<MockProvider>,
    stt: Arc<FakeStt>,
    db: Db,
    mode: Arc<ModeManager>,
    events: Arc<Mutex<Vec<VoiceEvent>>>,
    sink: Sink,
}

fn turn_json(reply: &str, correction: Option<(&str, &str)>, terms: &[(&str, &str)], phrases: &[&str]) -> String {
    let correction = correction.map(
        |(o, c)| json!({ "original": o, "corrected": c, "explanation": "Use the past tense.", "ask_repeat": true }),
    );
    let terms: Vec<Value> = terms
        .iter()
        .map(|(t, e)| json!({ "text": t, "explanation": e }))
        .collect();
    serde_json::to_string_pretty(&json!({
        "reply": reply, "correction": correction, "unknown_terms": terms, "useful_phrases": phrases
    }))
    .unwrap()
}

/// Split into two chunks so the reply streams.
fn chunks(s: &str) -> Vec<String> {
    let mid = s.char_indices().nth(s.chars().count() / 2).unwrap().0;
    vec![s[..mid].to_string(), s[mid..].to_string()]
}

async fn setup(answers: Vec<String>, transcripts: &[&str]) -> T {
    let db = Db::open_in_memory().unwrap();
    let clock: Arc<dyn Clock> = Arc::new(FakeClock::at("2026-09-30T08:00:00Z"));
    let settings = SettingsStore::load(db.clone()).await.unwrap();
    let sink_events = Arc::new(RecordingEventSink::default());
    let mode = ModeManager::load(db.clone(), sink_events.clone()).await.unwrap();
    let mock = Arc::new(MockProvider::default());
    mock.ready.store(true, Ordering::SeqCst);
    for a in answers {
        mock.answers.lock().unwrap().push_back(chunks(&a));
    }
    let ai = AiService::new(
        db.clone(),
        clock.clone(),
        settings.clone(),
        sink_events.clone(),
        mock.clone(),
    );
    let vocab = VocabService::new(
        db.clone(),
        clock.clone(),
        settings.clone(),
        sink_events.clone(),
        mode.clone(),
    );
    let stt = Arc::new(FakeStt::with(transcripts));
    let engine = VoiceEngine::new(VoiceDeps {
        db: db.clone(),
        clock,
        settings,
        events: sink_events,
        mode: mode.clone(),
        ai,
        stt: stt.clone(),
        vocab,
        llm_manager: None,
        stt_manager: None,
        data_dir: std::env::temp_dir(),
        dictionary: Arc::new(|w: &str| {
            (w == "scalability").then(|| DictEntry {
                headword: "scalability".into(),
                syllables: Some("scal·a·bil·i·ty".into()),
                pronunciation: Some("ˌskeɪləˈbɪlɪti".into()),
                part_of_speech: Some("noun".into()),
                senses: vec![],
                examples: vec![],
                parsed: true,
            })
        }),
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let e2 = events.clone();
    let sink: Sink = Arc::new(move |e| e2.lock().unwrap().push(e));
    T {
        engine,
        mock,
        stt,
        db,
        mode,
        events,
        sink,
    }
}

fn settings() -> SessionSettings {
    SessionSettings {
        level: 2,
        rate: 0.85,
        pause_ms: 400,
        correction: CorrectionPolicy::High,
        voice_uri: None,
    }
}

impl T {
    async fn start(&self) -> i64 {
        self.engine
            .start(None, settings(), false, self.sink.clone())
            .await
            .unwrap()
    }
    async fn say(&self, text: &str) {
        self.engine.send_text(text.into(), self.sink.clone()).await.unwrap();
    }
    fn take(&self) -> Vec<VoiceEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
    fn sentences(ev: &[VoiceEvent]) -> Vec<String> {
        ev.iter()
            .filter_map(|e| match e {
                VoiceEvent::TutorSentence { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }
    fn done(ev: &[VoiceEvent]) -> (String, Option<String>, bool) {
        ev.iter()
            .rev()
            .find_map(|e| match e {
                VoiceEvent::TutorDone {
                    phase,
                    repeat_target,
                    local,
                    ..
                } => Some((phase.clone(), repeat_target.clone(), *local)),
                _ => None,
            })
            .expect("a tutorDone event")
    }
    async fn observations(&self, conv: i64) -> Vec<repo::Observation> {
        self.db.call(move |c| repo::observations(c, conv)).await.unwrap()
    }
}

const OPENING: &str = "Hi! Local AI models run on laptops now. Do you use one?";

#[tokio::test]
async fn opening_turn_streams_sentences() {
    let t = setup(vec![turn_json(OPENING, None, &[], &[])], &[]).await;
    let conv = t.start().await;
    let ev = t.take();
    assert!(matches!(ev[0], VoiceEvent::Ready { conversation_id, .. } if conversation_id == conv));
    assert_eq!(
        T::sentences(&ev),
        vec!["Hi!", "Local AI models run on laptops now.", "Do you use one?"]
    );
    assert_eq!(T::done(&ev).0, "discuss");
    let req = &t.mock.requests.lock().unwrap()[0];
    assert!(req.json_schema.is_some());
    assert_eq!(req.messages[0].role, "system");
    assert!(req.messages[1].content.contains("Start the conversation"));
    assert!(t.engine.is_active());
}

#[tokio::test]
async fn correction_then_repeat_then_continue() {
    let t = setup(
        vec![
            turn_json(OPENING, None, &[], &[]),
            turn_json(
                "Good idea! We say deployed for the past. Please say: Yesterday I deployed the application.",
                Some((
                    "Yesterday I deploy the application.",
                    "Yesterday I deployed the application.",
                )),
                &[],
                &[],
            ),
            turn_json("Great. What did the app do?", None, &[], &[]),
        ],
        &[],
    )
    .await;
    let conv = t.start().await;
    t.take();
    t.say("Yesterday I deploy the application.").await;
    let ev = t.take();
    let (phase, target, _) = T::done(&ev);
    assert_eq!(phase, "awaitRepeat");
    assert_eq!(target.as_deref(), Some("Yesterday I deployed the application."));
    let obs = t.observations(conv).await;
    assert_eq!(obs[0].kind, "grammar");
    assert_eq!(
        obs[0].detail.as_ref().unwrap()["original"],
        "Yesterday I deploy the application."
    );

    // A correct repeat: local "Good. Let's continue." then one LLM turn.
    t.say("yesterday I deployed the application").await;
    let ev = t.take();
    let s = T::sentences(&ev);
    assert_eq!(s[0], "Good. Let's continue.");
    assert!(s.contains(&"What did the app do?".to_string()));
    assert_eq!(T::done(&ev).0, "discuss");
    assert_eq!(t.mock.calls(), 3);
    let last = t.mock.requests.lock().unwrap()[2]
        .messages
        .last()
        .unwrap()
        .content
        .clone();
    assert!(last.contains("repeated correctly") && last.contains("deploy → deployed"));
}

#[tokio::test]
async fn failing_the_repeat_twice_moves_on() {
    let t = setup(
        vec![
            turn_json(OPENING, None, &[], &[]),
            turn_json(
                "We say for. Please say: I have used Python for three years.",
                Some((
                    "I have used Python since three years.",
                    "I have used Python for three years.",
                )),
                &[],
                &[],
            ),
            turn_json("OK. Which Python library do you like?", None, &[], &[]),
        ],
        &[],
    )
    .await;
    t.start().await;
    t.say("I have used Python since three years.").await;
    t.take();
    t.say("I use Python three years").await;
    let ev = t.take();
    assert_eq!(
        T::sentences(&ev),
        vec!["Almost. Listen again:", "I have used Python for three years."]
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, VoiceEvent::TutorSentence { rate_delta: Some(d), .. } if (*d + 0.15).abs() < 1e-9))
    );
    assert_eq!(T::done(&ev).0, "awaitRepeat");
    assert_eq!(t.mock.calls(), 2, "no LLM call for the first failure");
    t.say("I used Python three year").await;
    let ev = t.take();
    assert_eq!(T::sentences(&ev)[0], "Good try. Let's continue.");
    assert_eq!(T::done(&ev).0, "discuss");
    assert_eq!(t.mock.calls(), 3);
}

#[tokio::test]
async fn missing_please_say_is_added() {
    let t = setup(
        vec![
            turn_json(OPENING, None, &[], &[]),
            turn_json(
                "We say doesn't. Does he like it?",
                Some(("He don't use Docker.", "He doesn't use Docker.")),
                &[],
                &[],
            ),
        ],
        &[],
    )
    .await;
    t.start().await;
    t.say("He don't use Docker.").await;
    let ev = t.take();
    assert_eq!(T::sentences(&ev).last().unwrap(), "Please say: He doesn't use Docker.");
    assert_eq!(T::done(&ev).0, "awaitRepeat");
}

#[tokio::test]
async fn local_intents_make_no_llm_call() {
    let t = setup(vec![turn_json(OPENING, None, &[], &[])], &[]).await;
    t.start().await;
    t.take();
    t.say("Speak slower, please.").await;
    t.say("Could you repeat that?").await;
    t.say("Faster").await;
    t.say("End the conversation.").await;
    let actions: Vec<(String, Option<f64>)> = t
        .take()
        .into_iter()
        .filter_map(|e| match e {
            VoiceEvent::LocalAction { action, rate } => Some((action, rate)),
            _ => None,
        })
        .collect();
    assert_eq!(
        actions,
        vec![
            ("slower".into(), Some(0.75)),
            ("replay".into(), None),
            ("faster".into(), Some(0.85)),
            ("end".into(), None)
        ]
    );
    assert_eq!(t.mock.calls(), 1, "only the opening turn");
    assert_eq!(t.engine.active().unwrap().settings.rate, 0.85);
}

#[tokio::test]
async fn define_intent_logs_and_asks_the_llm() {
    let t = setup(
        vec![
            turn_json(OPENING, None, &[], &[]),
            turn_json(
                "Deployment means putting software where people can use it. So, do you use one?",
                None,
                &[("deployment", "putting software where people can use it")],
                &["putting software live"],
            ),
        ],
        &[],
    )
    .await;
    let conv = t.start().await;
    t.say("What does deployment mean?").await;
    assert_eq!(t.mock.calls(), 2);
    let obs = t.observations(conv).await;
    let kinds: Vec<(&str, &str)> = obs.iter().map(|o| (o.kind.as_str(), o.text.as_str())).collect();
    assert_eq!(
        kinds,
        vec![
            ("unknown_word", "deployment"),
            ("unknown_word", "deployment"),
            ("useful_sentence", "putting software live")
        ]
    );
    assert_eq!(obs[0].detail.as_ref().unwrap()["context"], "What does deployment mean?");
}

#[tokio::test]
async fn empty_transcript_is_a_notice_without_llm() {
    let t = setup(
        vec![turn_json(OPENING, None, &[], &[])],
        &["[BLANK_AUDIO]", "Thank you."],
    )
    .await;
    t.start().await;
    t.take();
    let loud = Clip {
        samples_16k_mono: vec![0.2; 16_000],
        duration: Duration::from_secs(1),
        rms: 0.2,
        peak: 0.2,
    };
    t.engine
        .handle_clip(loud.clone(), Instant::now(), t.sink.clone())
        .await
        .unwrap();
    // "Thank you." on a quiet clip is a known whisper hallucination.
    let quiet = Clip {
        rms: 0.005,
        peak: 0.02,
        ..loud.clone()
    };
    t.engine
        .handle_clip(quiet, Instant::now(), t.sink.clone())
        .await
        .unwrap();
    // Too quiet to be speech: no STT call at all.
    let silent = Clip {
        rms: 0.001,
        peak: 0.003,
        ..loud.clone()
    };
    t.engine
        .handle_clip(silent, Instant::now(), t.sink.clone())
        .await
        .unwrap();
    let digital_zero = Clip {
        rms: 0.0,
        peak: 0.0,
        ..loud
    };
    t.engine
        .handle_clip(digital_zero, Instant::now(), t.sink.clone())
        .await
        .unwrap();
    let notices: Vec<String> = t
        .take()
        .into_iter()
        .filter_map(|e| match e {
            VoiceEvent::Notice { text } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(notices, vec![DIDNT_HEAR, DIDNT_HEAR, DIDNT_HEAR, NO_SIGNAL]);
    assert_eq!(t.stt.calls.load(Ordering::SeqCst), 2);
    assert_eq!(t.mock.calls(), 1);
}

#[tokio::test]
async fn voice_turns_count_speaking_time() {
    let t = setup(
        vec![
            turn_json(OPENING, None, &[], &[]),
            turn_json("Nice. Why?", None, &[], &[]),
        ],
        &["I use Ollama at work."],
    )
    .await;
    let conv = t.start().await;
    let clip = Clip {
        samples_16k_mono: vec![0.1; 16_000 * 3],
        duration: Duration::from_secs(3),
        rms: 0.1,
        peak: 0.3,
    };
    t.engine
        .handle_clip(clip, Instant::now(), t.sink.clone())
        .await
        .unwrap();
    assert!(
        t.take()
            .iter()
            .any(|e| matches!(e, VoiceEvent::Transcript { text } if text == "I use Ollama at work."))
    );
    t.engine.end(EndReason::Hibernate, true).await.unwrap();
    let c = t.db.call(move |c| repo::get(c, conv)).await.unwrap();
    assert_eq!((c.user_speaking_seconds, c.review_status.as_str()), (3, "pending"));
    assert!(c.ended_at.is_some());
    assert!(!t.engine.is_active());
    let l = t.engine.latency();
    assert_eq!(l.stt.count, 1, "one voice turn measured (the opening is not)");
}

#[tokio::test]
async fn drill_passes_fails_and_gives_up() {
    let t = setup(
        vec![
            turn_json(OPENING, None, &[], &[]),
            turn_json("Well done. Do you design for scale?", None, &[], &[]),
            turn_json("OK. Let's go on. What else?", None, &[], &[]),
        ],
        &[],
    )
    .await;
    let conv = t.start().await;
    t.take();
    t.say("How do I say scalability?").await;
    let ev = t.take();
    assert_eq!(T::sentences(&ev), vec!["Listen:", "scalability"]);
    assert!(
        ev.iter()
            .any(|e| matches!(e, VoiceEvent::TutorSentence { rate: Some(r), .. } if (*r - 0.6).abs() < 1e-9))
    );
    let drill = ev
        .iter()
        .find_map(|e| match e {
            VoiceEvent::TutorDone { drill, .. } => drill.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(drill.hint.as_deref(), Some("scal·a·bil·i·ty  /ˌskeɪləˈbɪlɪti/"));
    assert_eq!(drill.attempt, 1);
    t.say("stability").await;
    let ev = t.take();
    assert_eq!(T::sentences(&ev)[0], "I heard \"stability\". Listen again:");
    assert_eq!(t.mock.calls(), 1);
    t.say("Scalability.").await;
    let ev = t.take();
    assert_eq!(T::sentences(&ev)[0], "Good!");
    assert_eq!(T::done(&ev).0, "discuss");
    assert_eq!(t.mock.calls(), 2);

    // Three misses → move on.
    t.engine
        .start_drill("scalability".into(), t.sink.clone())
        .await
        .unwrap();
    for heard in ["stability", "sale ability", "capability"] {
        t.say(heard).await;
    }
    let ev = t.take();
    assert!(T::sentences(&ev).contains(&"Good try. Let's continue.".to_string()));
    assert_eq!(t.mock.calls(), 3);
    let obs = t.observations(conv).await;
    let drills: Vec<&repo::Observation> = obs.iter().filter(|o| o.kind == "pronunciation").collect();
    assert_eq!(drills.len(), 2);
    assert_eq!(drills[0].detail.as_ref().unwrap()["passed"], true);
    assert_eq!(
        drills[1].detail.as_ref().unwrap()["attempts"].as_array().unwrap().len(),
        3
    );
}

#[tokio::test]
async fn llm_error_keeps_the_phase() {
    let t = setup(vec![turn_json(OPENING, None, &[], &[])], &[]).await;
    t.start().await;
    t.take();
    t.say("I like it.").await; // no scripted answer → error
    let ev = t.take();
    assert_eq!(T::sentences(&ev), vec![SORRY]);
    assert_eq!(T::done(&ev).0, "discuss");
    assert!(t.engine.is_active());
}

#[tokio::test]
async fn history_is_trimmed_in_one_step() {
    let answers: Vec<String> = (0..16)
        .map(|i| turn_json(&format!("Answer {i}. Next?"), None, &[], &[]))
        .collect();
    let t = setup(answers, &[]).await;
    t.start().await;
    for i in 0..12 {
        t.say(&format!("Message number {i}")).await;
    }
    let n = |t: &T, i: usize| t.mock.requests.lock().unwrap()[i].messages.len();
    // system + 12 exchanges (24 messages) + the new user message
    assert_eq!(n(&t, 12), 1 + 24 + 1);
    t.say("One more").await;
    assert_eq!(n(&t, 13), 1 + 6 + 1, "trimmed to the last 3 exchanges");
    t.say("And more").await;
    assert_eq!(n(&t, 14), 1 + 8 + 1);
}

#[tokio::test]
async fn review_fallback_and_apply() {
    let t = setup(
        vec![
            turn_json(OPENING, None, &[], &[]),
            turn_json(
                "We say deployed. Please say: Yesterday I deployed the app.",
                Some(("Yesterday I deploy the app.", "Yesterday I deployed the app.")),
                &[("latency", "the waiting time")],
                &["It depends on"],
            ),
            // The review answers are not valid twice → fallback.
            "not json".into(),
            "{\"suggestions\": []}".into(),
        ],
        &[],
    )
    .await;
    let conv = t.start().await;
    t.say("Yesterday I deploy the app.").await;
    let review = t.engine.end(EndReason::User, true).await.unwrap().unwrap();
    assert_eq!(review.source, "fallback");
    let got: Vec<(&str, bool)> = review
        .suggestions
        .iter()
        .map(|s| (s.kind.as_str(), s.preselected))
        .collect();
    assert_eq!(got, vec![("correction", true), ("word", true), ("sentence", false)]);
    assert_eq!(review.stats.corrections, 1);
    // Only the selected items become Word Book items, linked to the conversation.
    let selected: Vec<Suggestion> = review.suggestions.iter().filter(|s| s.preselected).cloned().collect();
    assert_eq!(t.engine.apply_review(conv, selected).await.unwrap(), 2);
    let (items, contexts, saved, status) =
        t.db.call(move |c| {
            let items: Vec<(String, String)> = c
                .prepare("SELECT kind, text FROM vocab_items ORDER BY id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            let contexts: i64 = c.query_row(
                "SELECT COUNT(*) FROM vocab_contexts WHERE conversation_id = ?1",
                [conv],
                |r| r.get(0),
            )?;
            let saved: i64 = c.query_row(
                "SELECT COUNT(*) FROM learning_observations WHERE saved_item_id IS NOT NULL",
                [],
                |r| r.get(0),
            )?;
            Ok((items, contexts, saved, repo::get(c, conv)?.review_status))
        })
        .await
        .unwrap();
    assert_eq!(
        items,
        vec![
            ("correction".to_string(), "Yesterday I deployed the app.".to_string()),
            ("word".to_string(), "latency".to_string())
        ]
    );
    assert_eq!((contexts, saved, status.as_str()), (2, 2, "done"));
}

#[tokio::test]
async fn skip_saves_nothing_and_hibernate_blocks_start() {
    let t = setup(vec![turn_json(OPENING, None, &[], &[])], &[]).await;
    let conv = t.start().await;
    t.engine.end(EndReason::User, false).await.unwrap();
    assert_eq!(t.engine.apply_review(conv, vec![]).await.unwrap(), 0);
    let status = t.db.call(move |c| repo::get(c, conv)).await.unwrap().review_status;
    assert_eq!(status, "skipped");
    t.mode.set(Mode::Hibernate).await.unwrap();
    let e = t
        .engine
        .start(None, settings(), false, t.sink.clone())
        .await
        .unwrap_err();
    assert!(matches!(e, AppError::Hibernating));
}

#[test]
fn percentiles_are_nearest_rank() {
    let p = percentiles(vec![5, 1, 4, 2, 3, 10, 9, 8, 7, 6]);
    assert_eq!((p.count, p.p50, p.p90), (10, Some(5), Some(9)));
    assert_eq!(percentiles(vec![]).p50, None);
}
