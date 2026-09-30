use std::collections::{HashMap, HashSet};

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::{ids_to_values, placeholders};
use crate::error::{AppError, AppResult};
use crate::news::model::{HnStats, ScoreBreakdown};

pub struct NewArticle {
    pub url: String,
    pub normalized_url: String,
    pub title: String,
    pub title_key: String,
    pub source_name: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub published_at: Option<String>,
    pub discovered_at: String,
    pub hn: Option<HnStats>,
}

pub fn insert(conn: &Connection, a: &NewArticle) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO articles(url, normalized_url, title, title_key, source_name, author, description,
           published_at, discovered_at, hn_id, hn_points, hn_comments, hn_checked_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12, CASE WHEN ?10 IS NULL THEN NULL ELSE ?9 END)",
        params![
            a.url,
            a.normalized_url,
            a.title,
            a.title_key,
            a.source_name,
            a.author,
            a.description,
            a.published_at,
            a.discovered_at,
            a.hn.as_ref().map(|h| h.id),
            a.hn.as_ref().map(|h| h.points),
            a.hn.as_ref().map(|h| h.comments),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn find_id_by_normalized_url(conn: &Connection, n: &str) -> AppResult<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM articles WHERE normalized_url = ?1", [n], |r| r.get(0))
        .optional()?)
}

pub fn find_id_by_hn_id(conn: &Connection, hn_id: i64) -> AppResult<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM articles WHERE hn_id = ?1", [hn_id], |r| r.get(0))
        .optional()?)
}

/// (id, title_key) of articles discovered at or after `since`.
pub fn recent_title_keys(conn: &Connection, since: &str) -> AppResult<Vec<(i64, String)>> {
    let mut st = conn.prepare("SELECT id, title_key FROM articles WHERE discovered_at >= ?1")?;
    let rows = st
        .query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Fill empty fields of an existing article from a duplicate, and refresh HN stats.
pub fn merge(
    conn: &Connection,
    id: i64,
    description: Option<&str>,
    author: Option<&str>,
    published_at: Option<&str>,
    hn: Option<&HnStats>,
    now: &str,
) -> AppResult<()> {
    conn.execute(
        "UPDATE articles SET description = COALESCE(description, ?1), author = COALESCE(author, ?2),
           published_at = COALESCE(published_at, ?3) WHERE id = ?4",
        params![description, author, published_at, id],
    )?;
    if let Some(h) = hn {
        conn.execute(
            "UPDATE articles SET
               hn_id = CASE WHEN hn_id IS NULL AND NOT EXISTS (SELECT 1 FROM articles WHERE hn_id = ?1) THEN ?1 ELSE hn_id END,
               hn_points = ?2, hn_comments = ?3, hn_checked_at = ?4
             WHERE id = ?5",
            params![h.id, h.points, h.comments, now, id],
        )?;
    }
    Ok(())
}

pub fn add_source(conn: &Connection, article_id: i64, feed_id: i64, item_url: &str, now: &str) -> AppResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO article_sources(article_id, feed_id, item_url, seen_at) VALUES (?1,?2,?3,?4)",
        params![article_id, feed_id, item_url, now],
    )?;
    Ok(())
}

pub fn has_rss_source(conn: &Connection, article_id: i64) -> AppResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM article_sources s JOIN feeds f ON f.id = s.feed_id
                       WHERE s.article_id = ?1 AND f.kind = 'rss')",
        [article_id],
        |r| r.get(0),
    )?)
}

pub fn set_source_name(conn: &Connection, id: i64, name: &str) -> AppResult<()> {
    conn.execute("UPDATE articles SET source_name = ?1 WHERE id = ?2", params![name, id])?;
    Ok(())
}

/// Title and description, used for topic matching.
/// Title, and description plus the first 500 chars of the body (if extracted), for topic matching.
pub fn match_text(conn: &Connection, id: i64) -> AppResult<(String, Option<String>)> {
    let (title, desc, body): (String, Option<String>, Option<String>) = conn.query_row(
        "SELECT title, description, substr(body_text, 1, 500) FROM articles WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let text = match (desc, body) {
        (Some(d), Some(b)) => Some(format!("{d} {b}")),
        (d, b) => d.or(b),
    };
    Ok((title, text))
}

