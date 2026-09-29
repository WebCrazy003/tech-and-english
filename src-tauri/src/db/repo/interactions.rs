use std::collections::HashMap;

use rusqlite::{Connection, params};

use crate::error::AppResult;

pub fn insert(conn: &Connection, article_id: i64, kind: &str, now: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO article_interactions(article_id, kind, created_at) VALUES (?1,?2,?3)",
        params![article_id, kind, now],
    )?;
    Ok(())
}

pub fn has(conn: &Connection, article_id: i64, kind: &str) -> AppResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM article_interactions WHERE article_id = ?1 AND kind = ?2)",
        params![article_id, kind],
        |r| r.get(0),
    )?)
}

/// Add `delta` to an affinity, clamped to [-1, 1].
pub fn add_affinity(conn: &Connection, kind: &str, ref_id: i64, delta: f64, now: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO affinities(kind, ref_id, value, updated_at) VALUES (?1, ?2, max(-1.0, min(1.0, ?3)), ?4)
         ON CONFLICT(kind, ref_id) DO UPDATE SET
           value = max(-1.0, min(1.0, value + excluded.value)), updated_at = excluded.updated_at",
        params![kind, ref_id, delta, now],
    )?;
    Ok(())
}

/// All affinities keyed by (kind, ref_id).
pub fn all_affinities(conn: &Connection) -> AppResult<HashMap<(String, i64), f64>> {
    let mut st = conn.prepare("SELECT kind, ref_id, value FROM affinities")?;
    let rows = st
        .query_map([], |r| {
            Ok(((r.get::<_, String>(0)?, r.get::<_, i64>(1)?), r.get::<_, f64>(2)?))
        })?
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}
