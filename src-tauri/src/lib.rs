pub mod ai;
pub mod clock;
pub mod commands;
pub mod db;
pub mod error;
pub mod events;
pub mod http;
pub mod learning;
pub mod logging;
pub mod mode;
pub mod news;
pub mod notify;
pub mod scheduler;
pub mod seed;
pub mod settings;
pub mod shell;
pub mod sidecar;
pub mod state;
pub mod voice;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{App, AppHandle, Listener, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use clock::{Clock, SystemClock};
use db::Db;
use events::{EventSink, TauriEventSink};
use http::{HttpClient, ReqwestClient};
use mode::ModeManager;
use news::NewsService;
use news::pick::PickService;
use notify::{NotifyService, TauriNotifier};
use scheduler::Scheduler;
use settings::{SettingsStore, WidgetStyle};
use state::AppState;

struct LogGuard(#[allow(dead_code)] Option<tracing_appender::non_blocking::WorkerGuard>);

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    // TECH_ENGLISH_DATA_DIR points the app at a throwaway data folder (testing, perf runs).
    let data_dir = match std::env::var_os("TECH_ENGLISH_DATA_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => app.path().app_data_dir()?,
    };
    std::fs::create_dir_all(&data_dir)?;
    let log_dir = app.path().app_log_dir()?;
    std::fs::create_dir_all(&log_dir)?;
    app.manage(LogGuard(logging::init(&log_dir)));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting Tech English");

    let db = Db::open(&data_dir.join("app.db"))?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let events: Arc<dyn EventSink> = Arc::new(TauriEventSink(handle.clone()));
    let http: Arc<dyn HttpClient> = Arc::new(ReqwestClient::new()?);

    let (settings, mode, news) = tauri::async_runtime::block_on(async {
        let settings = SettingsStore::load(db.clone()).await?;
        let mode = ModeManager::load(db.clone(), events.clone()).await?;
        let news = NewsService::new(
            db.clone(),
            http,
            clock.clone(),
            settings.clone(),
            events.clone(),
            mode.clone(),
            None,
        )
        .await?;
        Ok::<_, error::AppError>((settings, mode, news))
    })?;
    let notify = Arc::new(NotifyService::new(
        db.clone(),
        clock.clone(),
        settings.clone(),
        Arc::new(TauriNotifier(handle.clone())),
    ));
    let pick = Arc::new(PickService::new(
        db.clone(),
        clock.clone(),
        settings.clone(),
        events.clone(),
        news.clone(),
        notify.clone(),
    ));
    let current = settings.get();

    // P2: local AI (llama-server sidecar), loaded on demand.
    let ai_manager = ai::manager::AiManager::llm(
        db.clone(),
        data_dir.clone(),
        settings.clone(),
        mode.clone(),
        events.clone(),
        Arc::new(ai::manager::RealLauncher),
    );
    // P5: whisper-server, started with a voice session.
    let stt_manager = sidecar::SidecarManager::whisper(
        db.clone(),
        data_dir.clone(),
        settings.clone(),
        mode.clone(),
        events.clone(),
        Arc::new(sidecar::RealLauncher),
    );
    for m in [&ai_manager, &stt_manager] {
        tauri::async_runtime::block_on(m.cleanup_orphan());
        m.spawn_idle_watch();
        let m = m.clone();
        mode.on_change(move |mode| {
            if mode == mode::Mode::Hibernate {
                let m = m.clone();
                tauri::async_runtime::spawn(async move { m.shutdown().await });
            }
        });
    }
    let provider: Arc<dyn ai::provider::LlmProvider> =
        Arc::new(ai::provider::LocalLlamaProvider::new(ai_manager.clone()));
    let ai_service = ai::service::AiService::new(db.clone(), clock.clone(), settings.clone(), events.clone(), provider);
    // P4: Word Book and quizzes.
    let vocab = learning::vocab::VocabService::new(
        db.clone(),
        clock.clone(),
        settings.clone(),
        events.clone(),
        mode.clone(),
    );

    #[cfg(target_os = "macos")]
    if !current.show_dock_icon {
        app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    }

    app.manage(AppState {
        db: db.clone(),
        clock: clock.clone(),
        settings: settings.clone(),
        events,
        mode: mode.clone(),
        news: news.clone(),
        pick: pick.clone(),
        notify: notify.clone(),
        ai_manager: ai_manager.clone(),
        stt_manager: stt_manager.clone(),
        ai_service: ai_service.clone(),
        vocab: vocab.clone(),
        downloader: Arc::new(ai::models::Downloader::default()),
        quitting: AtomicBool::new(false),
        pending_route: Mutex::new(None),
    });

    shell::build_tray(&handle, mode.get())?;
    let h = handle.clone();
    mode.on_change(move |m| shell::sync_mode_items(&h, m));
    let h = handle.clone();
    let p = pick.clone();
    handle.listen_any(events::PICK_CHANGED, move |_| {
        let (h, p) = (h.clone(), p.clone());
        tauri::async_runtime::spawn(async move {
            if let Ok(today) = p.today().await {
                shell::sync_pick_item(&h, today.as_ref());
            }
        });
    });
    if let Ok(today) = tauri::async_runtime::block_on(pick.today()) {
        shell::sync_pick_item(&handle, today.as_ref());
    }

    if let Some(w) = app.get_webview_window(shell::WIDGET) {
        shell::place_widget(&w, &current.widget);
    }
    if current.widget.style != WidgetStyle::Hidden {
        shell::apply_widget_style(&handle, &current.widget);
    }
    if !current.onboarding_done {
        shell::show_main(&handle, "/onboarding");
    }

    Arc::new(Scheduler {
        db,
        clock,
        news,
        pick,
        notify,
        settings: settings.clone(),
        ai: Some(ai_service),
        vocab: Some(vocab),
    })
    .spawn();
    Ok(())
}

fn on_second_instance(app: &AppHandle) {
    let style = app.try_state::<AppState>().map(|s| s.settings.get().widget.style);
    match (style, app.get_webview_window(shell::WIDGET)) {
        (Some(WidgetStyle::Hidden) | None, _) | (_, None) => shell::show_main(app, "/today"),
        (_, Some(w)) => {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            on_second_instance(app)
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_opener::init())
        .setup(setup)
        .on_window_event(|window, event| match event {
            // The widget only hides; the main window really closes to free its memory.
            WindowEvent::CloseRequested { api, .. } if window.label() == shell::WIDGET => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Moved(pos) if window.label() == shell::WIDGET => {
                shell::on_widget_moved(window.app_handle(), *pos);
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::get_mode,
            commands::settings::set_mode,
            commands::settings::get_autostart,
            commands::settings::set_autostart,
            commands::news::list_topics,
            commands::news::upsert_topic,
            commands::news::delete_topic,
            commands::news::preview_topic_matches,
            commands::news::list_feeds,
            commands::news::upsert_feed,
            commands::news::delete_feed,
            commands::news::test_feed,
            commands::news::refresh_now,
            commands::news::list_articles,
            commands::news::get_article,
            commands::news::record_interaction,
            commands::news::set_saved,
            commands::news::open_article,
            commands::news::get_today_pick,
            commands::news::get_pick_preview,
            commands::news::get_today_lesson,
            commands::news::discover_feeds,
            commands::news::add_feed_from_example,
            commands::news::news_status,
            commands::ai::ai_overview,
            commands::ai::download_model,
            commands::ai::cancel_download,
            commands::ai::delete_model,
            commands::ai::set_active_model,
            commands::ai::start_ai,
            commands::ai::unload_ai,
            commands::ai::get_derivative,
            commands::ai::get_cached_derivative,
            commands::ai::list_article_chat,
            commands::ai::send_article_chat,
            commands::ai::clear_article_chat,
            commands::ai::cancel_job,
            commands::reader::get_reader_article,
            commands::reader::fetch_article_html,
            commands::reader::save_article_body,
            commands::words::dictionary_lookup,
            commands::words::define_term,
            commands::words::add_vocab_item,
            commands::words::update_vocab_item,
            commands::words::delete_vocab_item,
            commands::words::list_vocab,
            commands::words::get_vocab_item,
            commands::words::list_vocab_keys,
            commands::words::due_count,
            commands::words::export_vocab_csv,
            commands::words::start_quiz,
            commands::words::grade_quiz_item,
            commands::words::finish_quiz,
            commands::words::list_quiz_history,
            commands::shell::set_widget_style,
            commands::shell::show_main,
            commands::shell::take_pending_route,
            commands::shell::quit_app,
            commands::onboarding::get_onboarding_defaults,
            commands::onboarding::complete_onboarding,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Tech English");

    app.run(|app, event| match event {
        RunEvent::ExitRequested { api, .. } => {
            let quitting = app
                .try_state::<AppState>()
                .is_none_or(|s| s.quitting.load(Ordering::SeqCst));
            if !quitting {
                api.prevent_exit();
            }
        }
        // Never leave the AI or speech engine running after Quit.
        RunEvent::Exit => {
            if let Some(state) = app.try_state::<AppState>() {
                let (llm, stt) = (state.ai_manager.clone(), state.stt_manager.clone());
                tauri::async_runtime::block_on(async move {
                    let both = async { tokio::join!(llm.shutdown(), stt.shutdown()) };
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), both).await;
                });
            }
        }
        _ => {}
    });
}
