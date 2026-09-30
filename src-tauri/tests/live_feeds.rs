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
    db.call(move |c| {
        seed::apply(c, &topics, &feeds, "2026-01-01T00:00:00Z")?;
        // P3: lessons for Data Engineering (or LEARN_TOPICS="Data Engineering,Python").
        let learn = std::env::var("LEARN_TOPICS").unwrap_or_else(|_| "Data Engineering".into());
        for name in learn.split(',') {
            c.execute("UPDATE topics SET learn = 1 WHERE name = ?1", [name.trim()])?;
        }
        Ok(())
    })
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
    if let Some(l) = pick.today_lesson().await.unwrap() {
        println!(
            "\nLESSON: {} [{}]\n  why: {}",
            l.article.title, l.article.source_name, l.why
        );
    }
    let lessons = db
        .call(|c| {
            let rows = c
                .prepare(
                    "SELECT a.lesson_score, a.learning_score, a.title || '  «' || substr(COALESCE(a.description, ''), 1, 90) || '»',
                       a.source_name FROM articles a
                     WHERE a.id IN (SELECT id FROM articles WHERE lesson_score IS NOT NULL)
                       AND EXISTS (SELECT 1 FROM article_topics at JOIN topics t ON t.id = at.topic_id
                                   WHERE at.article_id = a.id AND t.learn = 1 AND at.relevance >= 0.3)
                     ORDER BY a.lesson_score DESC LIMIT 15",
                )?
                .query_map([], |r| {
                    Ok((
                        r.get::<_, f64>(0)?,
                        r.get::<_, f64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap();
    println!("\nTop lesson candidates (topics with Learn on):");
    for (ls, l, title, source) in lessons {
        println!("{ls:5.1}  L{l:.2}  {source:<22.22}  {title}");
    }
    let learning: i64 = db
        .call(|c| {
            Ok(
                c.query_row("SELECT count(*) FROM articles WHERE learning_score >= 0.5", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .await
        .unwrap();
    let total = db.call(|c| articles::count(c)).await.unwrap();
    println!("\nlearning material: {learning} of {total} articles");
    println!("total articles stored: {total}");
    assert!(new > 50);
}

/// Feed discovery on real example URLs (P3 §4 live check). Prints what each one finds.
///   cargo test --test live_feeds discover_examples -- --ignored --nocapture
/// Extra URLs: EXAMPLE_URLS="https://a,https://b"
#[tokio::test]
#[ignore]
async fn discover_examples() {
    use tech_english_lib::news::discover;

    let mut urls: Vec<String> = [
        // Substack, WordPress, Ghost, Medium and <link rel=alternate> sites
        "https://seattledataguy.substack.com/p/do-you-actually-need-real-time-data",
        "https://www.confessionsofadataguy.com/",
        "https://www.freecodecamp.org/news/learn-sql-free-relational-database-courses-for-beginners/",
        "https://medium.com/@maximebeauchemin/the-rise-of-the-data-engineer-91be18f1e603",
        "https://realpython.com/python-requests/",
        // No feed at all (SPEC §7.12)
        "https://www.startdataengineering.com/",
        // The user's free data engineering resources (2026-09-30)
        "https://github.com/DataTalksClub/data-engineering-zoomcamp",
        "https://dataenglab.com/data-engineering-courses/",
        "https://www.d8loop.com/learn-data-engineering",
        "https://mode.com/sql-tutorial/introduction-to-sql/",
        "https://docs.getdbt.com/docs/get-started-dbt",
        "https://airflow.apache.org/docs/apache-airflow/stable/tutorial/",
        "https://github.com/harish303118/data-engineer-roadmap",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Ok(extra) = std::env::var("EXAMPLE_URLS") {
        urls = extra.split(',').map(|s| s.trim().to_string()).collect();
    }
    let http = ReqwestClient::new().unwrap();
    let mut no_feed = Vec::new();
    for url in &urls {
        let t = std::time::Instant::now();
        match discover::discover(&http, url, &[]).await {
            Ok((page, cands)) => {
                println!("\n{url}  ({:.1}s)", t.elapsed().as_secs_f64());
                println!(
                    "  page: {:?} · site {:?} · published {:?}",
                    page.title, page.site_name, page.published_at
                );
                if cands.is_empty() {
                    println!("  NO FEED");
                    no_feed.push(url.clone());
                }
                for c in cands {
                    println!(
                        "  feed: {} · {:?} · {} items · newest {:?}",
                        c.url, c.title, c.item_count, c.newest_published_at
                    );
                }
            }
            Err(e) => println!("\n{url}\n  ERROR {e}"),
        }
    }
    assert!(
        no_feed.iter().any(|u| u.contains("startdataengineering")),
        "Start DE has no feed"
    );
}

/// P3 migration + lessons on a COPY of a real database (never the live file).
///   DB_COPY=/path/to/copy.db cargo test --test live_feeds real_db_copy -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn real_db_copy() {
    use tech_english_lib::clock::{Clock, SystemClock};
    use tech_english_lib::db::Db;
    use tech_english_lib::events::RecordingEventSink;
    use tech_english_lib::mode::ModeManager;
    use tech_english_lib::news::NewsService;
    use tech_english_lib::settings::SettingsStore;

    let path = std::env::var("DB_COPY").expect("set DB_COPY to a copy of app.db");
    assert!(
        !path.contains("Application Support"),
        "use a copy, not the live database"
    );
    let t = std::time::Instant::now();
    let db = Db::open(std::path::Path::new(&path)).unwrap();
    println!("opened + migrated in {:?}", t.elapsed());
    let summary = db
        .call(|c| {
            let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
            let picks: Vec<(String, String, i64)> = c
                .prepare("SELECT date, kind, article_id FROM daily_picks ORDER BY date")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            let feeds: i64 = c.query_row("SELECT count(*) FROM feeds", [], |r| r.get(0))?;
            let learning: i64 = c.query_row("SELECT count(*) FROM feeds WHERE learning = 1", [], |r| r.get(0))?;
            c.execute("UPDATE topics SET learn = 1 WHERE name = 'Data Engineering'", [])?;
            Ok(format!(
                "user_version {v}; picks {picks:?}; feeds {feeds} ({learning} learning)"
            ))
        })
        .await
        .unwrap();
    println!("{summary}");

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let settings = SettingsStore::load(db.clone()).await.unwrap();
    let events = Arc::new(RecordingEventSink::default());
    let mode = ModeManager::load(db.clone(), events.clone()).await.unwrap();
    let news = NewsService::new(
        db.clone(),
        Arc::new(ReqwestClient::new().unwrap()),
        clock,
        settings,
        events,
        mode,
        None,
    )
    .await
    .unwrap();
    // What the app does once at start after the upgrade.
    let t = std::time::Instant::now();
    let n = news.backfill_learning().await.unwrap();
    news.reload_topics().await.unwrap();
    println!(
        "learning scores backfilled for {n} articles (+ rescore) in {:?}",
        t.elapsed()
    );
    let t = std::time::Instant::now();
    news.rescore().await.unwrap();
    println!("one rescore (stories + lessons) in {:?}", t.elapsed());
    let top = db
        .call(|c| {
            Ok(c.prepare(
                "SELECT a.lesson_score, a.learning_score, a.source_name, a.title FROM articles a
                 WHERE a.lesson_score IS NOT NULL AND EXISTS (SELECT 1 FROM article_topics at JOIN topics t
                   ON t.id = at.topic_id WHERE at.article_id = a.id AND t.learn = 1 AND at.relevance >= 0.3)
                 ORDER BY a.lesson_score DESC LIMIT 10",
            )?
            .query_map([], |r| {
                Ok(format!(
                    "{:5.1}  L{:.2}  {:<24.24} {}",
                    r.get::<_, f64>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?)
        })
        .await
        .unwrap();
    println!("\nTop lesson candidates from the existing articles:");
    for l in top {
        println!("{l}");
    }
}
