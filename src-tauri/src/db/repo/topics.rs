use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{json_list, parse_json_list};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Topic {
    pub id: i64,
    pub name: String,
    pub keywords: Vec<String>,
    pub excluded_keywords: Vec<String>,
    /// 1 = low, 2 = normal, 3 = high
    pub priority: u8,
    pub enabled: bool,
    pub notify: bool,
    pub notify_threshold: Option<u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub excluded_keywords: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: u8,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub notify: bool,
    #[serde(default)]
    pub notify_threshold: Option<u8>,
}

fn default_priority() -> u8 {
    2
}
fn default_true() -> bool {
    true
}

const COLS: &str = "id, name, keywords, excluded_keywords, priority, enabled, notify, notify_threshold";

fn from_row(r: &Row) -> rusqlite::Result<Topic> {
    Ok(Topic {
        id: r.get(0)?,
        name: r.get(1)?,
        keywords: parse_json_list(&r.get::<_, String>(2)?),
        excluded_keywords: parse_json_list(&r.get::<_, String>(3)?),
        priority: r.get(4)?,
        enabled: r.get(5)?,
        notify: r.get(6)?,
        notify_threshold: r.get(7)?,
    })
}

/// Trim, drop empties and case-insensitive duplicates, keep order.
fn clean_keywords(v: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    v.iter()
        .map(|k| k.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|k| !k.is_empty() && k.len() <= 80 && seen.insert(k.to_lowercase()))
        .collect()
}

pub fn validate(input: &TopicInput) -> AppResult<TopicInput> {
    let name = input.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(AppError::Invalid("Topic name must be 1–60 characters".into()));
    }
    if !(1..=3).contains(&input.priority) {
        return Err(AppError::Invalid("Priority must be low, normal or high".into()));
    }
    if input.notify_threshold.is_some_and(|t| t > 100) {
        return Err(AppError::Invalid("Notification threshold must be 0–100".into()));
    }
    let keywords = clean_keywords(&input.keywords);
    if keywords.is_empty() {
        return Err(AppError::Invalid("Add at least one keyword".into()));
    }
    Ok(TopicInput {
        name,
        keywords,
        excluded_keywords: clean_keywords(&input.excluded_keywords),
        ..input.clone()
    })
}

pub fn list(conn: &Connection) -> AppResult<Vec<Topic>> {
    let mut st = conn.prepare(&format!("SELECT {COLS} FROM topics ORDER BY priority DESC, name"))?;
    let rows = st.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: i64) -> AppResult<Topic> {
    conn.query_row(&format!("SELECT {COLS} FROM topics WHERE id = ?1"), [id], from_row)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("topic {id}")))
}

pub fn upsert(conn: &Connection, input: &TopicInput, now: &str) -> AppResult<Topic> {
    let t = validate(input)?;
    let dup: Option<i64> = conn
        .query_row(
            "SELECT id FROM topics WHERE lower(name) = lower(?1) AND id IS NOT ?2",
            params![t.name, t.id],
            |r| r.get(0),
        )
        .optional()?;
    if dup.is_some() {
        return Err(AppError::Invalid(format!(
            "A topic named \"{}\" already exists",
            t.name
        )));
    }
    let id = match t.id {
        Some(id) => {
            let n = conn.execute(
                "UPDATE topics SET name=?1, keywords=?2, excluded_keywords=?3, priority=?4, enabled=?5,
                 notify=?6, notify_threshold=?7 WHERE id=?8",
                params![
                    t.name,
                    json_list(&t.keywords),
                    json_list(&t.excluded_keywords),
                    t.priority,
                    t.enabled,
                    t.notify,
                    t.notify_threshold,
                    id
                ],
            )?;
            if n == 0 {
                return Err(AppError::NotFound(format!("topic {id}")));
            }
            id
        }
        None => {
            conn.execute(
                "INSERT INTO topics(name, keywords, excluded_keywords, priority, enabled, notify, notify_threshold, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    t.name,
                    json_list(&t.keywords),
                    json_list(&t.excluded_keywords),
                    t.priority,
                    t.enabled,
                    t.notify,
                    t.notify_threshold,
                    now
                ],
            )?;
            conn.last_insert_rowid()
        }
    };
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM topics WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn input(name: &str, kws: &[&str]) -> TopicInput {
        TopicInput {
            id: None,
            name: name.into(),
            keywords: kws.iter().map(|s| s.to_string()).collect(),
            excluded_keywords: vec![],
            priority: 2,
            enabled: true,
            notify: false,
            notify_threshold: None,
        }
    }

    #[tokio::test]
    async fn crud_and_validation() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            let t = upsert(
                c,
                &input(" AI ", &["LLM", " llm ", "", "agent"]),
                "2026-09-29T00:00:00Z",
            )?;
            assert_eq!(t.name, "AI");
            assert_eq!(t.keywords, vec!["LLM", "agent"]);
            assert!(upsert(c, &input("ai", &["x"]), "t").is_err(), "duplicate name");
            assert!(upsert(c, &input("Empty", &[]), "t").is_err(), "needs keywords");
            let mut edit = input("AI 2", &["x"]);
            edit.id = Some(t.id);
            assert_eq!(upsert(c, &edit, "t")?.name, "AI 2");
            delete(c, t.id)?;
            assert!(list(c)?.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    }
}
