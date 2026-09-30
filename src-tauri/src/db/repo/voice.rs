//! Voice tutor conversations, their turns and learning observations (P5 dev spec §11).

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use serde_json::Value;

use crate::clock::fmt_ts;
use crate::error::{AppError, AppResult};

/// Pending reviews are kept at least this long, even when transcripts expire sooner.
pub const PENDING_REVIEW_DAYS: i64 = 180;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: i64,
    pub article_id: Option<i64>,
    pub article_title: Option<String>,
    pub settings: Value,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub user_speaking_seconds: i64,
    /// pending | done | skipped
    pub review_status: String,
    pub corrections: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub id: i64,
    pub seq: i64,
    /// user | tutor
    pub role: String,
    pub text: String,
    pub meta: Option<Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub id: i64,
    /// unknown_word | unknown_phrase | pronunciation | grammar | useful_sentence
    pub kind: String,
    pub text: String,
    pub detail: Option<Value>,
    pub saved_item_id: Option<i64>,
    pub created_at: String,
}

fn json_col(s: Option<String>) -> Option<Value> {
    s.and_then(|s| serde_json::from_str(&s).ok())
}

const CONV_COLS: &str = "c.id, c.article_id, a.title, c.settings, c.started_at, c.ended_at, c.user_speaking_seconds,
    c.review_status,
    (SELECT COUNT(*) FROM learning_observations o WHERE o.conversation_id = c.id AND o.kind = 'grammar')";

fn conv_from_row(r: &Row) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: r.get(0)?,
        article_id: r.get(1)?,
        article_title: r.get(2)?,
        settings: json_col(r.get(3)?).unwrap_or(Value::Null),
        started_at: r.get(4)?,
        ended_at: r.get(5)?,
        user_speaking_seconds: r.get(6)?,
        review_status: r.get(7)?,
        corrections: r.get(8)?,
    })
}

pub fn create(conn: &Connection, article_id: Option<i64>, settings: &Value, now: &str) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO conversations(article_id, settings, started_at) VALUES (?1, ?2, ?3)",
        params![article_id, settings.to_string(), now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> AppResult<Conversation> {
    conn.query_row(
        &format!("SELECT {CONV_COLS} FROM conversations c LEFT JOIN articles a ON a.id = c.article_id WHERE c.id = ?1"),
        [id],
        conv_from_row,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("conversation {id}")))
}

/// Newest first.
pub fn list(conn: &Connection, limit: u32) -> AppResult<Vec<Conversation>> {
    let mut st = conn.prepare(&format!(
        "SELECT {CONV_COLS} FROM conversations c LEFT JOIN articles a ON a.id = c.article_id
         ORDER BY c.started_at DESC, c.id DESC LIMIT ?1"
    ))?;
    let rows = st.query_map([limit], conv_from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn end(conn: &Connection, id: i64, ended_at: &str, speaking_seconds: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE conversations SET ended_at = ?2, user_speaking_seconds = ?3 WHERE id = ?1",
        params![id, ended_at, speaking_seconds],
    )?;
    Ok(())
}

/// Conversations left open by a crash: end them at their last turn.
pub fn close_dangling(conn: &Connection) -> AppResult<usize> {
    Ok(conn.execute(
        "UPDATE conversations SET ended_at = COALESCE(
            (SELECT MAX(created_at) FROM conversation_turns t WHERE t.conversation_id = conversations.id),
            started_at)
         WHERE ended_at IS NULL",
        [],
    )?)
}

pub fn set_review_status(conn: &Connection, id: i64, status: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE conversations SET review_status = ?2 WHERE id = ?1",
        params![id, status],
    )?;
    Ok(())
}

