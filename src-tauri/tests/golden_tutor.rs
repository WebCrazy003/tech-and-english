//! Golden suite for the tutor's corrections (P5 dev spec §14), on the real model. Ignored by
//! default:
//!   cargo test --release --test golden_tutor -- --ignored --nocapture golden_tutor
//! Pass: ≥ 90 % agreement on `expect_correction`, and no correction on the disfluent cases.
//! Set TE_MODEL=<catalog id> to pick the model.

use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use serde::Deserialize;
use tech_english_lib::ai::manager::{AiManager, RealLauncher};
use tech_english_lib::ai::provider::{ChatMsg, LlmProvider, LlmRequest, LocalLlamaProvider};
use tech_english_lib::db::Db;
use tech_english_lib::events::RecordingEventSink;
use tech_english_lib::mode::ModeManager;
use tech_english_lib::settings::SettingsStore;
use tech_english_lib::voice::reply_stream::ReplyExtractor;
use tech_english_lib::voice::tutor::{self, CorrectionPolicy, Topic};

const SUMMARY: &str = "Data teams now run small AI models on their laptops. The models help them write SQL, \
explain error messages and document pipelines. They are private and free to run, but slower than cloud \
models. Many teams use them together with tools like Airflow, dbt and Docker.";
const OPENING: &str = r#"{
  "reply": "This article is about data teams that run small AI models on their laptops. The models help with SQL and error messages. Do you use AI tools in your work?",
  "correction": null,
  "unknown_terms": [],
  "useful_phrases": ["run on their laptops"]
}"#;

#[derive(Deserialize)]
struct Case {
    input: String,
    policy: CorrectionPolicy,
    expect_correction: bool,
    #[serde(default)]
    expect_contains: Option<String>,
    #[serde(default)]
    disfluent: bool,
}

fn data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Application Support/com.techenglish.app")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn golden_tutor() {
    let cases: Vec<Case> = include_str!("golden/tutor_corrections.jsonl")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("valid case"))
        .collect();
    assert_eq!(cases.len(), 30);

    let db = Db::open_in_memory().unwrap();
    let settings = SettingsStore::load(db.clone()).await.unwrap();
    if let Ok(m) = std::env::var("TE_MODEL") {
        settings
            .update(serde_json::json!({ "ai": { "activeModel": m } }))
            .await
            .unwrap();
    }
    let events = Arc::new(RecordingEventSink::default());
    let mode = ModeManager::load(db.clone(), events.clone()).await.unwrap();
    let manager = AiManager::llm(db, data_dir(), settings, mode, events, Arc::new(RealLauncher));
    let provider = LocalLlamaProvider::new(manager.clone());
    manager.ensure_ready(true).await.unwrap();
    println!("model: {}", provider.model_id());

    let topic = Topic::Article {
        title: "Small AI models help data teams",
        summary: SUMMARY,
    };
    let (mut agree, mut disfluent_corrected, mut contains_ok, mut contains_n) = (0, 0, 0, 0);
    let mut first_sentence_ms = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let messages = vec![
            ChatMsg::system(tutor::system_prompt(2, c.policy, &topic)),
            tutor::user_message(tutor::OPENING_INSTRUCTION, "", &[]),
            ChatMsg::assistant(OPENING),
            tutor::user_message("", &c.input, &[]),
        ];
        let mut req = LlmRequest::new(messages, 450);
        req.temperature = tutor::TEMPERATURE;
        req.json_schema = Some(tutor::schema());
        let t0 = Instant::now();
        let mut stream = provider.stream(req, true).await.unwrap();
        let mut x = ReplyExtractor::new();
        let mut first = None;
        while let Some(chunk) = stream.next().await {
            if !x.feed(&chunk.unwrap()).is_empty() && first.is_none() {
                first = Some(t0.elapsed().as_millis() as u64);
            }
        }
        let (_, parsed) = x.finish();
        let mut out = parsed.expect("valid tutor JSON");
        tutor::reconcile(&mut out, &c.input);
        if let Some(ms) = first {
            first_sentence_ms.push(ms);
        }
        let got = out.correction.is_some();
        let ok = got == c.expect_correction;
        agree += usize::from(ok);
        if c.disfluent && got {
            disfluent_corrected += 1;
        }
        let mut contains = String::new();
        if let (Some(want), Some(corr)) = (&c.expect_contains, &out.correction) {
            contains_n += 1;
            let hit = corr.corrected.to_lowercase().contains(&want.to_lowercase());
            contains_ok += usize::from(hit);
            contains = format!(" contains {want:?}: {}", if hit { "yes" } else { "NO" });
        }
        println!(
            "{:>2} {} [{:?}] {:?} → correction={} (want {}){}\n     corrected: {:?}\n     reply: {}",
            i + 1,
            if ok { "✅" } else { "❌" },
            c.policy,
            c.input,
            got,
            c.expect_correction,
            contains,
            out.correction.as_ref().map(|c| c.corrected.as_str()),
            out.reply
        );
    }
    first_sentence_ms.sort_unstable();
    let pct = agree as f64 / cases.len() as f64 * 100.0;
    println!(
        "\nagreement {agree}/{} = {pct:.0} % · disfluent corrected: {disfluent_corrected} · \
         expected words found: {contains_ok}/{contains_n} · first sentence p50 {} ms",
        cases.len(),
        first_sentence_ms.get(first_sentence_ms.len() / 2).copied().unwrap_or(0)
    );
    manager.shutdown().await;
    assert!(pct >= 90.0, "agreement below 90 %");
    assert_eq!(disfluent_corrected, 0, "a disfluency was corrected");
}
