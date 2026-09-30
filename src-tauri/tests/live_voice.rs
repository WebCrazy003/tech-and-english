//! Live voice-tutor check with the real whisper-server and llama-server (P5 T14). The learner's
//! turns are clips made with macOS `say` (no microphone). Ignored by default:
//!   cargo test --test live_voice -- --ignored --nocapture
//! Prints per-turn timings and p50/p90. The UI adds the speech start (≈ 0.1–0.3 s) on top.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tech_english_lib::ai::manager::{AiManager, RealLauncher};
use tech_english_lib::ai::provider::LocalLlamaProvider;
use tech_english_lib::ai::service::AiService;
use tech_english_lib::clock::{Clock, SystemClock};
use tech_english_lib::db::Db;
use tech_english_lib::db::repo::ai::{Derivative, put_derivative};
use tech_english_lib::events::RecordingEventSink;
use tech_english_lib::learning::vocab::VocabService;
use tech_english_lib::mode::ModeManager;
use tech_english_lib::settings::SettingsStore;
use tech_english_lib::sidecar::SidecarManager;
use tech_english_lib::voice::capture::{Clip, rms};
use tech_english_lib::voice::session::{EndReason, SessionSettings, Sink, VoiceDeps, VoiceEngine, VoiceEvent};
use tech_english_lib::voice::stt::WhisperServerProvider;
use tech_english_lib::voice::tutor::CorrectionPolicy;

const SUMMARY: &str = "DuckDB 2.0 has a new storage format. Files are about 30 percent smaller, and many \
analytical queries run twice as fast. DuckDB runs inside your application, so there is no server. Data \
engineers use it to explore Parquet and CSV files on a laptop with normal SQL. The new version can also \
read Apache Iceberg tables from a lakehouse.";

const TURNS: &[&str] = &[
    "Yes, I use DuckDB at work to check CSV files before we load them.",
    "Yesterday I deploy a new version of our pipeline.",
    "Yesterday I deployed a new version of our pipeline.",
    "What does lakehouse mean?",
    "I think smaller files are good because the storage is expensive.",
    "Our team use Airflow for the daily jobs.",
    "Our team uses Airflow for the daily jobs.",
    "Speak slower please.",
    "We have a lot of Parquet files in S3.",
    "I didn't understood the part about Iceberg.",
    "I didn't understand the part about Iceberg.",
    "How do I say analytical?",
    "analytical",
    "Maybe we can use DuckDB for tests of our pipelines.",
    "It depends of the size of the data.",
    "It depends on the size of the data.",
    "I want to learn more about SQL window functions.",
    "My manager said me to write more tests.",
    "My manager told me to write more tests.",
    "Thank you, this was a good conversation.",
];

fn data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Application Support/com.techenglish.app")
}

fn clip_from_say(text: &str, dir: &std::path::Path, i: usize) -> Clip {
    let path = dir.join(format!("t{i}.wav"));
    let ok = std::process::Command::new("say")
        .args(["-r", "150", "-o"])
        .arg(&path)
        .args(["--data-format=LEI16@16000", text])
        .status()
        .unwrap()
        .success();
    assert!(ok, "say failed");
    let mut r = hound::WavReader::open(&path).unwrap();
    let samples: Vec<f32> = r
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / i16::MAX as f32)
        .collect();
    Clip {
        duration: Duration::from_secs_f64(samples.len() as f64 / 16_000.0),
        rms: rms(&samples),
        peak: samples.iter().fold(0.0f32, |m, s| m.max(s.abs())),
        samples_16k_mono: samples,
    }
}

