//! Cached AI outputs (`article_derivatives`) and the per-article chat (`article_chats`).

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::AppResult;

#[derive(Debug, Clone, PartialEq)]
pub struct Derivative {
    pub text: String,
    pub model_id: String,
    pub prompt_version: String,
}

pub fn get_derivative(conn: &Connection, article_id: i64, kind: &str) -> AppResult<Option<Derivative>> {
    Ok(conn
        .query_row(
            "SELECT text, model_id, prompt_version FROM article_derivatives WHERE article_id = ?1 AND kind = ?2",
            params![article_id, kind],
            |r| {
                Ok(Derivative {
                    text: r.get(0)?,
                    model_id: r.get(1)?,
                    prompt_version: r.get(2)?,
                })
            },
        )
        .optional()?)
}

pub fn put_derivative(conn: &Connection, article_id: i64, kind: &str, d: &Derivative, now: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO article_derivatives(article_id, kind, model_id, prompt_version, text, created_at)
         VALUES (?1,?2,?3,?4,?5,?6)
         ON CONFLICT(article_id, kind) DO UPDATE SET model_id = excluded.model_id,
           prompt_version = excluded.prompt_version, text = excluded.text, created_at = excluded.created_at",
        params![article_id, kind, d.model_id, d.prompt_version, d.text, now],
    )?;
    Ok(())
}

pub fn delete_derivative(conn: &Connection, article_id: i64, kind: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM article_derivatives WHERE article_id = ?1 AND kind = ?2",
        params![article_id, kind],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: i64,
    /// "user" or "assistant"
    pub role: String,
    pub content: String,
    pub created_at: String,
}

fn chat_from_row(r: &rusqlite::Row) -> rusqlite::Result<ChatMessage> {
    Ok(ChatMessage {
        id: r.get(0)?,
        role: r.get(1)?,
        content: r.get(2)?,
        created_at: r.get(3)?,
    })
}

pub fn list_chat(conn: &Connection, article_id: i64) -> AppResult<Vec<ChatMessage>> {
    let mut st =
        conn.prepare("SELECT id, role, content, created_at FROM article_chats WHERE article_id = ?1 ORDER BY id")?;
    let rows = st
        .query_map([article_id], chat_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The last `n` messages, oldest first (what is sent to the model as history).
pub fn recent_chat(conn: &Connection, article_id: i64, n: u32) -> AppResult<Vec<ChatMessage>> {
    let mut st = conn.prepare(
        "SELECT id, role, content, created_at FROM article_chats WHERE article_id = ?1 ORDER BY id DESC LIMIT ?2",
    )?;
    let mut rows = st
        .query_map(params![article_id, n], chat_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.reverse();
    Ok(rows)
}

pub fn add_chat(
    conn: &Connection,
    article_id: i64,
    role: &str,
    content: &str,
    model_id: Option<&str>,
    now: &str,
) -> AppResult<ChatMessage> {
    conn.execute(
        "INSERT INTO article_chats(article_id, role, content, model_id, created_at) VALUES (?1,?2,?3,?4,?5)",
        params![article_id, role, content, model_id, now],
    )?;
    Ok(ChatMessage {
        id: conn.last_insert_rowid(),
        role: role.to_string(),
        content: content.to_string(),
        created_at: now.to_string(),
    })
}

pub fn clear_chat(conn: &Connection, article_id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM article_chats WHERE article_id = ?1", [article_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn article(c: &Connection) -> i64 {
        c.execute(
            "INSERT INTO articles(url, normalized_url, title, title_key, source_name, discovered_at)
             VALUES ('u','n','t','t','s','2026-09-29T00:00:00Z')",
            [],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    #[tokio::test]
    async fn derivatives_upsert_and_delete() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            let a = article(c);
            let d = Derivative {
                text: "one".into(),
                model_id: "m".into(),
                prompt_version: "1".into(),
            };
            put_derivative(c, a, "summary_b1", &d, "t")?;
            put_derivative(
                c,
                a,
                "summary_b1",
                &Derivative {
                    text: "two".into(),
                    ..d
                },
                "t",
            )?;
            assert_eq!(get_derivative(c, a, "summary_b1")?.unwrap().text, "two");
            delete_derivative(c, a, "summary_b1")?;
            assert!(get_derivative(c, a, "summary_b1")?.is_none());
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn chat_history_order_and_limit() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            let a = article(c);
            for i in 0..10 {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                add_chat(c, a, role, &format!("m{i}"), None, "t")?;
            }
            let recent = recent_chat(c, a, 8)?;
            assert_eq!(recent.len(), 8);
            assert_eq!(recent.first().unwrap().content, "m2");
            assert_eq!(recent.last().unwrap().content, "m9");
            assert_eq!(list_chat(c, a)?.len(), 10);
            clear_chat(c, a)?;
            assert!(list_chat(c, a)?.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    }
}
