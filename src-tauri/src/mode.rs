use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::db::repo::app_state;
use crate::error::AppResult;
use crate::events::{self, EventSink};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Standard,
    Hibernate,
}

impl Mode {
    fn parse(s: &str) -> Mode {
        if s == "hibernate" {
            Mode::Hibernate
        } else {
            Mode::Standard
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Standard => "standard",
            Mode::Hibernate => "hibernate",
        }
    }
}

type Observer = Box<dyn Fn(Mode) + Send + Sync>;

/// Standard / Hibernate. P3/P4 register observers to stop sidecars on Hibernate.
pub struct ModeManager {
    db: Db,
    events: Arc<dyn EventSink>,
    mode: RwLock<Mode>,
    observers: Mutex<Vec<Observer>>,
}

impl ModeManager {
    pub async fn load(db: Db, events: Arc<dyn EventSink>) -> AppResult<Arc<Self>> {
        let stored = db.call(|c| app_state::get(c, app_state::MODE)).await?;
        let mode = stored.as_deref().map(Mode::parse).unwrap_or_default();
        Ok(Arc::new(Self {
            db,
            events,
            mode: RwLock::new(mode),
            observers: Mutex::new(Vec::new()),
        }))
    }

    pub fn get(&self) -> Mode {
        *self.mode.read().unwrap()
    }

    pub async fn set(&self, mode: Mode) -> AppResult<Mode> {
        self.db
            .call(move |c| app_state::set(c, app_state::MODE, mode.as_str()))
            .await?;
        let changed = {
            let mut m = self.mode.write().unwrap();
            let changed = *m != mode;
            *m = mode;
            changed
        };
        if changed {
            tracing::info!(mode = mode.as_str(), "mode changed");
            events::emit(self.events.as_ref(), events::MODE_CHANGED, &mode);
            for f in self.observers.lock().unwrap().iter() {
                f(mode);
            }
        }
        Ok(mode)
    }

    pub fn on_change(&self, f: impl Fn(Mode) + Send + Sync + 'static) {
        self.observers.lock().unwrap().push(Box::new(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RecordingEventSink;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn persists_and_notifies() {
        let db = Db::open_in_memory().unwrap();
        let sink = Arc::new(RecordingEventSink::default());
        let m = ModeManager::load(db.clone(), sink.clone()).await.unwrap();
        assert_eq!(m.get(), Mode::Standard);
        let calls = Arc::new(AtomicUsize::new(0));
        let c2 = calls.clone();
        m.on_change(move |_| {
            c2.fetch_add(1, Ordering::SeqCst);
        });
        m.set(Mode::Hibernate).await.unwrap();
        m.set(Mode::Hibernate).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no event when unchanged");
        assert_eq!(sink.names(), vec![events::MODE_CHANGED]);
        let again = ModeManager::load(db, sink).await.unwrap();
        assert_eq!(again.get(), Mode::Hibernate);
    }
}
