//! Word Book service (P4 dev spec §6, §8): add/merge/edit, quizzes and reviews, CSV export.
//! Every write is refused in Hibernate (SPEC §6: the Word Book is read-only there).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::quiz;
use super::srs::{self, Grade, SrsState};
use crate::clock::{Clock, fmt_ts, parse_ts};
use crate::db::Db;
use crate::db::repo::vocab::{self as repo, Context, Fields, Review, Session, VocabFilter, VocabItem, VocabPage};
use crate::error::{AppError, AppResult};
use crate::events::{self, EventSink};
use crate::mode::{Mode, ModeManager};
use crate::settings::SettingsStore;

pub const MAX_TEXT: usize = 200;

/// Trim, collapse whitespace, lowercase, strip surrounding quotes and punctuation.
pub fn text_key(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewContext {
    pub sentence: Option<String>,
    pub article_id: Option<i64>,
    pub conversation_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewItem {
    #[serde(default = "default_kind")]
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub meaning_simple: Option<String>,
    #[serde(default)]
    pub meaning_b1: Option<String>,
    #[serde(default)]
    pub part_of_speech: Option<String>,
    #[serde(default)]
    pub ipa: Option<String>,
    #[serde(default)]
    pub syllables: Option<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub collocations: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub context: Option<NewContext>,
}

fn default_kind() -> String {
    "word".into()
}

/// Every field is replaced; an empty string clears it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPatch {
    pub kind: Option<String>,
    pub text: Option<String>,
    pub meaning_simple: Option<String>,
    pub meaning_b1: Option<String>,
    pub part_of_speech: Option<String>,
    pub ipa: Option<String>,
    pub examples: Option<Vec<String>>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddResult {
    /// "created" | "merged"
    pub outcome: &'static str,
    pub item: VocabItem,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDetail {
    pub item: VocabItem,
    pub contexts: Vec<Context>,
    pub reviews: Vec<Review>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DueCount {
    pub due: u32,
    pub new: u32,
    /// All items (the widget hides the line when the Word Book is empty).
    pub total: u32,
    /// Items with a meaning (a quiz needs at least 3).
    pub ready: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardAnswer {
    pub meaning: String,
    pub example: Option<String>,
    pub part_of_speech: Option<String>,
    pub ipa: Option<String>,
    /// The sentence it was saved from (or, for corrections, the explanation).
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuizCard {
    pub item_id: i64,
    pub kind: String,
    /// What to say aloud (🔊).
    pub text: String,
    pub prompt: String,
    /// For sentence/correction cards: the sentence to fix.
    pub original: Option<String>,
    pub answer: CardAnswer,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuizStart {
    pub session_id: i64,
    pub cards: Vec<QuizCard>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeResult {
    pub next_due_at: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuizResult {
    pub session_id: i64,
    pub finished_at: Option<String>,
    pub total: u32,
    pub remember: u32,
    pub unsure: u32,
    pub forgot: u32,
    /// 0..1
    pub score: f64,
    pub missed: Vec<VocabItem>,
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
}

fn clean_list(v: Vec<String>) -> Vec<String> {
    v.into_iter()
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn check_kind(kind: &str) -> AppResult<()> {
    if repo::KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(AppError::Invalid(format!("Unknown kind: {kind}")))
    }
}

fn check_text(text: &str) -> AppResult<String> {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() || text_key(&t).is_empty() {
        return Err(AppError::Invalid("Type a word or phrase first".into()));
    }
    if t.chars().count() > MAX_TEXT {
        return Err(AppError::Invalid(format!(
            "That is too long (max {MAX_TEXT} characters)"
        )));
    }
    Ok(t)
}

fn card(item: &VocabItem, first_context: Option<String>) -> QuizCard {
    let fixes = matches!(item.kind.as_str(), "sentence" | "correction");
    let meaning = item
        .meaning_simple
        .clone()
        .or_else(|| item.meaning_b1.clone())
        .unwrap_or_default();
    if fixes {
        QuizCard {
            item_id: item.id,
            kind: item.kind.clone(),
            text: item.text.clone(),
            prompt: "How would you say this correctly?".into(),
            original: item.notes.clone(),
            answer: CardAnswer {
                meaning: item.text.clone(),
                example: None,
                part_of_speech: None,
                ipa: None,
                context: Some(meaning).filter(|m| !m.is_empty()),
            },
        }
    } else {
        QuizCard {
            item_id: item.id,
            kind: item.kind.clone(),
            text: item.text.clone(),
            prompt: item.text.clone(),
            original: None,
            answer: CardAnswer {
                meaning,
                example: item.examples.first().cloned(),
                part_of_speech: item.part_of_speech.clone(),
                ipa: item.ipa.clone(),
                context: first_context,
            },
        }
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub struct VocabService {
    db: Db,
    clock: Arc<dyn Clock>,
    settings: Arc<SettingsStore>,
    events: Arc<dyn EventSink>,
    mode: Arc<ModeManager>,
    rng: Mutex<fastrand::Rng>,
}

impl VocabService {
    pub fn new(
        db: Db,
        clock: Arc<dyn Clock>,
        settings: Arc<SettingsStore>,
        events: Arc<dyn EventSink>,
        mode: Arc<ModeManager>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            clock,
            settings,
            events,
            mode,
            rng: Mutex::new(fastrand::Rng::new()),
        })
    }

    /// Deterministic quizzes (tests).
    pub fn seed(&self, seed: u64) {
        *self.rng.lock().unwrap() = fastrand::Rng::with_seed(seed);
    }

    fn writable(&self) -> AppResult<()> {
        match self.mode.get() {
            Mode::Hibernate => Err(AppError::Hibernating),
            Mode::Standard => Ok(()),
        }
    }

    fn changed(&self, item_id: Option<i64>) {
        events::emit(
            self.events.as_ref(),
            events::VOCAB_CHANGED,
            &json!({ "itemId": item_id }),
        );
    }

    pub async fn add(&self, n: NewItem) -> AppResult<AddResult> {
        self.writable()?;
        check_kind(&n.kind)?;
        let text = check_text(&n.text)?;
        let key = text_key(&text);
        let now = fmt_ts(self.clock.now());
        let fields = Fields {
            meaning_simple: clean(n.meaning_simple),
            meaning_b1: clean(n.meaning_b1),
            part_of_speech: clean(n.part_of_speech),
            ipa: clean(n.ipa),
            syllables: clean(n.syllables),
            examples: clean_list(n.examples),
            collocations: clean_list(n.collocations),
            notes: clean(n.notes),
        };
        let ctx = n.context.unwrap_or_default();
        let sentence = clean(ctx.sentence);
        let kind = n.kind;
        let out = self
            .db
            .tx(move |tx| {
                let (id, outcome) = match repo::find(tx, &kind, &key)? {
                    Some(existing) => {
                        repo::fill_empty(tx, existing.id, &fields, &now)?;
                        if existing.status == "known" {
                            repo::set_status(tx, existing.id, "learning", &now)?;
                        }
                        (existing.id, "merged")
                    }
                    None => (repo::insert(tx, &kind, &text, &key, &fields, &now)?, "created"),
                };
                let duplicate = match &sentence {
                    Some(s) => repo::has_context(tx, id, s)?,
                    None => ctx.article_id.is_none() && ctx.conversation_id.is_none(),
                };
                if !duplicate {
                    repo::add_context(tx, id, sentence.as_deref(), ctx.article_id, ctx.conversation_id, &now)?;
                }
                Ok(AddResult {
                    outcome,
                    item: repo::get(tx, id)?,
                })
            })
            .await?;
        tracing::info!(item = out.item.id, outcome = out.outcome, "word book add");
        self.changed(Some(out.item.id));
        Ok(out)
    }

    pub async fn update(&self, id: i64, p: ItemPatch) -> AppResult<VocabItem> {
        self.writable()?;
        let now = fmt_ts(self.clock.now());
        let item = self
            .db
            .tx(move |tx| {
                let cur = repo::get(tx, id)?;
                let kind = p.kind.unwrap_or(cur.kind.clone());
                check_kind(&kind)?;
                let text = match p.text {
                    Some(t) => check_text(&t)?,
                    None => cur.text.clone(),
                };
                let key = text_key(&text);
                if let Some(other) = repo::find(tx, &kind, &key)?
                    && other.id != id
                {
                    return Err(AppError::Invalid(format!(
                        "\"{}\" is already in your Word Book",
                        other.text
                    )));
                }
                let pick = |new: Option<String>, old: Option<String>| match new {
                    Some(v) => clean(Some(v)),
                    None => old,
                };
                let f = Fields {
                    meaning_simple: pick(p.meaning_simple, cur.meaning_simple),
                    meaning_b1: pick(p.meaning_b1, cur.meaning_b1),
                    part_of_speech: pick(p.part_of_speech, cur.part_of_speech),
                    ipa: pick(p.ipa, cur.ipa),
                    syllables: cur.syllables,
                    examples: p.examples.map(clean_list).unwrap_or(cur.examples),
                    collocations: cur.collocations,
                    notes: pick(p.notes, cur.notes),
                };
                repo::update(tx, id, &kind, &text, &key, &f, &now)?;
                repo::get(tx, id)
            })
            .await?;
        self.changed(Some(id));
        Ok(item)
    }

    pub async fn delete(&self, id: i64) -> AppResult<()> {
        self.writable()?;
        self.db.call(move |c| repo::delete(c, id)).await?;
        self.changed(Some(id));
        Ok(())
    }

    pub async fn list(&self, f: VocabFilter, cursor: Option<i64>, limit: u32) -> AppResult<VocabPage> {
        let now = fmt_ts(self.clock.now());
        self.db.call(move |c| repo::list(c, &f, &now, cursor, limit)).await
    }

    pub async fn detail(&self, id: i64) -> AppResult<ItemDetail> {
        self.db
            .call(move |c| {
                Ok(ItemDetail {
                    item: repo::get(c, id)?,
                    contexts: repo::contexts(c, id)?,
                    reviews: repo::reviews(c, id, 20)?,
                })
            })
            .await
    }

    pub async fn keys(&self) -> AppResult<Vec<String>> {
        self.db.call(|c| repo::keys(c)).await
    }

    pub async fn due_count(&self) -> AppResult<DueCount> {
        let now = fmt_ts(self.clock.now());
        self.db
            .call(move |c| {
                let (due, new) = repo::due_counts(c, &now)?;
                let (total, ready): (u32, u32) = c.query_row(
                    "SELECT count(*), COALESCE(SUM(meaning_simple IS NOT NULL OR meaning_b1 IS NOT NULL), 0) FROM vocab_items",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                Ok(DueCount { due, new, total, ready })
            })
            .await
    }

    /// Write `tech-english-wordbook-YYYY-MM-DD.csv` into `dir`; returns its path.
    pub async fn export_csv(&self, dir: &Path) -> AppResult<PathBuf> {
        let date = self.clock.today_local();
        let path = dir.join(format!("tech-english-wordbook-{date}.csv"));
        let rows = self
            .db
            .call(|c| {
                let items = repo::all(c)?;
                let mut out = Vec::with_capacity(items.len());
                for i in items {
                    let ctx = repo::contexts(c, i.id)?.into_iter().last();
                    out.push((i, ctx));
                }
                Ok(out)
            })
            .await?;
        let mut csv = String::from(
            "kind,text,meaning_simple,meaning_b1,part_of_speech,examples,status,review_count,due_at,first_context,source_article_url\n",
        );
        for (i, ctx) in &rows {
            let fields = [
                i.kind.clone(),
                i.text.clone(),
                i.meaning_simple.clone().unwrap_or_default(),
                i.meaning_b1.clone().unwrap_or_default(),
                i.part_of_speech.clone().unwrap_or_default(),
                i.examples.join(" | "),
                i.status.clone(),
                i.review_count.to_string(),
                i.due_at.clone().unwrap_or_default(),
                ctx.as_ref().and_then(|c| c.sentence.clone()).unwrap_or_default(),
                ctx.as_ref().and_then(|c| c.article_url.clone()).unwrap_or_default(),
            ];
            csv.push_str(&fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
            csv.push('\n');
        }
        // BOM so Numbers and Excel read UTF-8 (IPA symbols) correctly.
        let p = path.clone();
        tokio::task::spawn_blocking(move || std::fs::write(&p, format!("\u{feff}{csv}")))
            .await?
            .map_err(|e| AppError::Internal(format!("could not write the CSV: {e}")))?;
        tracing::info!(items = rows.len(), "word book exported");
        Ok(path)
    }

    // ------------------------------------------------------------ quizzes

    /// A new quiz: `item_ids` (e.g. "practice missed again"), or `size` items chosen by §10.2.
    pub async fn start_quiz(&self, size: Option<u32>, item_ids: Option<Vec<i64>>) -> AppResult<QuizStart> {
        self.writable()?;
        let size = size.unwrap_or(self.settings.get().learning.quiz_size).clamp(5, 30) as usize;
        let now = self.clock.now();
        let items = self.db.call(|c| repo::all(c)).await?;
        let ids = match item_ids {
            Some(ids) => {
                let ok: Vec<i64> = ids
                    .into_iter()
                    .filter(|id| items.iter().any(|i| i.id == *id && i.has_meaning()))
                    .collect();
                if ok.is_empty() {
                    return Err(AppError::Invalid("None of these words has a meaning yet".into()));
                }
                ok
            }
            None => {
                if items.iter().filter(|i| i.has_meaning()).count() < quiz::MIN_ITEMS {
                    return Err(AppError::Invalid(
                        "Add a few more words first (at least 3 with a meaning)".into(),
                    ));
                }
                let mut rng = self.rng.lock().unwrap();
                quiz::select(&items, size, now, &mut rng)
            }
        };
        let now_s = fmt_ts(now);
        self.db
            .tx(move |tx| {
                let session_id = repo::start_session(tx, ids.len() as u32, &now_s)?;
                let mut cards = Vec::with_capacity(ids.len());
                for id in &ids {
                    let item = repo::get(tx, *id)?;
                    let ctx = repo::contexts(tx, *id)?.into_iter().find_map(|c| c.sentence);
                    cards.push(card(&item, ctx));
                }
                Ok(QuizStart { session_id, cards })
            })
            .await
    }

    pub async fn grade(&self, session_id: i64, item_id: i64, grade: Grade) -> AppResult<GradeResult> {
        self.writable()?;
        let now = self.clock.now();
        let retention = self.settings.get().learning.desired_retention;
        let out = self
            .db
            .tx(move |tx| {
                repo::session(tx, session_id)?;
                if repo::graded_in_session(tx, session_id, item_id)? {
                    return Err(AppError::Invalid("This word was already graded in this quiz".into()));
                }
                let item = repo::get(tx, item_id)?;
                let state = SrsState {
                    stability: item.stability,
                    difficulty: item.difficulty,
                    last_reviewed_at: item.last_reviewed_at.as_deref().and_then(parse_ts),
                };
                let u = srs::review(&state, grade, now, retention)?;
                let status = srs::next_status(&item.status, grade, u.stability);
                let due = fmt_ts(u.due_at);
                repo::apply_review(
                    tx,
                    item_id,
                    &repo::ReviewUpdate {
                        grade: grade.as_str(),
                        status,
                        srs_state: u.srs_state,
                        stability: u.stability,
                        difficulty: u.difficulty,
                        due_at: &due,
                        now: &fmt_ts(now),
                    },
                    Some(session_id),
                )?;
                Ok(GradeResult {
                    next_due_at: due,
                    status: status.to_string(),
                })
            })
            .await?;
        self.changed(Some(item_id));
        Ok(out)
    }

    fn result(s: Session, missed: Vec<VocabItem>) -> QuizResult {
        QuizResult {
            session_id: s.id,
            finished_at: s.finished_at,
            total: s.item_count,
            remember: s.remember,
            unsure: s.unsure,
            forgot: s.forgot,
            score: s.score.unwrap_or_else(|| quiz::score(s.remember, s.unsure, s.forgot)),
            missed,
        }
    }

    pub async fn finish(&self, session_id: i64) -> AppResult<QuizResult> {
        let now = fmt_ts(self.clock.now());
        let r = self
            .db
            .tx(move |tx| {
                let s = repo::session(tx, session_id)?;
                repo::finish_session(tx, session_id, quiz::score(s.remember, s.unsure, s.forgot), &now)?;
                Ok(Self::result(
                    repo::session(tx, session_id)?,
                    repo::missed_in_session(tx, session_id)?,
                ))
            })
            .await?;
        tracing::info!(session = session_id, score = r.score, "quiz finished");
        self.changed(None);
        Ok(r)
    }

    pub async fn history(&self, limit: u32) -> AppResult<Vec<QuizResult>> {
        self.db
            .call(move |c| {
                Ok(repo::history(c, limit.clamp(1, 100))?
                    .into_iter()
                    .map(|s| Self::result(s, vec![]))
                    .collect())
            })
            .await
    }

    // ------------------------------------------------------------ meanings

    /// Items still without a meaning, oldest first (for the auto-fill job).
    pub async fn pending(&self, limit: u32) -> AppResult<Vec<(VocabItem, Option<Context>)>> {
        self.db
            .call(move |c| {
                repo::pending(c, limit)?
                    .into_iter()
                    .map(|i| {
                        let ctx = repo::contexts(c, i.id)?.into_iter().next();
                        Ok((i, ctx))
                    })
                    .collect()
            })
            .await
    }

    /// Fill only the empty fields of one item. Returns false in Hibernate.
    pub async fn fill(&self, id: i64, f: Fields) -> AppResult<bool> {
        if self.writable().is_err() {
            return Ok(false);
        }
        let now = fmt_ts(self.clock.now());
        self.db.call(move |c| repo::fill_empty(c, id, &f, &now)).await?;
        self.changed(Some(id));
        Ok(true)
    }

    pub fn can_write(&self) -> bool {
        self.writable().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use crate::events::RecordingEventSink;
    use chrono::Duration;

    struct T {
        svc: Arc<VocabService>,
        clock: Arc<FakeClock>,
        mode: Arc<ModeManager>,
        db: Db,
        events: Arc<RecordingEventSink>,
    }

    async fn setup() -> T {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            Ok(c.execute(
                "INSERT INTO articles(id, url, normalized_url, title, title_key, source_name, discovered_at)
                 VALUES (1, 'https://a/1', 'https://a/1', 'Local AI', 'local ai', 'Blog', '2026-09-29T00:00:00Z'),
                        (2, 'https://a/2', 'https://a/2', 'Kafka', 'kafka', 'Blog', '2026-09-29T00:00:00Z')",
                [],
            )?)
        })
        .await
        .unwrap();
        let clock = Arc::new(FakeClock::at("2026-09-30T09:00:00Z"));
        let events = Arc::new(RecordingEventSink::default());
        let settings = SettingsStore::load(db.clone()).await.unwrap();
        let mode = ModeManager::load(db.clone(), events.clone()).await.unwrap();
        let svc = VocabService::new(db.clone(), clock.clone(), settings, events.clone(), mode.clone());
        svc.seed(1);
        T {
            svc,
            clock,
            mode,
            db,
            events,
        }
    }

    fn word(text: &str, meaning: Option<&str>, sentence: Option<&str>, article: Option<i64>) -> NewItem {
        NewItem {
            kind: "word".into(),
            text: text.into(),
            meaning_simple: meaning.map(str::to_string),
            meaning_b1: None,
            part_of_speech: None,
            ipa: None,
            syllables: None,
            examples: vec![],
            collocations: vec![],
            notes: None,
            context: Some(NewContext {
                sentence: sentence.map(str::to_string),
                article_id: article,
                conversation_id: None,
            }),
        }
    }

    #[test]
    fn keys() {
        assert_eq!(text_key("  \"Inference,\" "), "inference");
        assert_eq!(text_key("Data   Lakehouse."), "data lakehouse");
        assert_eq!(text_key("C++"), "c");
        assert_eq!(text_key("..."), "");
    }

    #[tokio::test]
    async fn re_adding_merges_contexts_and_fills_empty_fields() {
        let t = setup().await;
        let a = t
            .svc
            .add(word("Inference", None, Some("It runs inference locally."), Some(1)))
            .await
            .unwrap();
        assert_eq!(a.outcome, "created");
        assert_eq!(a.item.text_key, "inference");
        assert!(!a.item.has_meaning());
        // Same word, new sentence and a meaning → merged, meaning filled, 2 contexts.
        let b = t
            .svc
            .add(word(
                "inference ",
                Some("a conclusion from evidence"),
                Some("Inference is cheap now."),
                Some(2),
            ))
            .await
            .unwrap();
        assert_eq!((b.outcome, b.item.id), ("merged", a.item.id));
        assert_eq!(b.item.meaning_simple.as_deref(), Some("a conclusion from evidence"));
        // The same sentence again adds no context; a different meaning does not overwrite.
        t.svc
            .add(word(
                "inference",
                Some("other"),
                Some("Inference is cheap now."),
                Some(2),
            ))
            .await
            .unwrap();
        let d = t.svc.detail(a.item.id).await.unwrap();
        assert_eq!(d.contexts.len(), 2);
        assert_eq!(d.contexts[0].article_title.as_deref(), Some("Kafka"));
        assert_eq!(d.item.meaning_simple.as_deref(), Some("a conclusion from evidence"));
        assert!(t.events.names().contains(&events::VOCAB_CHANGED.to_string()));
    }

    #[tokio::test]
    async fn known_items_go_back_to_learning_when_added_again() {
        let t = setup().await;
        let a = t.svc.add(word("latency", Some("delay"), None, None)).await.unwrap();
        let id = a.item.id;
        t.db.call(move |c| repo::set_status(c, id, "known", "x")).await.unwrap();
        let b = t
            .svc
            .add(word("Latency", None, Some("Low latency matters."), Some(1)))
            .await
            .unwrap();
        assert_eq!(b.item.status, "learning");
    }

    #[tokio::test]
    async fn validation_and_key_collisions() {
        let t = setup().await;
        assert!(t.svc.add(word("   ", None, None, None)).await.is_err());
        assert!(t.svc.add(word(&"x".repeat(201), None, None, None)).await.is_err());
        let mut bad = word("x", None, None, None);
        bad.kind = "emoji".into();
        assert!(t.svc.add(bad).await.is_err());
        let a = t.svc.add(word("throughput", None, None, None)).await.unwrap().item;
        let b = t.svc.add(word("latency", None, None, None)).await.unwrap().item;
        let patch = |text: &str| ItemPatch {
            kind: None,
            text: Some(text.into()),
            meaning_simple: Some("how much work per second".into()),
            meaning_b1: None,
            part_of_speech: None,
            ipa: None,
            examples: None,
            notes: None,
        };
        let e = t.svc.update(b.id, patch("Throughput")).await.unwrap_err();
        assert!(e.to_string().contains("already in your Word Book"));
        let ok = t.svc.update(a.id, patch("Throughput")).await.unwrap();
        assert_eq!(ok.text, "Throughput", "same item: a new spelling is fine");
        // A phrase with the same key as a word is a different item.
        let mut phrase = word("latency", None, None, None);
        phrase.kind = "phrase".into();
        assert_eq!(t.svc.add(phrase).await.unwrap().outcome, "created");
    }

    #[tokio::test]
    async fn hibernate_makes_the_word_book_read_only() {
        let t = setup().await;
        let a = t
            .svc
            .add(word("latency", Some("delay"), None, None))
            .await
            .unwrap()
            .item;
        t.mode.set(Mode::Hibernate).await.unwrap();
        assert!(matches!(
            t.svc.add(word("x", None, None, None)).await,
            Err(AppError::Hibernating)
        ));
        assert!(matches!(t.svc.delete(a.id).await, Err(AppError::Hibernating)));
        assert!(matches!(t.svc.start_quiz(None, None).await, Err(AppError::Hibernating)));
        assert!(
            t.svc.list(VocabFilter::default(), None, 50).await.is_ok(),
            "reading is fine"
        );
        assert!(t.svc.detail(a.id).await.is_ok());
    }

    #[tokio::test]
    async fn list_filters() {
        let t = setup().await;
        t.svc
            .add(word("latency", Some("delay"), Some("s"), Some(1)))
            .await
            .unwrap();
        t.svc.add(word("throughput", None, Some("s2"), Some(2))).await.unwrap();
        let mut term = word("vector database", Some("stores embeddings"), None, None);
        term.kind = "term".into();
        t.svc.add(term).await.unwrap();
        let f = |f: VocabFilter| {
            let svc = t.svc.clone();
            async move { svc.list(f, None, 50).await.unwrap() }
        };
        assert_eq!(f(VocabFilter::default()).await.total, 3);
        assert_eq!(
            f(VocabFilter {
                pending_only: true,
                ..Default::default()
            })
            .await
            .items[0]
                .text,
            "throughput"
        );
        assert_eq!(
            f(VocabFilter {
                query: Some("embedd".into()),
                ..Default::default()
            })
            .await
            .total,
            1,
            "search looks in meanings too"
        );
        assert_eq!(
            f(VocabFilter {
                article_id: Some(1),
                ..Default::default()
            })
            .await
            .total,
            1
        );
        assert_eq!(
            f(VocabFilter {
                kind: Some("term".into()),
                ..Default::default()
            })
            .await
            .total,
            1
        );
        let page = t.svc.list(VocabFilter::default(), None, 2).await.unwrap();
        assert_eq!(page.items.len(), 2);
        let rest = t.svc.list(VocabFilter::default(), page.next_cursor, 2).await.unwrap();
        assert_eq!(rest.items.len(), 1);
        assert_eq!(t.svc.keys().await.unwrap().len(), 3);
    }

    async fn add_words(t: &T, n: usize) -> Vec<i64> {
        let mut ids = Vec::new();
        for i in 0..n {
            let a = t
                .svc
                .add(word(
                    &format!("word{i}"),
                    Some(&format!("meaning {i}")),
                    Some(&format!("Sentence {i}.")),
                    Some(1),
                ))
                .await
                .unwrap();
            ids.push(a.item.id);
        }
        ids
    }

    #[tokio::test]
    async fn quiz_needs_three_words_with_a_meaning() {
        let t = setup().await;
        add_words(&t, 2).await;
        t.svc.add(word("pending", None, None, None)).await.unwrap();
        let e = t.svc.start_quiz(None, None).await.unwrap_err();
        assert!(e.to_string().contains("at least 3"));
        t.svc.add(word("third", Some("m"), None, None)).await.unwrap();
        let q = t.svc.start_quiz(None, None).await.unwrap();
        assert_eq!(q.cards.len(), 3, "a small pool uses every word with a meaning");
        assert!(q.cards.iter().all(|c| c.prompt != "pending"));
    }

    #[tokio::test]
    async fn a_quiz_updates_srs_counters_and_the_score() {
        let t = setup().await;
        add_words(&t, 12).await;
        let before = t.svc.due_count().await.unwrap();
        assert_eq!((before.due, before.new, before.total, before.ready), (0, 12, 12, 12));
        let q = t.svc.start_quiz(Some(10), None).await.unwrap();
        assert_eq!(q.cards.len(), 10);
        let card = &q.cards[0];
        assert_eq!(
            card.answer.context.as_deref().map(|s| s.starts_with("Sentence")),
            Some(true)
        );
        let grades = [Grade::Remember; 6]
            .into_iter()
            .chain([Grade::Unsure; 2])
            .chain([Grade::Forgot; 2]);
        let mut forgot_id = 0;
        for (c, g) in q.cards.iter().zip(grades) {
            let r = t.svc.grade(q.session_id, c.item_id, g).await.unwrap();
            if g == Grade::Forgot {
                forgot_id = c.item_id;
                assert_eq!(parse_ts(&r.next_due_at).unwrap(), t.clock.now() + Duration::minutes(10));
            }
            assert_eq!(r.status, "learning");
        }
        assert!(
            t.svc
                .grade(q.session_id, q.cards[0].item_id, Grade::Forgot)
                .await
                .is_err(),
            "one grade per word per quiz"
        );
        let r = t.svc.finish(q.session_id).await.unwrap();
        assert_eq!((r.total, r.remember, r.unsure, r.forgot), (10, 6, 2, 2));
        assert!((r.score - 0.7).abs() < 1e-9, "(6 + 0.5×2) / 10");
        assert_eq!(r.missed.len(), 4);
        let item = t.svc.detail(forgot_id).await.unwrap();
        assert_eq!((item.item.review_count, item.item.forgot_count), (1, 1));
        assert_eq!(item.item.last_grade.as_deref(), Some("forgot"));
        assert_eq!(item.reviews.len(), 1);

        let after = t.svc.due_count().await.unwrap();
        assert_eq!(after.new, 2, "10 of 12 were reviewed");
        t.clock.advance(Duration::minutes(11));
        assert_eq!(
            t.svc.due_count().await.unwrap().due,
            2,
            "the 2 forgotten words are due again"
        );
        let h = t.svc.history(5).await.unwrap();
        assert_eq!(h.len(), 1);
        assert!((h[0].score - 0.7).abs() < 1e-9);

        // Practice the missed ones again.
        let again = t
            .svc
            .start_quiz(None, Some(r.missed.iter().map(|m| m.id).collect()))
            .await
            .unwrap();
        assert_eq!(again.cards.len(), 4);
    }

    #[tokio::test]
    async fn unfinished_quizzes_are_not_in_history_but_grades_count() {
        let t = setup().await;
        add_words(&t, 5).await;
        let q = t.svc.start_quiz(Some(5), None).await.unwrap();
        t.svc
            .grade(q.session_id, q.cards[0].item_id, Grade::Remember)
            .await
            .unwrap();
        assert!(t.svc.history(5).await.unwrap().is_empty());
        assert_eq!(t.svc.due_count().await.unwrap().new, 4);
    }

    #[tokio::test]
    async fn correction_cards_ask_for_the_fixed_sentence() {
        let t = setup().await;
        let mut c = word(
            "I have been to Paris last year.",
            Some("Use the past simple with 'last year'."),
            None,
            None,
        );
        c.kind = "correction".into();
        c.text = "I went to Paris last year.".into();
        c.notes = Some("I have been to Paris last year.".into());
        let item = t.svc.add(c).await.unwrap().item;
        let card = card(&item, None);
        assert_eq!(card.prompt, "How would you say this correctly?");
        assert_eq!(card.original.as_deref(), Some("I have been to Paris last year."));
        assert_eq!(card.answer.meaning, "I went to Paris last year.");
    }

    #[tokio::test]
    async fn csv_export() {
        let t = setup().await;
        let mut w = word(
            "latency",
            Some("delay, in \"network\" terms"),
            Some("Low latency, please."),
            Some(1),
        );
        w.examples = vec!["a".into(), "b".into()];
        t.svc.add(w).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let p = t.svc.export_csv(dir.path()).await.unwrap();
        assert!(p.ends_with("tech-english-wordbook-2026-09-30.csv"));
        let s = std::fs::read_to_string(p).unwrap();
        let mut lines = s.trim_start_matches('\u{feff}').lines();
        assert!(lines.next().unwrap().starts_with("kind,text,meaning_simple"));
        assert_eq!(
            lines.next().unwrap(),
            "word,latency,\"delay, in \"\"network\"\" terms\",,,a | b,new,0,,\"Low latency, please.\",https://a/1"
        );
    }
}
