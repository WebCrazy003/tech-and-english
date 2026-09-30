//! Word Book tables (P4): items, contexts, reviews and quiz sessions. Rules live in `learning::vocab`.

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::{json_list, parse_json_list};
use crate::error::{AppError, AppResult};

pub const KINDS: &[&str] = &["word", "phrase", "sentence", "term", "pronunciation", "correction"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VocabItem {
    pub id: i64,
    pub kind: String,
    pub text: String,
    pub text_key: String,
    pub meaning_simple: Option<String>,
    pub meaning_b1: Option<String>,
    pub part_of_speech: Option<String>,
    pub ipa: Option<String>,
    pub syllables: Option<String>,
    pub examples: Vec<String>,
    pub collocations: Vec<String>,
    pub notes: Option<String>,
    /// "new" | "learning" | "known"
    pub status: String,
    pub srs_state: Option<String>,
    pub stability: Option<f64>,
    pub difficulty: Option<f64>,
    pub due_at: Option<String>,
    pub last_reviewed_at: Option<String>,
    /// "forgot" | "unsure" | "remember"
    pub last_grade: Option<String>,
    pub review_count: u32,
    pub remember_count: u32,
    pub unsure_count: u32,
    pub forgot_count: u32,
    pub created_at: String,
    pub updated_at: String,
}

impl VocabItem {
    pub fn has_meaning(&self) -> bool {
        self.meaning_simple.is_some() || self.meaning_b1.is_some()
    }
}

const COLS: &str = "id, kind, text, text_key, meaning_simple, meaning_b1, part_of_speech, ipa, syllables, examples,
    collocations, notes, status, srs_state, stability, difficulty, due_at, last_reviewed_at, last_grade, review_count,
    remember_count, unsure_count, forgot_count, created_at, updated_at";

fn from_row(r: &Row) -> rusqlite::Result<VocabItem> {
    Ok(VocabItem {
        id: r.get(0)?,
        kind: r.get(1)?,
        text: r.get(2)?,
        text_key: r.get(3)?,
        meaning_simple: r.get(4)?,
        meaning_b1: r.get(5)?,
        part_of_speech: r.get(6)?,
        ipa: r.get(7)?,
        syllables: r.get(8)?,
        examples: parse_json_list(&r.get::<_, String>(9)?),
        collocations: parse_json_list(&r.get::<_, String>(10)?),
        notes: r.get(11)?,
        status: r.get(12)?,
        srs_state: r.get(13)?,
        stability: r.get(14)?,
        difficulty: r.get(15)?,
        due_at: r.get(16)?,
        last_reviewed_at: r.get(17)?,
        last_grade: r.get(18)?,
        review_count: r.get(19)?,
        remember_count: r.get(20)?,
        unsure_count: r.get(21)?,
        forgot_count: r.get(22)?,
        created_at: r.get(23)?,
        updated_at: r.get(24)?,
    })
}

pub fn get(conn: &Connection, id: i64) -> AppResult<VocabItem> {
    conn.query_row(&format!("SELECT {COLS} FROM vocab_items WHERE id = ?1"), [id], from_row)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("word {id}")))
}

pub fn find(conn: &Connection, kind: &str, text_key: &str) -> AppResult<Option<VocabItem>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM vocab_items WHERE kind = ?1 AND text_key = ?2"),
            [kind, text_key],
            from_row,
        )
        .optional()?)
}

/// Fields written on insert and on edits.
#[derive(Debug, Clone, Default)]
pub struct Fields {
    pub meaning_simple: Option<String>,
    pub meaning_b1: Option<String>,
    pub part_of_speech: Option<String>,
    pub ipa: Option<String>,
    pub syllables: Option<String>,
    pub examples: Vec<String>,
    pub collocations: Vec<String>,
    pub notes: Option<String>,
}

