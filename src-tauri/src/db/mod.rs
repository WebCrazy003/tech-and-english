pub mod repo;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

/// Embedded migrations. Never edit a shipped migration — add a new one.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../../migrations/0001_init.sql")),
    (2, include_str!("../../migrations/0002_reader_ai.sql")),
    (3, include_str!("../../migrations/0003_learning.sql")),
    (4, include_str!("../../migrations/0004_vocab.sql")),
    (5, include_str!("../../migrations/0005_voice.sql")),
];

/// Single SQLite connection shared by all services. rusqlite is synchronous, so
/// every call runs on the blocking thread pool.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> AppResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> AppResult<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run `f` with the connection on the blocking pool.
    pub async fn call<F, R>(&self, f: F) -> AppResult<R>
    where
        F: FnOnce(&mut Connection) -> AppResult<R> + Send + 'static,
        R: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().map_err(|_| AppError::Internal("db lock poisoned".into()))?;
            f(&mut guard)
        })
        .await?
    }

    /// Run `f` inside a transaction; commits on `Ok`, rolls back on `Err`.
    pub async fn tx<F, R>(&self, f: F) -> AppResult<R>
    where
        F: FnOnce(&rusqlite::Transaction) -> AppResult<R> + Send + 'static,
        R: Send + 'static,
    {
        self.call(move |c| {
            let tx = c.transaction()?;
            let r = f(&tx)?;
            tx.commit()?;
            Ok(r)
        })
        .await
    }
}

fn migrate(conn: &mut Connection) -> AppResult<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (version, sql) in MIGRATIONS {
        if *version > current {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
            tracing::info!(version, "applied migration");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_apply_and_are_idempotent() {
        let db = Db::open_in_memory().unwrap();
        let v: i64 = db
            .call(|c| {
                migrate(c)?;
                Ok(c.pragma_query_value(None, "user_version", |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(v, MIGRATIONS.last().unwrap().0);
        let n: i64 = db
            .call(|c| {
                Ok(
                    c.query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert!(n >= 13);
    }

    /// 0003 on a database with P1/P2 data: picks become stories, new feeds are added once.
    #[test]
    fn learning_migration_keeps_picks_and_skips_existing_feeds() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(MIGRATIONS[0].1).unwrap();
        c.execute_batch(MIGRATIONS[1].1).unwrap();
        c.pragma_update(None, "user_version", 2).unwrap();
        c.execute_batch(
            "INSERT INTO articles(id, url, normalized_url, title, title_key, source_name, discovered_at)
               VALUES (1, 'https://a/1', 'https://a/1', 't', 't', 's', '2026-09-28T00:00:00Z');
             INSERT INTO daily_picks(date, article_id, why, why_source, created_at)
               VALUES ('2026-09-28', 1, 'why', 'llm', '2026-09-28T08:00:00Z');
             INSERT INTO feeds(kind, name, url, source_weight, created_at)
               VALUES ('rss', 'My Real Python', 'https://realpython.com/atom.xml', 0.9, 'x'),
                      ('rss', 'DuckDB', 'https://duckdb.org/feed.xml', 0.6, 'x');",
        )
        .unwrap();
        migrate(&mut c).unwrap();

        let (kind, why_source): (String, String) = c
            .query_row(
                "SELECT kind, why_source FROM daily_picks WHERE date = '2026-09-28'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((kind.as_str(), why_source.as_str()), ("story", "llm"));
        // A lesson for the same date is allowed now.
        c.execute(
            "INSERT INTO daily_picks(date, kind, article_id, why, created_at) VALUES ('2026-09-28', 'lesson', 1, 'w', 'x')",
            [],
        )
        .unwrap();

        let rp: (String, f64, bool) = c
            .query_row(
                "SELECT name, source_weight, learning FROM feeds WHERE url = 'https://realpython.com/atom.xml'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            rp,
            ("My Real Python".to_string(), 0.9, false),
            "existing feed untouched"
        );
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM feeds WHERE url = 'https://realpython.com/atom.xml'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "no duplicate");
        let learning: i64 = c
            .query_row("SELECT count(*) FROM feeds WHERE learning = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(learning, 8 + 1, "8 new sources + DuckDB flagged");
    }

    #[test]
    fn learning_migration_adds_no_feeds_to_a_fresh_install() {
        let mut c = Connection::open_in_memory().unwrap();
        migrate(&mut c).unwrap();
        let n: i64 = c.query_row("SELECT count(*) FROM feeds", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "onboarding offers them instead");
    }
}
