use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;

pub fn insert(conn: &Connection, kind: &str, article_id: Option<i64>, now: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO notifications_log(kind, article_id, sent_at) VALUES (?1,?2,?3)",
        params![kind, article_id, now],
    )?;
    Ok(())
}

pub fn count_since(conn: &Connection, kind: &str, since: &str) -> AppResult<u32> {
    Ok(conn.query_row(
        "SELECT count(*) FROM notifications_log WHERE kind = ?1 AND sent_at >= ?2",
        params![kind, since],
        |r| r.get(0),
    )?)
}

pub fn last_sent_at(conn: &Connection, kind: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT max(sent_at) FROM notifications_log WHERE kind = ?1",
            [kind],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

pub fn was_notified(conn: &Connection, article_id: i64) -> AppResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM notifications_log WHERE article_id = ?1)",
        [article_id],
        |r| r.get(0),
    )?)
}