pub fn insert(conn: &Connection, kind: &str, text: &str, key: &str, f: &Fields, now: &str) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO vocab_items(kind, text, text_key, meaning_simple, meaning_b1, part_of_speech, ipa, syllables,
           examples, collocations, notes, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12)",
        params![
            kind,
            text,
            key,
            f.meaning_simple,
            f.meaning_b1,
            f.part_of_speech,
            f.ipa,
            f.syllables,
            json_list(&f.examples),
            json_list(&f.collocations),
            f.notes,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Fill only the fields that are still empty (merging a re-added item, or the auto-fill job).
pub fn fill_empty(conn: &Connection, id: i64, f: &Fields, now: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE vocab_items SET
           meaning_simple = COALESCE(meaning_simple, ?1), meaning_b1 = COALESCE(meaning_b1, ?2),
           part_of_speech = COALESCE(part_of_speech, ?3), ipa = COALESCE(ipa, ?4), syllables = COALESCE(syllables, ?5),
           examples = CASE WHEN examples = '[]' THEN ?6 ELSE examples END,
           collocations = CASE WHEN collocations = '[]' THEN ?7 ELSE collocations END,
           notes = COALESCE(notes, ?8), updated_at = ?9
         WHERE id = ?10",
        params![
            f.meaning_simple,
            f.meaning_b1,
            f.part_of_speech,
            f.ipa,
            f.syllables,
            json_list(&f.examples),
            json_list(&f.collocations),
            f.notes,
            now,
            id
        ],
    )?;
    Ok(())
}

/// Replace text and fields (an edit in the Word Book).
pub fn update(conn: &Connection, id: i64, kind: &str, text: &str, key: &str, f: &Fields, now: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE vocab_items SET kind=?1, text=?2, text_key=?3, meaning_simple=?4, meaning_b1=?5, part_of_speech=?6,
           ipa=?7, syllables=?8, examples=?9, collocations=?10, notes=?11, updated_at=?12 WHERE id=?13",
        params![
            kind,
            text,
            key,
            f.meaning_simple,
            f.meaning_b1,
            f.part_of_speech,
            f.ipa,
            f.syllables,
            json_list(&f.examples),
            json_list(&f.collocations),
            f.notes,
            now,
            id
        ],
    )?;
    Ok(())
}

pub fn set_status(conn: &Connection, id: i64, status: &str, now: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE vocab_items SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status, now, id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM vocab_items WHERE id = ?1", [id])?;
    Ok(())
}

// ---------------------------------------------------------------- contexts

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub id: i64,
    pub sentence: Option<String>,
    pub article_id: Option<i64>,
    pub article_title: Option<String>,
    pub article_url: Option<String>,
    pub conversation_id: Option<i64>,
    pub created_at: String,
}

pub fn has_context(conn: &Connection, item_id: i64, sentence: &str) -> AppResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM vocab_contexts WHERE item_id = ?1 AND sentence = ?2)",
        params![item_id, sentence],
        |r| r.get(0),
    )?)
}

pub fn add_context(
    conn: &Connection,
    item_id: i64,
    sentence: Option<&str>,
    article_id: Option<i64>,
    conversation_id: Option<i64>,
    now: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO vocab_contexts(item_id, article_id, conversation_id, sentence, created_at) VALUES (?1,?2,?3,?4,?5)",
        params![item_id, article_id, conversation_id, sentence, now],
    )?;
    Ok(())
}

