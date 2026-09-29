use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    pub id: i64,
    /// "rss" or "hn"
    pub kind: String,
    pub name: String,
    pub url: String,
    pub source_weight: f64,
    pub enabled: bool,
    pub last_fetched_at: Option<String>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    #[serde(skip)]
    pub etag: Option<String>,
    #[serde(skip)]
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedInput {
    pub id: Option<i64>,
    #[serde(default = "default_kind")]
    pub kind: String,
    pub name: String,
    pub url: String,
    #[serde(default = "default_weight")]
    pub source_weight: f64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_kind() -> String {
    "rss".into()
}
fn default_weight() -> f64 {
    0.5
}
fn default_true() -> bool {
    true
}

const COLS: &str = "id, kind, name, url, source_weight, enabled, last_fetched_at, last_error,
                    consecutive_failures, etag, last_modified";

fn from_row(r: &Row) -> rusqlite::Result<Feed> {
    Ok(Feed {
        id: r.get(0)?,
        kind: r.get(1)?,
        name: r.get(2)?,
        url: r.get(3)?,
        source_weight: r.get(4)?,
        enabled: r.get(5)?,
        last_fetched_at: r.get(6)?,
        last_error: r.get(7)?,
        consecutive_failures: r.get(8)?,
        etag: r.get(9)?,
        last_modified: r.get(10)?,
    })
}

pub fn validate(input: &FeedInput) -> AppResult<FeedInput> {
    let name = input.name.trim().to_string();
    let url = input.url.trim().to_string();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AppError::Invalid("Feed name must be 1–80 characters".into()));
    }
    match input.kind.as_str() {
        "rss" => {
            let u = url::Url::parse(&url).map_err(|_| AppError::Invalid("That is not a valid URL".into()))?;
            if !matches!(u.scheme(), "http" | "https") {
                return Err(AppError::Invalid("Feed URL must start with http:// or https://".into()));
            }
        }
        "hn" => {
            if !url.starts_with("hn:") {
                return Err(AppError::Invalid(
                    "Hacker News feed URL must look like hn:top,best,show".into(),
                ));
            }
        }
        _ => return Err(AppError::Invalid("Unknown feed kind".into())),
    }
    if !(0.0..=1.0).contains(&input.source_weight) {
        return Err(AppError::Invalid("Source weight must be between 0 and 1".into()));
    }
    Ok(FeedInput {
        name,
        url,
        ..input.clone()
    })
}

pub fn list(conn: &Connection) -> AppResult<Vec<Feed>> {
    let mut st = conn.prepare(&format!("SELECT {COLS} FROM feeds ORDER BY kind DESC, name"))?;
    let rows = st.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: i64) -> AppResult<Feed> {
    conn.query_row(&format!("SELECT {COLS} FROM feeds WHERE id = ?1"), [id], from_row)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("feed {id}")))
}

pub fn upsert(conn: &Connection, input: &FeedInput, now: &str) -> AppResult<Feed> {
    let f = validate(input)?;
    let dup: Option<i64> = conn
        .query_row(
            "SELECT id FROM feeds WHERE url = ?1 AND id IS NOT ?2",
            params![f.url, f.id],
            |r| r.get(0),
        )
        .optional()?;
    if dup.is_some() {
        return Err(AppError::Invalid("This feed is already in your list".into()));
    }
    let id = match f.id {
        Some(id) => {
            let n = conn.execute(
                "UPDATE feeds SET name=?1, url=?2, source_weight=?3, enabled=?4 WHERE id=?5",
                params![f.name, f.url, f.source_weight, f.enabled, id],
            )?;
            if n == 0 {
                return Err(AppError::NotFound(format!("feed {id}")));
            }
            id
        }
        None => {
            conn.execute(
                "INSERT INTO feeds(kind, name, url, source_weight, enabled, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
                params![f.kind, f.name, f.url, f.source_weight, f.enabled, now],
            )?;
            conn.last_insert_rowid()
        }
    };
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM feeds WHERE id = ?1", [id])?;
    Ok(())
}

pub fn record_success(
    conn: &Connection,
    id: i64,
    now: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE feeds SET last_fetched_at=?1, last_error=NULL, consecutive_failures=0,
         etag=COALESCE(?2, etag), last_modified=COALESCE(?3, last_modified) WHERE id=?4",
        params![now, etag, last_modified, id],
    )?;
    Ok(())
}

pub fn record_failure(conn: &Connection, id: i64, now: &str, error: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE feeds SET last_fetched_at=?1, last_error=?2, consecutive_failures=consecutive_failures+1 WHERE id=?3",
        params![now, error, id],
    )?;
    Ok(())
}
