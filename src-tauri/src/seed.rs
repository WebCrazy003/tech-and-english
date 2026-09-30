//! Default topics/feeds offered during onboarding (SPEC §5.4, §7.2).

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::repo::feeds::{self, FeedInput};
use crate::db::repo::topics::{self, TopicInput};
use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicSeed {
    pub name: String,
    pub priority: u8,
    pub keywords: Vec<String>,
    pub excluded_keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedSeed {
    pub kind: String,
    pub name: String,
    pub url: String,
    pub source_weight: f64,
    /// Section shown in onboarding ("General tech", "AI", …).
    #[serde(default)]
    pub group: String,
    /// Mostly publishes learning material (P3).
    #[serde(default)]
    pub learning: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingDefaults {
    pub topics: Vec<TopicSeed>,
    pub feeds: Vec<FeedSeed>,
}

pub fn defaults() -> OnboardingDefaults {
    OnboardingDefaults {
        topics: serde_json::from_str(include_str!("../resources/default_topics.json")).expect("valid topics seed"),
        feeds: serde_json::from_str(include_str!("../resources/default_feeds.json")).expect("valid feeds seed"),
    }
}

/// Insert the chosen topics and feeds. Existing names/URLs are skipped, so it is idempotent.
pub fn apply(conn: &Connection, topic_names: &[String], feed_urls: &[String], now: &str) -> AppResult<()> {
    let d = defaults();
    let existing_topics: Vec<String> = topics::list(conn)?.into_iter().map(|t| t.name.to_lowercase()).collect();
    for t in d.topics.iter().filter(|t| topic_names.contains(&t.name)) {
        if existing_topics.contains(&t.name.to_lowercase()) {
            continue;
        }
        topics::upsert(
            conn,
            &TopicInput {
                id: None,
                name: t.name.clone(),
                keywords: t.keywords.clone(),
                excluded_keywords: t.excluded_keywords.clone(),
                priority: t.priority,
                enabled: true,
                notify: t.priority == 3,
                notify_threshold: None,
                learn: false,
            },
            now,
        )?;
    }
    let existing_feeds: Vec<String> = feeds::list(conn)?.into_iter().map(|f| f.url).collect();
    for f in d.feeds.iter().filter(|f| feed_urls.contains(&f.url)) {
        if existing_feeds.contains(&f.url) {
            continue;
        }
        feeds::upsert(
            conn,
            &FeedInput {
                id: None,
                kind: f.kind.clone(),
                name: f.name.clone(),
                url: f.url.clone(),
                source_weight: f.source_weight,
                enabled: true,
                learning: f.learning,
            },
            now,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn seeds_parse_and_validate() {
        let d = defaults();
        assert_eq!(d.topics.len(), 13);
        assert!(d.feeds.len() >= 30);
        for f in &d.feeds {
            feeds::validate(&FeedInput {
                id: None,
                kind: f.kind.clone(),
                name: f.name.clone(),
                url: f.url.clone(),
                source_weight: f.source_weight,
                enabled: true,
                learning: f.learning,
            })
            .unwrap();
        }
    }

    #[tokio::test]
    async fn applies_only_selected_and_is_idempotent() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            let names = vec!["LLMs".to_string(), "Python".to_string()];
            let urls = vec![
                "hn:top,best,show".to_string(),
                "https://duckdb.org/feed.xml".to_string(),
            ];
            apply(c, &names, &urls, "2026-09-29T00:00:00Z")?;
            apply(c, &names, &urls, "2026-09-29T00:00:00Z")?;
            let t = topics::list(c)?;
            assert_eq!(t.len(), 2);
            assert!(
                t.iter().find(|t| t.name == "LLMs").unwrap().notify,
                "high priority → notify on"
            );
            assert_eq!(feeds::list(c)?.len(), 2);
            Ok(())
        })
        .await
        .unwrap();
    }
}
