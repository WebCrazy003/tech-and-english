//! Store fetched items: normalize → find duplicate → merge or insert → match topics.
//! Called inside one transaction per feed batch.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;

use super::model::RawItem;
use super::normalize::{host_of, jaccard, looks_english, normalize_url, title_key, truncate_chars, word_set};
use super::topics::{CompiledTopic, match_topics};
use crate::clock::fmt_ts;
use crate::db::repo::articles::{self, NewArticle};
use crate::db::repo::feeds::Feed;
use crate::error::AppResult;

pub const DEDUPE_WINDOW_HOURS: i64 = 72;
const JACCARD_DUPLICATE: f64 = 0.8;

#[derive(Debug, Default, PartialEq)]
pub struct IngestStats {
    pub new: u32,
    pub merged: u32,
    pub skipped: u32,
}

/// Recent titles held in memory for fuzzy dedupe during one batch.
pub struct RecentTitles {
    entries: Vec<(i64, String)>,
}

impl RecentTitles {
    pub fn load(conn: &Connection, now: DateTime<Utc>) -> AppResult<Self> {
        let since = fmt_ts(now - Duration::hours(DEDUPE_WINDOW_HOURS));
        Ok(Self {
            entries: articles::recent_title_keys(conn, &since)?,
        })
    }

    fn find(&self, key: &str) -> Option<i64> {
        if let Some((id, _)) = self.entries.iter().find(|(_, k)| k == key) {
            return Some(*id);
        }
        let ws = word_set(key);
        self.entries
            .iter()
            .find(|(_, k)| jaccard(&ws, &word_set(k)) >= JACCARD_DUPLICATE)
            .map(|(id, _)| *id)
    }

    fn push(&mut self, id: i64, key: String) {
        self.entries.push((id, key));
    }
}

fn source_name_for(feed: &Feed, raw: &RawItem) -> String {
    if feed.kind == "hn" {
        match host_of(&raw.url).as_deref() {
            Some("news.ycombinator.com") | None => "Hacker News".into(),
            Some(h) => h.to_string(),
        }
    } else {
        feed.name.clone()
    }
}

pub fn rematch(conn: &Connection, id: i64, topics: &[CompiledTopic]) -> AppResult<()> {
    let (title, desc) = articles::match_text(conn, id)?;
    articles::set_topics(conn, id, &match_topics(topics, &title, desc.as_deref()))
}

pub fn ingest(
    conn: &Connection,
    feed: &Feed,
    items: &[RawItem],
    topics: &[CompiledTopic],
    recent: &mut RecentTitles,
    now: DateTime<Utc>,
    max_age_days: u32,
) -> AppResult<IngestStats> {
    let mut stats = IngestStats::default();
    let now_s = fmt_ts(now);
    let oldest = now - Duration::days(max_age_days as i64);

    for raw in items {
        if raw.published_at.is_some_and(|p| p < oldest) {
            stats.skipped += 1;
            continue;
        }
        let sample = format!(
            "{} {}",
            raw.title,
            raw.description
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(300)
                .collect::<String>()
        );
        if !looks_english(&sample) {
            stats.skipped += 1;
            continue;
        }
        let Ok(norm) = normalize_url(&raw.url) else {
            tracing::debug!(feed = feed.id, "skipping item with invalid url");
            stats.skipped += 1;
            continue;
        };
        let title = truncate_chars(&raw.title, 300);
        let key_source = if feed.kind == "hn" { "" } else { feed.name.as_str() };
        let key = title_key(&title, key_source);
        if key.is_empty() {
            stats.skipped += 1;
            continue;
        }
        let published = raw.published_at.map(fmt_ts);

        let existing = match articles::find_id_by_normalized_url(conn, &norm)? {
            Some(id) => Some(id),
            None => match &raw.hn {
                Some(h) => articles::find_id_by_hn_id(conn, h.id)?,
                None => None,
            },
        }
        .or_else(|| recent.find(&key));

        let id = match existing {
            Some(id) => {
                articles::merge(
                    conn,
                    id,
                    raw.description.as_deref(),
                    raw.author.as_deref(),
                    published.as_deref(),
                    raw.hn.as_ref(),
                    &now_s,
                )?;
                // Prefer a readable publication name over an HN link's domain.
                if feed.kind == "rss" && !articles::has_rss_source(conn, id)? {
                    articles::set_source_name(conn, id, &feed.name)?;
                }
                stats.merged += 1;
                id
            }
            None => {
                let id = articles::insert(
                    conn,
                    &NewArticle {
                        url: raw.url.clone(),
                        normalized_url: norm,
                        title,
                        title_key: key.clone(),
                        source_name: source_name_for(feed, raw),
                        author: raw.author.clone(),
                        description: raw.description.clone(),
                        published_at: published,
                        discovered_at: now_s.clone(),
                        hn: raw.hn.clone(),
                    },
                )?;
                recent.push(id, key);
                stats.new += 1;
                id
            }
        };
        articles::add_source(conn, id, feed.id, &raw.url, &now_s)?;
        rematch(conn, id, topics)?;
    }
    Ok(stats)
}