/// Newest first.
pub fn contexts(conn: &Connection, item_id: i64) -> AppResult<Vec<Context>> {
    let mut st = conn.prepare(
        "SELECT c.id, c.sentence, c.article_id, a.title, a.url, c.conversation_id, c.created_at
         FROM vocab_contexts c LEFT JOIN articles a ON a.id = c.article_id
         WHERE c.item_id = ?1 ORDER BY c.created_at DESC, c.id DESC",
    )?;
    let rows = st
        .query_map([item_id], |r| {
            Ok(Context {
                id: r.get(0)?,
                sentence: r.get(1)?,
                article_id: r.get(2)?,
                article_title: r.get(3)?,
                article_url: r.get(4)?,
                conversation_id: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------- listing

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VocabFilter {
    pub query: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub due_only: bool,
    pub article_id: Option<i64>,
    /// Only items that have no meaning yet.
    pub pending_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabPage {
    pub items: Vec<VocabItem>,
    pub next_cursor: Option<i64>,
    pub total: i64,
}

/// Newest first, paginated by id.
pub fn list(conn: &Connection, f: &VocabFilter, now: &str, cursor: Option<i64>, limit: u32) -> AppResult<VocabPage> {
    let limit = limit.clamp(1, 500);
    let mut wh: Vec<String> = vec!["1 = 1".into()];
    let mut args: Vec<SqlValue> = Vec::new();
    if let Some(q) = f.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        let esc = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        wh.push(
            "(text LIKE ?1 ESCAPE '\\' OR meaning_simple LIKE ?1 ESCAPE '\\' OR meaning_b1 LIKE ?1 ESCAPE '\\')"
                .replace("?1", "?"),
        );
        let pat = format!("%{esc}%");
        args.extend([pat.clone().into(), pat.clone().into(), pat.into()]);
    }
    if let Some(k) = &f.kind {
        wh.push("kind = ?".into());
        args.push(k.clone().into());
    }
    if let Some(s) = &f.status {
        wh.push("status = ?".into());
        args.push(s.clone().into());
    }
    if f.due_only {
        wh.push("due_at <= ?".into());
        args.push(now.to_string().into());
    }
    if let Some(a) = f.article_id {
        wh.push("EXISTS (SELECT 1 FROM vocab_contexts c WHERE c.item_id = vocab_items.id AND c.article_id = ?)".into());
        args.push(a.into());
    }
    if f.pending_only {
        wh.push("meaning_simple IS NULL AND meaning_b1 IS NULL".into());
    }
    let where_sql = wh.join(" AND ");
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) FROM vocab_items WHERE {where_sql}"),
        params_from_iter(args.iter()),
        |r| r.get(0),
    )?;
    let mut page_args = args.clone();
    let mut page_where = where_sql.clone();
    if let Some(c) = cursor {
        page_where.push_str(" AND id < ?");
        page_args.push(c.into());
    }
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM vocab_items WHERE {page_where} ORDER BY id DESC LIMIT {}",
        limit + 1
    ))?;
    let mut items = st
        .query_map(params_from_iter(page_args), from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if items.len() > limit as usize {
        items.truncate(limit as usize);
        items.last().map(|i| i.id)
    } else {
        None
    };
    Ok(VocabPage {
        items,
        next_cursor,
        total,
    })
}

pub fn all(conn: &Connection) -> AppResult<Vec<VocabItem>> {
    let mut st = conn.prepare(&format!("SELECT {COLS} FROM vocab_items ORDER BY id"))?;
    let rows = st.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn keys(conn: &Connection) -> AppResult<Vec<String>> {
    let mut st = conn.prepare("SELECT DISTINCT text_key FROM vocab_items")?;
    let rows = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// (items with a meaning that are due now, items with a meaning never reviewed).
pub fn due_counts(conn: &Connection, now: &str) -> AppResult<(u32, u32)> {
    Ok(conn.query_row(
        "SELECT
           COALESCE(SUM(CASE WHEN review_count > 0 AND due_at <= ?1 THEN 1 ELSE 0 END), 0),
           COALESCE(SUM(CASE WHEN review_count = 0 THEN 1 ELSE 0 END), 0)
         FROM vocab_items WHERE meaning_simple IS NOT NULL OR meaning_b1 IS NOT NULL",
        [now],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// Items with no meaning, oldest first (for the auto-fill job).
pub fn pending(conn: &Connection, limit: u32) -> AppResult<Vec<VocabItem>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM vocab_items WHERE meaning_simple IS NULL AND meaning_b1 IS NULL ORDER BY id LIMIT ?1"
    ))?;
    let rows = st.query_map([limit], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------- reviews & quiz sessions

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub grade: String,
    pub reviewed_at: String,
    pub due_after: Option<String>,
}

/// SRS fields after a review.
pub struct ReviewUpdate<'a> {
    pub grade: &'a str,
    pub status: &'a str,
    pub srs_state: &'a str,
    pub stability: f64,
    pub difficulty: f64,
    pub due_at: &'a str,
    pub now: &'a str,
}

pub fn apply_review(conn: &Connection, id: i64, u: &ReviewUpdate, session: Option<i64>) -> AppResult<()> {
    conn.execute(
        "UPDATE vocab_items SET status = ?1, srs_state = ?2, stability = ?3, difficulty = ?4, due_at = ?5,
           last_reviewed_at = ?6, last_grade = ?7, review_count = review_count + 1,
           remember_count = remember_count + (?7 = 'remember'), unsure_count = unsure_count + (?7 = 'unsure'),
           forgot_count = forgot_count + (?7 = 'forgot'), updated_at = ?6
         WHERE id = ?8",
        params![
            u.status,
            u.srs_state,
            u.stability,
            u.difficulty,
            u.due_at,
            u.now,
            u.grade,
            id
        ],
    )?;
    conn.execute(
        "INSERT INTO vocab_reviews(item_id, quiz_session_id, grade, reviewed_at, stability_after, difficulty_after, due_after)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![id, session, u.grade, u.now, u.stability, u.difficulty, u.due_at],
    )?;
    if let Some(s) = session {
        conn.execute(
            "UPDATE quiz_sessions SET remember = remember + (?1 = 'remember'), unsure = unsure + (?1 = 'unsure'),
               forgot = forgot + (?1 = 'forgot') WHERE id = ?2",
            params![u.grade, s],
        )?;
    }
    Ok(())
}

pub fn reviews(conn: &Connection, item_id: i64, limit: u32) -> AppResult<Vec<Review>> {
    let mut st = conn.prepare(
        "SELECT grade, reviewed_at, due_after FROM vocab_reviews WHERE item_id = ?1 ORDER BY reviewed_at DESC, id DESC LIMIT ?2",
    )?;
    let rows = st
        .query_map(params![item_id, limit], |r| {
            Ok(Review {
                grade: r.get(0)?,
                reviewed_at: r.get(1)?,
                due_after: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn graded_in_session(conn: &Connection, session: i64, item_id: i64) -> AppResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM vocab_reviews WHERE quiz_session_id = ?1 AND item_id = ?2)",
        params![session, item_id],
        |r| r.get(0),
    )?)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub item_count: u32,
    pub remember: u32,
    pub unsure: u32,
    pub forgot: u32,
    pub score: Option<f64>,
}

fn session_row(r: &Row) -> rusqlite::Result<Session> {
    Ok(Session {
        id: r.get(0)?,
        started_at: r.get(1)?,
        finished_at: r.get(2)?,
        item_count: r.get(3)?,
        remember: r.get(4)?,
        unsure: r.get(5)?,
        forgot: r.get(6)?,
        score: r.get(7)?,
    })
}

const SESSION_COLS: &str = "id, started_at, finished_at, item_count, remember, unsure, forgot, score";

pub fn start_session(conn: &Connection, item_count: u32, now: &str) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO quiz_sessions(started_at, item_count) VALUES (?1, ?2)",
        params![now, item_count],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn session(conn: &Connection, id: i64) -> AppResult<Session> {
    conn.query_row(
        &format!("SELECT {SESSION_COLS} FROM quiz_sessions WHERE id = ?1"),
        [id],
        session_row,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("quiz {id}")))
}

pub fn finish_session(conn: &Connection, id: i64, score: f64, now: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE quiz_sessions SET finished_at = COALESCE(finished_at, ?1), score = ?2 WHERE id = ?3",
        params![now, score, id],
    )?;
    Ok(())
}

/// Finished sessions, newest first.
pub fn history(conn: &Connection, limit: u32) -> AppResult<Vec<Session>> {
    let mut st = conn.prepare(&format!(
        "SELECT {SESSION_COLS} FROM quiz_sessions WHERE finished_at IS NOT NULL ORDER BY finished_at DESC, id DESC LIMIT ?1"
    ))?;
    let rows = st.query_map([limit], session_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Items graded forgot/unsure in a session (the "missed" list).
pub fn missed_in_session(conn: &Connection, session: i64) -> AppResult<Vec<VocabItem>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM vocab_items WHERE id IN
           (SELECT item_id FROM vocab_reviews WHERE quiz_session_id = ?1 AND grade IN ('forgot','unsure'))
         ORDER BY id"
    ))?;
    let rows = st.query_map([session], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
