pub mod repo;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

/// Embedded migrations. Never edit a shipped migration — add a new one.
const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../../migrations/0001_init.sql"))];

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
        assert!(n >= 11);
    }
}