/// Articles in `ids` that have a new or changed topic match. Used after topic edits.
pub fn rematch_all(conn: &Connection, ids: &[i64], topics: &[CompiledTopic]) -> AppResult<()> {
    let unique: HashSet<i64> = ids.iter().copied().collect();
    for id in unique {
        rematch(conn, id, topics)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::parse_ts;
    use crate::db::Db;
    use crate::db::repo::{feeds, topics};
    use crate::news::model::HnStats;
    use crate::news::topics::compile;

    fn raw(url: &str, title: &str, hn: Option<i64>, published: &str) -> RawItem {
        RawItem {
            url: url.into(),
            title: title.into(),
            description: None,
            author: None,
            published_at: parse_ts(published),
            hn: hn.map(|id| HnStats {
                id,
                points: 100,
                comments: 10,
            }),
        }
    }

    fn setup(c: &Connection) -> (Feed, Feed, Vec<CompiledTopic>) {
        let now = "2026-09-29T08:00:00Z";
        let rss = feeds::upsert(
            c,
            &feeds::FeedInput {
                id: None,
                kind: "rss".into(),
                name: "Example Tech".into(),
                url: "https://example.com/feed".into(),
                source_weight: 0.5,
                enabled: true,
            },
            now,
        )
        .unwrap();
        let hn = feeds::upsert(
            c,
            &feeds::FeedInput {
                id: None,
                kind: "hn".into(),
                name: "Hacker News".into(),
                url: "hn:top".into(),
                source_weight: 0.5,
                enabled: true,
            },
            now,
        )
        .unwrap();
        let t = topics::upsert(
            c,
            &topics::TopicInput {
                id: None,
                name: "AI".into(),
                keywords: vec!["AI".into()],
                excluded_keywords: vec![],
                priority: 3,
                enabled: true,
                notify: false,
                notify_threshold: None,
            },
            now,
        )
        .unwrap();
        (rss, hn, compile(&[t]))
    }

    #[tokio::test]
    async fn same_story_from_hn_and_rss_merges() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            let (rss, hn, topics) = setup(c);
            let now = parse_ts("2026-09-29T08:00:00Z").unwrap();
            let mut recent = RecentTitles::load(c, now)?;
            let s1 = ingest(
                c,
                &hn,
                &[raw(
                    "https://www.example.com/posts/local-ai-model/",
                    "New local AI model",
                    Some(1),
                    "2026-09-29T06:00:00Z",
                )],
                &topics,
                &mut recent,
                now,
                7,
            )?;
            assert_eq!(s1.new, 1);
            let s2 = ingest(
                c,
                &rss,
                &[raw(
                    "https://example.com/posts/local-ai-model?utm_source=rss",
                    "New local AI model",
                    None,
                    "2026-09-29T06:00:00Z",
                )],
                &topics,
                &mut recent,
                now,
                7,
            )?;
            assert_eq!(
                s2,
                IngestStats {
                    new: 0,
                    merged: 1,
                    skipped: 0
                }
            );
            assert_eq!(articles::count(c)?, 1);
            let item = articles::get_item(c, 1)?;
            assert_eq!(item.hn_points, Some(100));
            assert_eq!(item.source_name, "Example Tech", "RSS name replaces HN domain");
            assert_eq!(item.topics, vec!["AI"]);
            assert_eq!(articles::source_feed_ids(c, 1)?.len(), 2);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn dedupe_by_hn_id_and_titles() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            let (rss, hn, topics) = setup(c);
            let now = parse_ts("2026-09-29T08:00:00Z").unwrap();
            let mut recent = RecentTitles::load(c, now)?;
            let p = "2026-09-29T06:00:00Z";
            ingest(
                c,
                &hn,
                &[raw(
                    "https://a.com/x",
                    "Rust compiler gets much faster builds today",
                    Some(7),
                    p,
                )],
                &topics,
                &mut recent,
                now,
                7,
            )?;
            // same hn id, different url
            let s = ingest(
                c,
                &hn,
                &[raw("https://a.com/x2", "Other", Some(7), p)],
                &topics,
                &mut recent,
                now,
                7,
            )?;
            assert_eq!(s.merged, 1);
            // same title key, different url
            let s = ingest(
                c,
                &rss,
                &[raw(
                    "https://b.com/y",
                    "Rust compiler gets much faster builds today!",
                    None,
                    p,
                )],
                &topics,
                &mut recent,
                now,
                7,
            )?;
            assert_eq!(s.merged, 1);
            // jaccard 6/7 = 0.86 ≥ 0.8 → merge
            let s = ingest(
                c,
                &rss,
                &[raw("https://c.com/z", "Rust compiler gets much faster builds", None, p)],
                &topics,
                &mut recent,
                now,
                7,
            )?;
            assert_eq!(s.merged, 1);
            // clearly different → new
            let s = ingest(
                c,
                &rss,
                &[raw("https://d.com/w", "Postgres 19 released", None, p)],
                &topics,
                &mut recent,
                now,
                7,
            )?;
            assert_eq!(s.new, 1);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn jaccard_threshold_boundary() {
        // 4 shared of 5 total words = exactly 0.8 → counts as duplicate
        let a = word_set("alpha beta gamma delta epsilon");
        let b = word_set("alpha beta gamma delta");
        assert!((jaccard(&a, &b) - 0.8).abs() < 1e-9);
        let c = word_set("alpha beta gamma zeta eta");
        assert!(jaccard(&a, &c) < 0.8);
    }

    #[tokio::test]
    async fn title_dedupe_only_within_window_and_old_items_skipped() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            let (rss, _hn, topics) = setup(c);
            let t0 = parse_ts("2026-09-20T08:00:00Z").unwrap();
            let mut recent = RecentTitles::load(c, t0)?;
            ingest(
                c,
                &rss,
                &[raw(
                    "https://a.com/1",
                    "Weekly AI roundup",
                    None,
                    "2026-09-20T07:00:00Z",
                )],
                &topics,
                &mut recent,
                t0,
                7,
            )?;
            let t1 = parse_ts("2026-09-29T08:00:00Z").unwrap();
            let mut recent = RecentTitles::load(c, t1)?;
            let s = ingest(
                c,
                &rss,
                &[
                    raw("https://a.com/2", "Weekly AI roundup", None, "2026-09-29T07:00:00Z"),
                    raw("https://a.com/3", "Ancient", None, "2024-01-01T00:00:00Z"),
                ],
                &topics,
                &mut recent,
                t1,
                7,
            )?;
            assert_eq!(
                s,
                IngestStats {
                    new: 1,
                    merged: 0,
                    skipped: 1
                }
            );
            Ok(())
        })
        .await
        .unwrap();
    }
}