pub fn add_turn(
    conn: &Connection,
    conversation_id: i64,
    role: &str,
    text: &str,
    meta: Option<&Value>,
    now: &str,
) -> AppResult<Turn> {
    let seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM conversation_turns WHERE conversation_id = ?1",
        [conversation_id],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO conversation_turns(conversation_id, seq, role, text, meta, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
        params![conversation_id, seq, role, text, meta.map(Value::to_string), now],
    )?;
    Ok(Turn {
        id: conn.last_insert_rowid(),
        seq,
        role: role.into(),
        text: text.into(),
        meta: meta.cloned(),
        created_at: now.into(),
    })
}

pub fn turns(conn: &Connection, conversation_id: i64) -> AppResult<Vec<Turn>> {
    let mut st = conn.prepare(
        "SELECT id, seq, role, text, meta, created_at FROM conversation_turns WHERE conversation_id = ?1 ORDER BY seq",
    )?;
    let rows = st
        .query_map([conversation_id], |r| {
            Ok(Turn {
                id: r.get(0)?,
                seq: r.get(1)?,
                role: r.get(2)?,
                text: r.get(3)?,
                meta: json_col(r.get(4)?),
                created_at: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn add_observation(
    conn: &Connection,
    conversation_id: i64,
    kind: &str,
    text: &str,
    detail: Option<&Value>,
    now: &str,
) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO learning_observations(conversation_id, kind, text, detail, created_at) VALUES (?1,?2,?3,?4,?5)",
        params![conversation_id, kind, text, detail.map(Value::to_string), now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn observations(conn: &Connection, conversation_id: i64) -> AppResult<Vec<Observation>> {
    let mut st = conn.prepare(
        "SELECT id, kind, text, detail, saved_item_id, created_at FROM learning_observations
         WHERE conversation_id = ?1 ORDER BY id",
    )?;
    let rows = st
        .query_map([conversation_id], |r| {
            Ok(Observation {
                id: r.get(0)?,
                kind: r.get(1)?,
                text: r.get(2)?,
                detail: json_col(r.get(3)?),
                saved_item_id: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_observation_saved(conn: &Connection, id: i64, item_id: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE learning_observations SET saved_item_id = ?2 WHERE id = ?1",
        params![id, item_id],
    )?;
    Ok(())
}

fn delete_where(conn: &Connection, condition: &str, args: &[&dyn rusqlite::ToSql]) -> AppResult<usize> {
    // Word Book contexts point at conversations without a foreign key (0004): clear them first.
    conn.execute(
        &format!(
            "DELETE FROM vocab_contexts WHERE conversation_id IN (SELECT id FROM conversations WHERE {condition})"
        ),
        args,
    )?;
    Ok(conn.execute(&format!("DELETE FROM conversations WHERE {condition}"), args)?)
}

/// Settings › Data › "Delete conversation history".
pub fn delete_all(conn: &Connection) -> AppResult<usize> {
    delete_where(conn, "1 = 1", &[])
}

/// Delete conversations older than `keep_days` (0 = keep forever). A pending review is kept
/// until it is `PENDING_REVIEW_DAYS` old.
pub fn retention(conn: &Connection, now: DateTime<Utc>, keep_days: u32) -> AppResult<usize> {
    if keep_days == 0 {
        return Ok(0);
    }
    let before = fmt_ts(now - Duration::days(keep_days as i64));
    let pending_before = fmt_ts(now - Duration::days(PENDING_REVIEW_DAYS));
    let n = delete_where(
        conn,
        "started_at < ?1 AND (review_status != 'pending' OR started_at < ?2)",
        &[&before, &pending_before],
    )?;
    if n > 0 {
        tracing::info!(deleted = n, "conversation retention");
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::parse_ts;
    use crate::db::Db;
    use serde_json::json;

    fn count(c: &Connection, table: &str) -> i64 {
        c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[tokio::test]
    async fn turns_observations_and_listing() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            c.execute(
                "INSERT INTO articles(id, url, normalized_url, title, title_key, source_name, discovered_at)
                 VALUES (1, 'u', 'n', 'Local AI', 't', 's', 'x')",
                [],
            )?;
            let id = create(c, Some(1), &json!({"level": 2}), "2026-09-30T08:00:00Z")?;
            let t1 = add_turn(c, id, "tutor", "Hello.", None, "2026-09-30T08:00:01Z")?;
            let t2 = add_turn(
                c,
                id,
                "user",
                "Hi",
                Some(&json!({"local": true})),
                "2026-09-30T08:00:05Z",
            )?;
            assert_eq!((t1.seq, t2.seq), (1, 2));
            let o = add_observation(
                c,
                id,
                "grammar",
                "I deployed it.",
                Some(&json!({"original": "I deploy it."})),
                "t",
            )?;
            add_observation(c, id, "unknown_word", "latency", None, "t")?;
            set_observation_saved(c, o, 99).unwrap_err(); // no such vocab item → FK error
            let obs = observations(c, id)?;
            assert_eq!(obs.len(), 2);
            assert_eq!(obs[0].detail.as_ref().unwrap()["original"], "I deploy it.");
            let ts = turns(c, id)?;
            assert_eq!(ts[1].meta, Some(json!({"local": true})));
            assert_eq!(close_dangling(c)?, 1);
            let conv = get(c, id)?;
            assert_eq!(
                conv.ended_at.as_deref(),
                Some("2026-09-30T08:00:05Z"),
                "ended at the last turn"
            );
            assert_eq!(
                (
                    conv.article_title.as_deref(),
                    conv.corrections,
                    conv.review_status.as_str()
                ),
                (Some("Local AI"), 1, "pending")
            );
            end(c, id, "2026-09-30T08:10:00Z", 95)?;
            set_review_status(c, id, "done")?;
            let l = list(c, 10)?;
            assert_eq!(
                (l.len(), l[0].user_speaking_seconds, l[0].review_status.as_str()),
                (1, 95, "done")
            );
            assert!(matches!(get(c, 42), Err(AppError::NotFound(_))));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn retention_rules() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            let mk = |c: &Connection, started: &str, status: &str| -> AppResult<i64> {
                let id = create(c, None, &json!({}), started)?;
                set_review_status(c, id, status)?;
                add_turn(c, id, "user", "x", None, started)?;
                Ok(id)
            };
            let old_done = mk(c, "2026-06-01T00:00:00Z", "done")?; // 121 days → delete
            let old_pending = mk(c, "2026-06-01T00:00:00Z", "pending")?; // pending, < 180 days → keep
            let ancient_pending = mk(c, "2026-03-01T00:00:00Z", "pending")?; // > 180 days → delete
            let recent = mk(c, "2026-09-01T00:00:00Z", "skipped")?; // 29 days → keep
            c.execute(
                "INSERT INTO vocab_items(id, kind, text, text_key, created_at, updated_at) VALUES (1, 'word', 'x', 'x', 't', 't')",
                [],
            )?;
            for conv in [old_done, recent] {
                c.execute(
                    "INSERT INTO vocab_contexts(item_id, conversation_id, sentence, created_at) VALUES (1, ?1, 's', 't')",
                    [conv],
                )?;
            }
            let now = parse_ts("2026-09-30T00:00:00Z").unwrap();
            assert_eq!(retention(c, now, 0)?, 0, "0 = keep forever");
            assert_eq!(retention(c, now, 90)?, 2);
            let left: Vec<i64> = list(c, 10)?.into_iter().map(|x| x.id).collect();
            assert_eq!(left, vec![recent, old_pending]);
            let _ = ancient_pending;
            assert_eq!(count(c, "conversation_turns"), 2, "turns cascade");
            assert_eq!(count(c, "vocab_contexts"), 1, "the deleted conversation's context is gone");
            assert_eq!(count(c, "vocab_items"), 1, "the Word Book item stays");
            assert_eq!(delete_all(c)?, 2);
            assert_eq!(count(c, "vocab_contexts"), 0);
            Ok(())
        })
        .await
        .unwrap();
    }
}