/// Replace the topic matches of an article. The primary topic is the most relevant one.
pub fn set_topics(conn: &Connection, id: i64, matches: &[(i64, f64)]) -> AppResult<()> {
    conn.execute("DELETE FROM article_topics WHERE article_id = ?1", [id])?;
    for (topic_id, rel) in matches {
        conn.execute(
            "INSERT INTO article_topics(article_id, topic_id, relevance) VALUES (?1,?2,?3)",
            params![id, topic_id, rel],
        )?;
    }
    let primary = matches.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|m| m.0);
    conn.execute(
        "UPDATE articles SET primary_topic_id = ?1 WHERE id = ?2",
        params![primary, id],
    )?;
    Ok(())
}

pub fn ids_discovered_since(conn: &Connection, since: &str) -> AppResult<Vec<i64>> {
    let mut st = conn.prepare("SELECT id FROM articles WHERE discovered_at >= ?1")?;
    let rows = st.query_map([since], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// HN ids whose stats were refreshed at or after `since` (no need to fetch them again).
pub fn hn_fresh_ids(conn: &Connection, since: &str) -> AppResult<HashSet<i64>> {
    let mut st = conn.prepare("SELECT hn_id FROM articles WHERE hn_id IS NOT NULL AND hn_checked_at >= ?1")?;
    let rows = st
        .query_map([since], |r| r.get(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------- scoring

#[derive(Debug, Clone)]
pub struct ScoringRow {
    pub id: i64,
    pub title_key: String,
    pub published_at: Option<String>,
    pub discovered_at: String,
    pub primary_topic_id: Option<i64>,
    pub relevance: f64,
    pub hn_points: Option<i64>,
    pub hn_comments: Option<i64>,
    pub source_weight: f64,
    pub source_ids: Vec<i64>,
}

pub fn scoring_rows(conn: &Connection, since: &str) -> AppResult<Vec<ScoringRow>> {
    let mut st = conn.prepare(
        "SELECT a.id, a.title_key, a.published_at, a.discovered_at, a.primary_topic_id, a.hn_points, a.hn_comments,
           COALESCE((SELECT MAX(relevance) FROM article_topics WHERE article_id = a.id), 0),
           COALESCE((SELECT MAX(f.source_weight) FROM article_sources s JOIN feeds f ON f.id = s.feed_id
                     WHERE s.article_id = a.id), 0.5),
           (SELECT group_concat(feed_id) FROM article_sources WHERE article_id = a.id)
         FROM articles a WHERE a.discovered_at >= ?1 AND a.hidden = 0",
    )?;
    let rows = st
        .query_map([since], |r| {
            let ids: Option<String> = r.get(9)?;
            Ok(ScoringRow {
                id: r.get(0)?,
                title_key: r.get(1)?,
                published_at: r.get(2)?,
                discovered_at: r.get(3)?,
                primary_topic_id: r.get(4)?,
                hn_points: r.get(5)?,
                hn_comments: r.get(6)?,
                relevance: r.get(7)?,
                source_weight: r.get(8)?,
                source_ids: ids
                    .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
                    .unwrap_or_default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn update_score(conn: &Connection, id: i64, b: &ScoreBreakdown, now: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE articles SET score = ?1, score_breakdown = ?2, scored_at = ?3 WHERE id = ?4",
        params![b.total, serde_json::to_string(b)?, now, id],
    )?;
    Ok(())
}

/// Title keys of articles the user opened, or that were picked, since `since` (for novelty).
pub fn engaged_title_keys(conn: &Connection, since: &str) -> AppResult<Vec<(i64, String)>> {
    let mut st = conn.prepare(
        "SELECT id, title_key FROM articles WHERE
           id IN (SELECT article_id FROM article_interactions WHERE kind = 'opened' AND created_at >= ?1)
           OR id IN (SELECT article_id FROM daily_picks WHERE created_at >= ?1)",
    )?;
    let rows = st
        .query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------- listing

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArticleListItem {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub source_name: String,
    pub description: Option<String>,
    pub published_at: Option<String>,
    pub discovered_at: String,
    pub primary_topic: Option<String>,
    pub topics: Vec<String>,
    pub score: Option<f64>,
    pub breakdown: Option<ScoreBreakdown>,
    pub hn_id: Option<i64>,
    pub hn_points: Option<i64>,
    pub hn_comments: Option<i64>,
    pub read_status: String,
    pub saved: bool,
    pub hidden: bool,
    /// "none" | "ok" | "failed" | "paywalled"
    pub body_status: String,
    /// "easy" | "medium" | "hard", once the body is extracted
    pub difficulty: Option<String>,
    pub reading_minutes: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArticleFilter {
    pub topic_id: Option<i64>,
    pub feed_id: Option<i64>,
    pub unread_only: bool,
    pub saved_only: bool,
    pub min_score: Option<f64>,
    pub query: Option<String>,
    /// Only articles discovered at or after this time (RFC 3339).
    pub since: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

const ITEM_SELECT: &str = "SELECT a.id, a.url, a.title, a.source_name, a.description, a.published_at, a.discovered_at,
    t.name, a.score, a.score_breakdown, a.hn_id, a.hn_points, a.hn_comments, a.read_status, a.saved, a.hidden,
    a.body_status, a.difficulty, a.word_count
    FROM articles a LEFT JOIN topics t ON t.id = a.primary_topic_id";

fn item_from_row(r: &Row) -> rusqlite::Result<ArticleListItem> {
    let breakdown: Option<String> = r.get(9)?;
    Ok(ArticleListItem {
        id: r.get(0)?,
        url: r.get(1)?,
        title: r.get(2)?,
        source_name: r.get(3)?,
        description: r.get(4)?,
        published_at: r.get(5)?,
        discovered_at: r.get(6)?,
        primary_topic: r.get(7)?,
        topics: Vec::new(),
        score: r.get(8)?,
        breakdown: breakdown.and_then(|s| serde_json::from_str(&s).ok()),
        hn_id: r.get(10)?,
        hn_points: r.get(11)?,
        hn_comments: r.get(12)?,
        read_status: r.get(13)?,
        saved: r.get(14)?,
        hidden: r.get(15)?,
        body_status: r.get(16)?,
        difficulty: r.get(17)?,
        reading_minutes: r
            .get::<_, Option<u32>>(18)?
            .map(crate::news::difficulty::reading_minutes),
    })
}

fn attach_topics(conn: &Connection, items: &mut [ArticleListItem]) -> AppResult<()> {
    if items.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = items.iter().map(|i| i.id).collect();
    let sql = format!(
        "SELECT at.article_id, t.name FROM article_topics at JOIN topics t ON t.id = at.topic_id
         WHERE at.article_id IN ({}) ORDER BY at.relevance DESC",
        placeholders(ids.len())
    );
    let mut st = conn.prepare(&sql)?;
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    let rows = st.query_map(params_from_iter(ids_to_values(&ids)), |r| {
        Ok((r.get::<_, i64>(0)?, r.get(1)?))
    })?;
    for row in rows {
        let (id, name) = row?;
        map.entry(id).or_default().push(name);
    }
    for it in items.iter_mut() {
        it.topics = map.remove(&it.id).unwrap_or_default();
    }
    Ok(())
}

pub fn get_item(conn: &Connection, id: i64) -> AppResult<ArticleListItem> {
    let mut item = conn
        .query_row(&format!("{ITEM_SELECT} WHERE a.id = ?1"), [id], item_from_row)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("article {id}")))?;
    attach_topics(conn, std::slice::from_mut(&mut item))?;
    Ok(item)
}

fn parse_cursor(c: &str) -> Option<(f64, i64)> {
    let (s, id) = c.split_once(':')?;
    Some((s.parse().ok()?, id.parse().ok()?))
}

/// Visible articles sorted by score (desc), paginated with a "score:id" cursor.
pub fn list_items(
    conn: &Connection,
    f: &ArticleFilter,
    cursor: Option<&str>,
    limit: u32,
) -> AppResult<Page<ArticleListItem>> {
    let limit = limit.clamp(1, 200);
    let mut wh = vec!["a.hidden = 0".to_string()];
    let mut args: Vec<SqlValue> = Vec::new();
    if let Some(t) = f.topic_id {
        wh.push("EXISTS (SELECT 1 FROM article_topics x WHERE x.article_id = a.id AND x.topic_id = ?)".into());
        args.push(t.into());
    }
    if let Some(fid) = f.feed_id {
        wh.push("EXISTS (SELECT 1 FROM article_sources x WHERE x.article_id = a.id AND x.feed_id = ?)".into());
        args.push(fid.into());
    }
    if f.unread_only {
        wh.push("a.read_status = 'unread'".into());
    }
    if f.saved_only {
        wh.push("a.saved = 1".into());
    }
    if let Some(m) = f.min_score {
        wh.push("a.score >= ?".into());
        args.push(m.into());
    }
    if let Some(q) = f.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        let esc = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        wh.push("a.title LIKE ? ESCAPE '\\'".into());
        args.push(format!("%{esc}%").into());
    }
    if let Some(s) = &f.since {
        wh.push("a.discovered_at >= ?".into());
        args.push(s.clone().into());
    }
    if let Some((s, id)) = cursor.and_then(parse_cursor) {
        wh.push("(COALESCE(a.score, -1) < ? OR (COALESCE(a.score, -1) = ? AND a.id < ?))".into());
        args.push(s.into());
        args.push(s.into());
        args.push(id.into());
    }
    let sql = format!(
        "{ITEM_SELECT} WHERE {} ORDER BY COALESCE(a.score, -1) DESC, a.id DESC LIMIT {}",
        wh.join(" AND "),
        limit + 1
    );
    let mut st = conn.prepare(&sql)?;
    let mut items = st
        .query_map(params_from_iter(args), item_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if items.len() > limit as usize {
        items.truncate(limit as usize);
        items.last().map(|l| format!("{}:{}", l.score.unwrap_or(-1.0), l.id))
    } else {
        None
    };
    attach_topics(conn, &mut items)?;
    Ok(Page { items, next_cursor })
}

// ---------------------------------------------------------------- state changes

pub fn mark_opened(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE articles SET read_status = 'opened' WHERE id = ?1 AND read_status = 'unread'",
        [id],
    )?;
    Ok(())
}

pub fn mark_read(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("UPDATE articles SET read_status = 'read' WHERE id = ?1", [id])?;
    Ok(())
}

pub fn set_saved(conn: &Connection, id: i64, saved: bool) -> AppResult<()> {
    conn.execute("UPDATE articles SET saved = ?1 WHERE id = ?2", params![saved, id])?;
    Ok(())
}

pub fn set_hidden(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("UPDATE articles SET hidden = 1 WHERE id = ?1", [id])?;
    Ok(())
}

/// (topic_id, relevance) pairs for an article, most relevant first.
pub fn topic_relevances(conn: &Connection, id: i64) -> AppResult<Vec<(i64, f64)>> {
    let mut st =
        conn.prepare("SELECT topic_id, relevance FROM article_topics WHERE article_id = ?1 ORDER BY relevance DESC")?;
    let rows = st
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn source_feed_ids(conn: &Connection, id: i64) -> AppResult<Vec<i64>> {
    let mut st = conn.prepare("SELECT feed_id FROM article_sources WHERE article_id = ?1")?;
    let rows = st.query_map([id], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn count(conn: &Connection) -> AppResult<i64> {
    Ok(conn.query_row("SELECT count(*) FROM articles", [], |r| r.get(0))?)
}

// ---------------------------------------------------------------- body (P2 reader)

pub struct BodyUpdate<'a> {
    pub status: &'a str,
    pub text: Option<&'a str>,
    pub html: Option<&'a str>,
    pub canonical_url: Option<&'a str>,
    pub word_count: Option<u32>,
    pub difficulty: Option<&'a str>,
}

pub fn save_body(conn: &Connection, id: i64, b: &BodyUpdate) -> AppResult<()> {
    let n = conn.execute(
        "UPDATE articles SET body_status = ?1, body_text = ?2, body_html = ?3,
           canonical_url = COALESCE(?4, canonical_url), word_count = ?5, difficulty = ?6 WHERE id = ?7",
        params![
            b.status,
            b.text,
            b.html,
            b.canonical_url,
            b.word_count,
            b.difficulty,
            id
        ],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("article {id}")));
    }
    Ok(())
}

/// Sanitized HTML body and body status for the Reader.
pub fn body_html(conn: &Connection, id: i64) -> AppResult<(String, Option<String>)> {
    Ok(
        conn.query_row("SELECT body_status, body_html FROM articles WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?,
    )
}

pub fn body_text(conn: &Connection, id: i64) -> AppResult<Option<String>> {
    Ok(conn.query_row("SELECT body_text FROM articles WHERE id = ?1", [id], |r| r.get(0))?)
}
