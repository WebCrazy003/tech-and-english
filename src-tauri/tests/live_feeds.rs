//! Live check of every default feed (P1-T16). Needs the internet, so it is ignored by default:
//!   cargo test --test live_feeds -- --ignored --nocapture

use std::sync::Arc;

use futures::{StreamExt, stream};
use tech_english_lib::http::ReqwestClient;
use tech_english_lib::news::{hackernews, rss, source::FetchCtx};
use tech_english_lib::seed;

#[tokio::test]
#[ignore]
async fn every_default_feed_works() {
    let http = Arc::new(ReqwestClient::new().unwrap());
    let feeds = seed::defaults().feeds;
    let results: Vec<(String, Result<String, String>)> = stream::iter(feeds)
        .map(|f| {
            let http = http.clone();
            async move {
                let r = if f.kind == "hn" {
                    let ctx = FetchCtx {
                        http,
                        etag: None,
                        last_modified: None,
                        skip_hn_ids: Arc::new(Default::default()),
                        hn_include_new: false,
                        hn_base: String::new(),
                    };
                    hackernews::fetch(&ctx, &f.url)
                        .await
                        .map(|r| format!("{} stories", r.items.len()))
                        .map_err(|e| e.to_string())
                } else {
                    rss::test_feed(http.as_ref(), &f.url)
                        .await
                        .map(|r| {
                            format!(
                                "{} items, newest {}",
                                r.item_count,
                                r.newest_published_at.unwrap_or_default()
                            )
                        })
                        .map_err(|e| e.to_string())
                };
                (f.name, r)
            }
        })
        .buffer_unordered(6)
        .collect()
        .await;
    let mut failed = Vec::new();
    for (name, r) in &results {
        match r {
            Ok(s) => println!("OK    {name:<40} {s}"),
            Err(e) => {
                println!("FAIL  {name:<40} {e}");
                failed.push(name.clone());
            }
        }
    }
    assert!(failed.is_empty(), "failing feeds: {failed:?}");
}

/// Debug one feed: FEED_URL=https://… cargo test --test live_feeds one_feed -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn one_feed() {
    let url = std::env::var("FEED_URL").expect("set FEED_URL");
    let http = ReqwestClient::new().unwrap();
    let t = std::time::Instant::now();
    let r = rss::test_feed(&http, &url).await;
    println!("{:?} after {:?}", r.map(|r| r.item_count), t.elapsed());
}

/// Full pipeline on real feeds: seed everything, fetch, rank, pick. Prints the top stories.
///   cargo test --test live_feeds full_pipeline -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn full_pipeline() {
    use serde_json::json;
    use tech_english_lib::clock::{Clock, SystemClock};
    use tech_english_lib::db::{Db, repo::articles};
    use tech_english_lib::events::RecordingEventSink;
    use tech_english_lib::mode::ModeManager;
    use tech_english_lib::news::{NewsService, pick::PickService};
    use tech_english_lib::notify::{NotifyService, RecordingNotifier};
    use tech_english_lib::settings::SettingsStore;

    let db = Db::open_in_memory().unwrap();
    let d = seed::defaults();
    let topics: Vec<String> = d.topics.iter().map(|t| t.name.clone()).collect();
    let feeds: Vec<String> = d.feeds.iter().map(|f| f.url.clone()).collect();
    db.call(move |c| seed::apply(c, &topics, &feeds, "2026-01-01T00:00:00Z"))
        .await
        .unwrap();

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let settings = SettingsStore::load(db.clone()).await.unwrap();
    settings
        .update(json!({ "onboardingDone": true, "pickTime": "00:00" }))
        .await
        .unwrap();
    let events = Arc::new(RecordingEventSink::default());
    let mode = ModeManager::load(db.clone(), events.clone()).await.unwrap();
    let news = NewsService::new(
        db.clone(),
        Arc::new(ReqwestClient::new().unwrap()),
        clock.clone(),
        settings.clone(),
        events.clone(),
        mode,
        None,
    )
    .await
    .unwrap();

    let t = std::time::Instant::now();
    let new = news.fetch_cycle(true).await.unwrap();
    println!("fetch cycle: {new} new articles in {:?}", t.elapsed());

    let notifier = Arc::new(RecordingNotifier::default());
    let notify = Arc::new(NotifyService::new(
        db.clone(),
        clock.clone(),
        settings.clone(),
        notifier.clone(),
    ));
    let pick = PickService::new(db.clone(), clock, settings, events, news.clone(), notify);
    let p = pick.ensure_today().await.unwrap().expect("a daily pick");
    println!(
        "\nPICK: {} [{}] — {}\n  why: {}",
        p.article.title,
        p.article.source_name,
        p.article.score.unwrap_or(0.0),
        p.why
    );

    let page = db
        .call(|c| articles::list_items(c, &Default::default(), None, 15))
        .await
        .unwrap();
    println!("\nTop 15:");
    for a in page.items {
        println!(
            "{:5.1}  {:<70.70}  {:<22.22} {}",
            a.score.unwrap_or(0.0),
            a.title,
            a.source_name,
            a.topics.join(", ")
        );
    }
    let total = db.call(|c| articles::count(c)).await.unwrap();
    println!("\ntotal articles stored: {total}");
    assert!(new > 50);
}