fn pct(mut v: Vec<u64>, q: f64) -> u64 {
    v.sort_unstable();
    v[((v.len() as f64 * q).ceil() as usize).clamp(1, v.len()) - 1]
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn voice_session_on_real_engines() {
    let db = Db::open_in_memory().unwrap();
    let article = db
        .call(|c| {
            c.execute(
                "INSERT INTO articles(url, normalized_url, title, title_key, source_name, discovered_at)
                 VALUES ('https://duckdb.org/x','https://duckdb.org/x','DuckDB 2.0 brings a faster storage format','d','DuckDB Blog','2026-09-29T00:00:00Z')",
                [],
            )?;
            let id = c.last_insert_rowid();
            let d = Derivative {
                text: SUMMARY.into(),
                model_id: "x".into(),
                prompt_version: tech_english_lib::ai::prompts::PROMPT_VERSION.into(),
            };
            put_derivative(c, id, "summary_b1", &d, "2026-09-30T00:00:00Z")?;
            Ok(id)
        })
        .await
        .unwrap();
    let settings = SettingsStore::load(db.clone()).await.unwrap();
    // TE_LLAMA_SERVER / TE_WHISPER_SERVER=<path> test other engine binaries (the bundled sidecars, P6).
    for (var, key) in [
        ("TE_LLAMA_SERVER", "llamaServerPath"),
        ("TE_WHISPER_SERVER", "whisperServerPath"),
    ] {
        if let Ok(p) = std::env::var(var) {
            settings.update(serde_json::json!({ "ai": { key: p } })).await.unwrap();
        }
    }
    let events = Arc::new(RecordingEventSink::default());
    let mode = ModeManager::load(db.clone(), events.clone()).await.unwrap();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let launcher = Arc::new(RealLauncher);
    let llm = AiManager::llm(
        db.clone(),
        data_dir(),
        settings.clone(),
        mode.clone(),
        events.clone(),
        launcher.clone(),
    );
    let stt = SidecarManager::whisper(
        db.clone(),
        data_dir(),
        settings.clone(),
        mode.clone(),
        events.clone(),
        launcher,
    );
    let ai = AiService::new(
        db.clone(),
        clock.clone(),
        settings.clone(),
        events.clone(),
        Arc::new(LocalLlamaProvider::new(llm.clone())),
    );
    let vocab = VocabService::new(
        db.clone(),
        clock.clone(),
        settings.clone(),
        events.clone(),
        mode.clone(),
    );
    let engine = VoiceEngine::new(VoiceDeps {
        db: db.clone(),
        clock,
        settings,
        events,
        mode,
        ai,
        stt: Arc::new(WhisperServerProvider::new(stt.clone())),
        vocab,
        llm_manager: Some(llm.clone()),
        stt_manager: Some(stt.clone()),
        data_dir: std::env::temp_dir(),
        dictionary: Arc::new(tech_english_lib::learning::dictionary::lookup),
    });

    let log = Arc::new(Mutex::new(Vec::<VoiceEvent>::new()));
    let l2 = log.clone();
    let sink: Sink = Arc::new(move |e| l2.lock().unwrap().push(e));
    let t0 = Instant::now();
    let s = SessionSettings {
        level: 2,
        rate: 0.85,
        pause_ms: 400,
        correction: CorrectionPolicy::High,
        voice_uri: None,
    };
    engine.start(Some(article), s, true, sink.clone()).await.unwrap();
    println!("session ready + opening turn: {:.1} s", t0.elapsed().as_secs_f64());
    print_new(&log);

    let dir = tempfile::tempdir().unwrap();
    for (i, text) in TURNS.iter().enumerate() {
        let clip = clip_from_say(text, dir.path(), i);
        println!("\n▶ learner ({:.1} s audio): {text}", clip.duration.as_secs_f64());
        engine.handle_clip(clip, Instant::now(), sink.clone()).await.unwrap();
        print_new(&log);
    }

    let report = engine.latency();
    let firsts: Vec<u64> = report.recent.iter().filter_map(|t| t.first_sentence_ms).collect();
    println!(
        "\n=== timings over {} spoken turns (ms after end of speech) ===",
        report.stt.count
    );
    for (name, p) in [
        ("stt", &report.stt),
        ("first token", &report.first_token),
        ("first sentence", &report.first_sentence),
        ("whole answer", &report.llm_done),
    ] {
        println!("{name:>15}: p50 {:?}  p90 {:?}  (n={})", p.p50, p.p90, p.count);
    }
    if !firsts.is_empty() {
        println!(
            "last 10 first-sentence: p50 {} p90 {}",
            pct(firsts.clone(), 0.5),
            pct(firsts, 0.9)
        );
    }
    let review = engine.end(EndReason::User, true).await.unwrap().unwrap();
    println!(
        "\nreview ({}): {:?}",
        review.source,
        review
            .suggestions
            .iter()
            .map(|s| format!("{}{} {}", if s.preselected { "☑" } else { "☐" }, s.kind, s.text))
            .collect::<Vec<_>>()
    );
    println!("stats: {:?}", review.stats);
    llm.shutdown().await;
    stt.shutdown().await;
}

fn print_new(log: &Mutex<Vec<VoiceEvent>>) {
    for e in log.lock().unwrap().drain(..) {
        match e {
            VoiceEvent::Transcript { text } => println!("  heard: {text}"),
            VoiceEvent::Notice { text } => println!("  notice: {text}"),
            VoiceEvent::TutorDone {
                text,
                correction,
                phase,
                local,
                ..
            } => println!(
                "  tutor{}: {text}\n    phase={phase}{}",
                if local { " (local)" } else { "" },
                correction
                    .map(|c| format!(" correction: {} → {}", c.original, c.corrected))
                    .unwrap_or_default()
            ),
            VoiceEvent::LocalAction { action, rate } => println!("  local action: {action} {rate:?}"),
            VoiceEvent::Error { code, message } => println!("  ERROR {code}: {message}"),
            _ => {}
        }
    }
}
