use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;

#[derive(Debug, Clone, PartialEq)]
pub struct PickRow {
    pub date: String,
    pub article_id: i64,
    pub why: String,
    pub why_source: String,
}

pub fn get(conn: &Connection, date: &str) -> AppResult<Option<PickRow>> {
    Ok(conn
        .query_row(
            "SELECT date, article_id, why, why_source FROM daily_picks WHERE date = ?1",
            [date],
            |r| {
                Ok(PickRow {
                    date: r.get(0)?,
                    article_id: r.get(1)?,
                    why: r.get(2)?,
                    why_source: r.get(3)?,
                })
            },
        )
        .optional()?)
}

pub fn insert(conn: &Connection, date: &str, article_id: i64, why: &str, now: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO daily_picks(date, article_id, why, created_at) VALUES (?1,?2,?3,?4)",
        params![date, article_id, why, now],
    )?;
    Ok(())
}

/// Articles picked on or after `since_date` (YYYY-MM-DD).
pub fn article_ids_since(conn: &Connection, since_date: &str) -> AppResult<Vec<i64>> {
    let mut st = conn.prepare("SELECT article_id FROM daily_picks WHERE date >= ?1")?;
    let rows = st
        .query_map([since_date], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_why(conn: &Connection, date: &str, why: &str, source: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE daily_picks SET why = ?1, why_source = ?2 WHERE date = ?3",
        params![why, source, date],
    )?;
    Ok(())
}
