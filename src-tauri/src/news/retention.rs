//! Daily cleanup (SPEC §7.10). Later phases extend the "protected" conditions for their tables.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};

use crate::clock::fmt_ts;
use crate::error::AppResult;

pub const DELETE_AFTER_DAYS: i64 = 60;
pub const CLEAR_BODY_AFTER_DAYS: i64 = 14;

#[derive(Debug, PartialEq)]
pub struct RetentionStats {
    pub deleted: usize,
    pub bodies_cleared: usize,
}

pub fn run(conn: &Connection, now: DateTime<Utc>) -> AppResult<RetentionStats> {
    let delete_before = fmt_ts(now - Duration::days(DELETE_AFTER_DAYS));
    let clear_before = fmt_ts(now - Duration::days(CLEAR_BODY_AFTER_DAYS));
    let deleted = conn.execute(
        "DELETE FROM articles WHERE discovered_at < ?1 AND saved = 0
           AND id NOT IN (SELECT article_id FROM daily_picks)
           AND NOT EXISTS (SELECT 1 FROM article_chats c WHERE c.article_id = articles.id)
           AND NOT EXISTS (SELECT 1 FROM vocab_contexts v WHERE v.article_id = articles.id)
           AND NOT EXISTS (SELECT 1 FROM conversations v WHERE v.article_id = articles.id)",
        params![delete_before],
    )?;
    let bodies_cleared = conn.execute(
        "UPDATE articles SET body_text = NULL, body_html = NULL,
           body_status = CASE WHEN body_status = 'ok' THEN 'none' ELSE body_status END
         WHERE discovered_at < ?1 AND saved = 0 AND (body_text IS NOT NULL OR body_html IS NOT NULL)",
        params![clear_before],
    )?;
    if deleted > 0 || bodies_cleared > 0 {
        tracing::info!(deleted, bodies_cleared, "retention cleanup");
    }
    Ok(RetentionStats {
        deleted,
        bodies_cleared,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::parse_ts;
    use crate::db::Db;

    fn add(c: &Connection, id: i64, discovered: &str, saved: bool) {
        c.execute(
            "INSERT INTO articles(id, url, normalized_url, title, title_key, source_name, discovered_at, saved, body_text, body_status)
             VALUES (?1, 'u', ?2, 't', 't', 's', ?3, ?4, 'body', 'ok')",
            params![id, format!("https://x/{id}"), discovered, saved],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn deletes_old_keeps_saved_and_picked() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            add(c, 1, "2026-07-01T00:00:00Z", false); // old → delete
            add(c, 2, "2026-07-01T00:00:00Z", true); // old but saved → keep
            add(c, 3, "2026-07-01T00:00:00Z", false); // old but was a pick → keep
            add(c, 4, "2026-09-10T00:00:00Z", false); // 19 days → clear body
            add(c, 5, "2026-09-28T00:00:00Z", false); // recent → untouched
            add(c, 6, "2026-07-01T00:00:00Z", false); // old but has an AI chat → keep
            add(c, 7, "2026-07-01T00:00:00Z", false); // old but a Word Book context → keep
            add(c, 8, "2026-07-01T00:00:00Z", false); // old but talked about (P5) → keep
            c.execute(
                "INSERT INTO conversations(article_id, settings, started_at) VALUES (8, '{}', 't')",
                [],
            )?;
            c.execute(
                "INSERT INTO vocab_items(id, kind, text, text_key, created_at, updated_at) VALUES (1, 'word', 'x', 'x', 't', 't');
                 ",
                [],
            )?;
            c.execute(
                "INSERT INTO vocab_contexts(item_id, article_id, sentence, created_at) VALUES (1, 7, 's', 't')",
                [],
            )?;
            c.execute(
                "INSERT INTO article_chats(article_id, role, content, created_at) VALUES (6, 'user', 'hi', 'x')",
                [],
            )?;
            c.execute(
                "INSERT INTO daily_picks(date, article_id, why, created_at) VALUES ('2026-07-01', 3, 'w', 'x')",
                [],
            )?;
            let s = run(c, parse_ts("2026-09-29T00:00:00Z").unwrap())?;
            assert_eq!(s.deleted, 1);
            let ids: Vec<i64> = c
                .prepare("SELECT id FROM articles ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            assert_eq!(ids, vec![2, 3, 4, 5, 6, 7, 8]);
            let (body, status): (Option<String>, String) =
                c.query_row("SELECT body_text, body_status FROM articles WHERE id = 4", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            assert_eq!((body, status.as_str()), (None, "none"));
            let body5: Option<String> = c.query_row("SELECT body_text FROM articles WHERE id = 5", [], |r| r.get(0))?;
            assert!(body5.is_some());
            Ok(())
        })
        .await
        .unwrap();
    }
}
