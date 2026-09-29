//! Small key/value store for runtime state (mode, first-fetch time, daily job date…).

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;

pub const MODE: &str = "mode";
pub const FIRST_FETCH_COMPLETED_AT: &str = "first_fetch_completed_at";
pub const LAST_DAILY_JOB_DATE: &str = "last_daily_job_date";

pub fn get(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM app_state WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn set(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO app_state(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}
