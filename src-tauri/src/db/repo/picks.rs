use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;

/// `daily_picks.kind` values (P3).
pub const STORY: &str = "story";
pub const LESSON: &str = "lesson";

#[derive(Debug, Clone, PartialEq)]
pub struct PickRow {
    pub date: String,
    pub kind: String,
    pub article_id: i64,
    pub why: String,
    pub why_source: String,
}

pub fn get(conn: &Connection, date: &str, kind: &str) -> AppResult<Option<PickRow>> {
    Ok(conn
        .query_row(
            "SELECT date, kind, article_id, why, why_source FROM daily_picks WHERE date = ?1 AND kind = ?2",
            [date, kind],
            |r| {
                Ok(PickRow {
                    date: r.get(0)?,
                    kind: r.get(1)?,
                    article_id: r.get(2)?,
                    why: r.get(3)?,
                    why_source: r.get(4)?,
                })
            },
        )
        .optional()?)
}

pub fn insert(conn: &Connection, date: &str, kind: &str, article_id: i64, why: &str, now: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO daily_picks(date, kind, article_id, why, created_at) VALUES (?1,?2,?3,?4,?5)",
        params![date, kind, article_id, why, now],
    )?;
    Ok(())
}

/// Articles picked (story or lesson) on or after `since_date` (YYYY-MM-DD).
pub fn article_ids_since(conn: &Connection, since_date: &str) -> AppResult<Vec<i64>> {
    let mut st = conn.prepare("SELECT article_id FROM daily_picks WHERE date >= ?1")?;
    let rows = st
        .query_map([since_date], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_why(conn: &Connection, date: &str, kind: &str, why: &str, source: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE daily_picks SET why = ?1, why_source = ?2 WHERE date = ?3 AND kind = ?4",
        params![why, source, date, kind],
    )?;
    Ok(())
}
