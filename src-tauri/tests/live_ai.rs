//! Live AI check with the real llama-server and a downloaded model (P2). Needs the engine and a
//! model in `~/Library/Application Support/com.techenglish.app`. Ignored by default:
//!   cargo test --test live_ai -- --ignored --nocapture
//! Set TE_MODEL=<catalog id> to pick the model.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use tech_english_lib::ai::manager::{AiManager, RealLauncher};
use tech_english_lib::ai::prompts::QuickAction;
use tech_english_lib::ai::provider::{LlmProvider, LocalLlamaProvider};
use tech_english_lib::ai::service::{AiService, DerivKind, Sink, StreamEvent};
use tech_english_lib::clock::{Clock, SystemClock};
use tech_english_lib::db::Db;
use tech_english_lib::events::RecordingEventSink;
use tech_english_lib::mode::ModeManager;
use tech_english_lib::settings::SettingsStore;
use tokio_util::sync::CancellationToken;

const ARTICLE: &str = "DuckDB is an in-process analytical database. It runs inside your application, so there is no server to install or manage.\n\n\
Many data engineers use DuckDB to explore Parquet and CSV files on a laptop. They write normal SQL, and DuckDB reads the files directly, without loading them into a separate system first.\n\n\
The new storage format in version 2.0 compresses columns better. Files are about 30 percent smaller, and many analytical queries run twice as fast, because less data has to be read from disk.\n\n\
The team also added better support for Apache Iceberg tables. This means DuckDB can read data from a lakehouse directly. A lakehouse combines cheap file storage, like a data lake, with table features, like a data warehouse.\n\n\
The release keeps the single-file design. You can copy one database file to another computer, and it works the same way. The authors say this makes DuckDB useful for teaching, for testing pipelines, and for small production workloads.";

fn timing_sink(label: &'static str) -> (Sink, Arc<Mutex<Option<f64>>>) {
    let t0 = Instant::now();
    let first = Arc::new(Mutex::new(None));
    let f2 = first.clone();
    (
        Arc::new(move |e| {
            if let StreamEvent::Delta { .. } = e {
                let mut f = f2.lock().unwrap();
                if f.is_none() {
                    *f = Some(t0.elapsed().as_secs_f64());
                    println!("[{label}] first text after {:.2}s", t0.elapsed().as_secs_f64());
                }
            }
            if let StreamEvent::Error { message, .. } = &e {
                println!("[{label}] ERROR {message}");
            }
        }),
        first,
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn summary_and_chat_on_real_model() {
    let data_dir = dirs_data();
    let db = Db::open_in_memory().unwrap();
    let id = db
        .call(|c| {
            c.execute(
                "INSERT INTO articles(url, normalized_url, title, title_key, source_name, discovered_at, body_text, body_status)
                 VALUES ('https://duckdb.org/x','https://duckdb.org/x','DuckDB 2.0 brings a faster storage format','duckdb','DuckDB Blog','2026-09-29T00:00:00Z',?1,'ok')",
                [ARTICLE],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await
        .unwrap();
    let settings = SettingsStore::load(db.clone()).await.unwrap();
    if let Ok(m) = std::env::var("TE_MODEL") {
        settings
            .update(serde_json::json!({ "ai": { "activeModel": m } }))
            .await
            .unwrap();
    }
    let events = Arc::new(RecordingEventSink::default());
    let mode = ModeManager::load(db.clone(), events.clone()).await.unwrap();
    let manager = AiManager::new(
        db.clone(),
        data_dir,
        settings.clone(),
        mode,
        events.clone(),
        Arc::new(RealLauncher),
    );
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let provider = Arc::new(LocalLlamaProvider::new(manager.clone()));
    let svc = AiService::new(db.clone(), clock, settings, events, provider.clone());

    let t = Instant::now();
    manager.ensure_ready(true).await.unwrap();
    println!(
        "model {} ready after {:.1}s",
        provider.model_id(),
        t.elapsed().as_secs_f64()
    );

    let tok = CancellationToken::new();
    let (sink, _) = timing_sink("summary");
    let t = Instant::now();
    let summary = svc
        .derivative(id, DerivKind::SummaryB1, false, false, &tok, &sink)
        .await
        .unwrap()
        .unwrap();
    println!(
        "[summary] done in {:.1}s, {} words\n{summary}\n",
        t.elapsed().as_secs_f64(),
        summary.split_whitespace().count()
    );

    for (q, action) in [
        ("Why are the files smaller?", None),
        ("Who is the CEO of DuckDB?", None),
        ("", Some(QuickAction::KeyWords)),
    ] {
        let text = if q.is_empty() {
            action.unwrap().message().to_string()
        } else {
            q.to_string()
        };
        svc.add_user_message(id, text.clone()).await.unwrap();
        let (sink, _) = timing_sink("chat");
        let t = Instant::now();
        svc.answer_chat(id, action, false, &tok, &sink).await.unwrap();
        let msgs = db
            .call(move |c| tech_english_lib::db::repo::ai::list_chat(c, id))
            .await
            .unwrap();
        println!(
            "[chat] {:.1}s  Q: {text}\n  A: {}\n",
            t.elapsed().as_secs_f64(),
            msgs.last().unwrap().content
        );
    }
    manager.shutdown().await;
}

fn dirs_data() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Application Support/com.techenglish.app")
}
