//! Reader AI features: cached summaries, the per-article chat, and the background "why" text
//! (P2 dev spec §10–§14). One request runs at a time; interactive requests go first.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use rusqlite::Connection;
use serde::Serialize;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::prompts::{self, ArticleText, QuickAction};
use super::provider::{ChatMsg, LlmProvider, LlmRequest};
use crate::clock::{Clock, fmt_ts};
use crate::db::Db;
use crate::db::repo::ai::{self as ai_repo, ChatMessage, Derivative};
use crate::db::repo::{articles, picks};
use crate::error::{AppError, AppResult};
use crate::events::{self, EventSink};
use crate::settings::SettingsStore;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StreamEvent {
    /// The model is starting (first use, or after an idle unload).
    Loading,
    Delta {
        text: String,
    },
    Done {
        cached: bool,
        model_id: String,
    },
    Error {
        code: String,
        message: String,
    },
}

pub type Sink = Arc<dyn Fn(StreamEvent) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivKind {
    SummaryB1,
    EasyEnglish,
}

impl DerivKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DerivKind::SummaryB1 => "summary_b1",
            DerivKind::EasyEnglish => "easy_english",
        }
    }
}

struct Loaded {
    title: String,
    source: String,
    text: String,
    from_description_only: bool,
    keywords: Vec<String>,
}

fn load_article(conn: &Connection, id: i64) -> AppResult<Loaded> {
    let item = articles::get_item(conn, id)?;
    let body = articles::body_text(conn, id)?.filter(|b| !b.trim().is_empty());
    let (text, from_description_only) = match body {
        Some(b) => (b, false),
        None => (item.description.clone().unwrap_or_else(|| item.title.clone()), true),
    };
    let mut st = conn.prepare(
        "SELECT t.keywords FROM article_topics at JOIN topics t ON t.id = at.topic_id WHERE at.article_id = ?1",
    )?;
    let keywords = st
        .query_map([id], |r| r.get::<_, String>(0))?
        .filter_map(Result::ok)
        .flat_map(|k| serde_json::from_str::<Vec<String>>(&k).unwrap_or_default())
        .collect();
    Ok(Loaded {
        title: item.title,
        source: item.source_name,
        text,
        from_description_only,
        keywords,
    })
}

/// "Explain simply" output. The model writes snake_case JSON; the UI gets camelCase.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct DefineTermOut {
    pub meaning_simple: String,
    #[serde(default)]
    pub meaning_b1: String,
    #[serde(default)]
    pub part_of_speech: String,
    #[serde(default)]
    pub ipa: Option<String>,
    #[serde(default)]
    pub syllables: Option<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub collocations: Vec<String>,
}

fn trim_to(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>().trim_end())
    }
}

/// Read the model's JSON (tolerating code fences or text around it) and enforce the limits.
pub fn parse_define_term(text: &str) -> AppResult<DefineTermOut> {
    let bad = || AppError::ai("ai_error", "The AI answer could not be read. Please try again.");
    let (start, end) = (text.find('{').ok_or_else(bad)?, text.rfind('}').ok_or_else(bad)?);
    let mut o: DefineTermOut = serde_json::from_str(text.get(start..=end).ok_or_else(bad)?).map_err(|_| bad())?;
    if o.meaning_simple.trim().is_empty() {
        return Err(bad());
    }
    o.meaning_simple = trim_to(&o.meaning_simple, 160);
    o.meaning_b1 = trim_to(&o.meaning_b1, 240);
    o.ipa = o.ipa.map(|x| trim_to(&x, 60)).filter(|x| !x.is_empty());
    o.syllables = o.syllables.map(|x| trim_to(&x, 60)).filter(|x| !x.is_empty());
    o.examples = o
        .examples
        .iter()
        .map(|e| trim_to(e, 160))
        .filter(|e| !e.is_empty())
        .take(3)
        .collect();
    o.collocations = o
        .collocations
        .iter()
        .map(|c| trim_to(c, 40))
        .filter(|c| !c.is_empty())
        .take(4)
        .collect();
    Ok(o)
}

pub struct AiService {
    db: Db,
    clock: Arc<dyn Clock>,
    settings: Arc<SettingsStore>,
    events: Arc<dyn EventSink>,
    provider: Arc<dyn LlmProvider>,
    llm: Semaphore,
    interactive_waiting: AtomicUsize,
    jobs: Mutex<HashMap<u64, CancellationToken>>,
    next_job: AtomicU64,
    /// "Explain simply" answers for this session, keyed by (term key, sentence).
    term_cache: Mutex<HashMap<(String, String), DefineTermOut>>,
}

impl AiService {
    pub fn new(
        db: Db,
        clock: Arc<dyn Clock>,
        settings: Arc<SettingsStore>,
        events: Arc<dyn EventSink>,
        provider: Arc<dyn LlmProvider>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            clock,
            settings,
            events,
            provider,
            llm: Semaphore::new(1),
            interactive_waiting: AtomicUsize::new(0),
            jobs: Mutex::new(HashMap::new()),
            next_job: AtomicU64::new(1),
            term_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Register a cancellable job; returns (id, token).
    pub fn new_job(&self) -> (u64, CancellationToken) {
        let id = self.next_job.fetch_add(1, Ordering::SeqCst);
        let token = CancellationToken::new();
        self.jobs.lock().unwrap().insert(id, token.clone());
        (id, token)
    }

    pub fn cancel(&self, job_id: u64) {
        if let Some(t) = self.jobs.lock().unwrap().get(&job_id) {
            t.cancel();
        }
    }

    pub fn finish_job(&self, job_id: u64) {
        self.jobs.lock().unwrap().remove(&job_id);
    }

    /// Stream `req` to `sink`; returns the full text and whether it was cancelled.
    async fn run_llm(
        &self,
        req: LlmRequest,
        force: bool,
        token: &CancellationToken,
        sink: &Sink,
    ) -> AppResult<(String, bool)> {
        if !self.provider.is_ready() {
            sink(StreamEvent::Loading);
        }
        self.interactive_waiting.fetch_add(1, Ordering::SeqCst);
        let permit = self.llm.acquire().await;
        self.interactive_waiting.fetch_sub(1, Ordering::SeqCst);
        let _permit = permit.map_err(|_| AppError::Internal("AI queue closed".into()))?;
        let mut stream = self.provider.stream(req, force).await?;
        let mut text = String::new();
        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => return Ok((text, true)),
                next = stream.next() => match next {
                    None => break,
                    Some(Ok(delta)) => {
                        text.push_str(&delta);
                        sink(StreamEvent::Delta { text: delta });
                    }
                    Some(Err(e)) => return Err(e),
                }
            }
        }
        Ok((text, false))
    }

    /// One full (non-streamed) answer with interactive priority. May start the model.
    async fn complete_interactive(&self, req: LlmRequest, force: bool) -> AppResult<String> {
        self.interactive_waiting.fetch_add(1, Ordering::SeqCst);
        let permit = self.llm.acquire().await;
        self.interactive_waiting.fetch_sub(1, Ordering::SeqCst);
        let _permit = permit.map_err(|_| AppError::Internal("AI queue closed".into()))?;
        let mut stream = self.provider.stream(req, force).await?;
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            text.push_str(&chunk?);
        }
        Ok(text)
    }

    fn term_request(&self, term: &str, sentence: &str, title: &str) -> LlmRequest {
        let mut req = LlmRequest::new(prompts::define_term(self.level(), term, sentence, title), 400);
        req.temperature = 0.3;
        req.json_schema = Some(prompts::define_term_schema());
        req
    }

    /// "Explain simply" for a selected term (P4 dev spec §3.2). Cached for the session.
    pub async fn define_term(
        &self,
        term: &str,
        sentence: Option<&str>,
        article_id: Option<i64>,
        force: bool,
    ) -> AppResult<DefineTermOut> {
        let term = term.trim();
        if term.is_empty() || term.chars().count() > 80 {
            return Err(AppError::Invalid("Select 1–8 words first".into()));
        }
        let sentence = sentence.unwrap_or("").trim().to_string();
        let key = (crate::learning::vocab::text_key(term), sentence.clone());
        if let Some(hit) = self.term_cache.lock().unwrap().get(&key) {
            return Ok(hit.clone());
        }
        let title = match article_id {
            Some(id) => self.db.call(move |c| Ok(articles::get_item(c, id)?.title)).await?,
            None => String::new(),
        };
        let text = self
            .complete_interactive(self.term_request(term, &sentence, &title), force)
            .await?;
        let out = parse_define_term(&text)?;
        self.term_cache.lock().unwrap().insert(key, out.clone());
        Ok(out)
    }

    /// The same, for the background auto-fill: only when the model is already loaded and nothing
    /// else is running. `Ok(None)` = not now (never starts the model).
    pub async fn define_term_background(
        &self,
        term: &str,
        sentence: &str,
        title: &str,
    ) -> AppResult<Option<DefineTermOut>> {
        if !self.provider.is_ready() || self.interactive_waiting.load(Ordering::SeqCst) > 0 {
            return Ok(None);
        }
        let Ok(_permit) = self.llm.try_acquire() else {
            return Ok(None);
        };
        let text = self.provider.complete(self.term_request(term, sentence, title)).await?;
        parse_define_term(&text).map(Some)
    }

    fn level(&self) -> u8 {
        self.settings.get().ai.english_level
    }

    /// Cached B1 summary / Easy English, generated (and cached) on a miss.
    pub async fn derivative(
        &self,
        article_id: i64,
        kind: DerivKind,
        regenerate: bool,
        force: bool,
        token: &CancellationToken,
        sink: &Sink,
    ) -> AppResult<Option<String>> {
        let k = kind.as_str();
        if regenerate {
            self.db
                .call(move |c| ai_repo::delete_derivative(c, article_id, k))
                .await?;
        } else if let Some(d) = self.db.call(move |c| ai_repo::get_derivative(c, article_id, k)).await?
            && d.prompt_version == prompts::PROMPT_VERSION
        {
            sink(StreamEvent::Delta { text: d.text.clone() });
            sink(StreamEvent::Done {
                cached: true,
                model_id: d.model_id,
            });
            return Ok(Some(d.text));
        }
        let a = self.db.call(move |c| load_article(c, article_id)).await?;
        let at = ArticleText {
            title: &a.title,
            source: &a.source,
            text: &a.text,
            from_description_only: a.from_description_only,
        };
        let (messages, max_tokens) = match kind {
            DerivKind::SummaryB1 => (prompts::summarize_b1(self.level(), &at, &a.keywords), 550),
            DerivKind::EasyEnglish => (prompts::simplify_easy(self.level(), &at, &a.keywords), 750),
        };
        let (mut text, cancelled) = self
            .run_llm(LlmRequest::new(messages, max_tokens), force, token, sink)
            .await?;
        if cancelled {
            return Ok(None);
        }
        if a.from_description_only {
            text = format!("{}\n\n_Based on the short description only._", text.trim_end());
        }
        let model_id = self.provider.model_id();
        let d = Derivative {
            text: text.clone(),
            model_id: model_id.clone(),
            prompt_version: prompts::PROMPT_VERSION.into(),
        };
        let now = fmt_ts(self.clock.now());
        self.db
            .call(move |c| ai_repo::put_derivative(c, article_id, k, &d, &now))
            .await?;
        sink(StreamEvent::Done {
            cached: false,
            model_id,
        });
        Ok(Some(text))
    }

    /// Save the user's message (the frontend shows it immediately).
    pub async fn add_user_message(&self, article_id: i64, text: String) -> AppResult<ChatMessage> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(AppError::Invalid("Type a question first".into()));
        }
        if text.chars().count() > 2_000 {
            return Err(AppError::Invalid(
                "That question is too long (max 2,000 characters)".into(),
            ));
        }
        let now = fmt_ts(self.clock.now());
        self.db
            .call(move |c| {
                articles::get_item(c, article_id)?;
                ai_repo::add_chat(c, article_id, "user", &text, None, &now)
            })
            .await
    }

    /// Answer the last user message of the article's chat (P2 dev spec §11.4).
    pub async fn answer_chat(
        &self,
        article_id: i64,
        action: Option<QuickAction>,
        force: bool,
        token: &CancellationToken,
        sink: &Sink,
    ) -> AppResult<()> {
        let (answer, cancelled) = if action == Some(QuickAction::Summarize) {
            // Reuse the cached B1 summary: no extra model call.
            match self
                .derivative(article_id, DerivKind::SummaryB1, false, force, token, sink)
                .await?
            {
                Some(s) => (s, false),
                None => (String::new(), true),
            }
        } else {
            let a = self.db.call(move |c| load_article(c, article_id)).await?;
            let history = self
                .db
                .call(move |c| ai_repo::recent_chat(c, article_id, prompts::CHAT_HISTORY))
                .await?;
            let question = history
                .iter()
                .rev()
                .find(|m| m.role == "user")
                .map(|m| m.content.clone())
                .unwrap_or_default();
            let summary = self
                .db
                .call(move |c| ai_repo::get_derivative(c, article_id, "summary_b1"))
                .await?
                .map(|d| d.text);
            let relevant = prompts::relevant_paragraphs(&a.text, &question, prompts::CHAT_CONTEXT_CHARS);
            let at = ArticleText {
                title: &a.title,
                source: &a.source,
                text: &a.text,
                from_description_only: a.from_description_only,
            };
            let mut messages = vec![ChatMsg::system(prompts::article_chat_system(
                self.level(),
                &at,
                summary.as_deref(),
                &relevant,
            ))];
            messages.extend(history.into_iter().map(|m| ChatMsg {
                role: m.role,
                content: m.content,
            }));
            let (text, cancelled) = self.run_llm(LlmRequest::new(messages, 450), force, token, sink).await?;
            if !cancelled {
                sink(StreamEvent::Done {
                    cached: false,
                    model_id: self.provider.model_id(),
                });
            }
            (text, cancelled)
        };
        let content = if cancelled {
            format!("{} …(stopped)", answer.trim_end())
        } else {
            answer
        };
        if !content.trim().is_empty() && content.trim() != "…(stopped)" {
            let (now, model) = (fmt_ts(self.clock.now()), self.provider.model_id());
            self.db
                .call(move |c| ai_repo::add_chat(c, article_id, "assistant", &content, Some(&model), &now))
                .await?;
        }
        Ok(())
    }

    /// AI-written "why" for today's story (the lesson keeps its template). Only when the model is already loaded and idle;
    /// never starts the model (P2 dev spec §14.2).
    pub async fn background_why(&self, date: String) -> AppResult<bool> {
        if !self.settings.get().ai.llm_why || !self.provider.is_ready() {
            return Ok(false);
        }
        if self.interactive_waiting.load(Ordering::SeqCst) > 0 {
            return Ok(false);
        }
        let Ok(_permit) = self.llm.try_acquire() else {
            return Ok(false);
        };
        let d = date.clone();
        let row = self.db.call(move |c| picks::get(c, &d, picks::STORY)).await?;
        let Some(row) = row.filter(|r| r.why_source == "template") else {
            return Ok(false);
        };
        let id = row.article_id;
        let (item, topics) = self
            .db
            .call(move |c| {
                let item = articles::get_item(c, id)?;
                Ok((item.clone(), item.topics))
            })
            .await?;
        let messages = prompts::why_interesting(
            &item.title,
            item.description.as_deref().unwrap_or(""),
            &topics,
            item.hn_points,
        );
        let text = self.provider.complete(LlmRequest::new(messages, 90)).await?;
        let text = text.trim().trim_matches('"').to_string();
        if text.is_empty() {
            return Ok(false);
        }
        self.db
            .call(move |c| picks::set_why(c, &date, picks::STORY, &text, "llm"))
            .await?;
        events::emit(
            self.events.as_ref(),
            events::NEWS_UPDATED,
            &serde_json::json!({ "newCount": 0 }),
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::MockProvider;
    use crate::clock::FakeClock;
    use crate::events::RecordingEventSink;

    struct T {
        db: Db,
        svc: Arc<AiService>,
        mock: Arc<MockProvider>,
        article: i64,
    }

    async fn setup(answers: &[&[&str]]) -> T {
        let db = Db::open_in_memory().unwrap();
        let article = db
            .call(|c| {
                c.execute(
                    "INSERT INTO articles(url, normalized_url, title, title_key, source_name, discovered_at, description, body_text, body_status)
                     VALUES ('u','n','Kafka 5','kafka 5','Blog','2026-09-29T00:00:00Z','Short desc',
                             'Kafka is a log.\n\nTopics hold events.\n\nConsumers read them.','ok')",
                    [],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap();
        let settings = SettingsStore::load(db.clone()).await.unwrap();
        let mock = Arc::new(MockProvider::with_answers(answers));
        let svc = AiService::new(
            db.clone(),
            Arc::new(FakeClock::at("2026-09-29T10:00:00Z")),
            settings,
            Arc::new(RecordingEventSink::default()),
            mock.clone(),
        );
        T { db, svc, mock, article }
    }

    fn collector() -> (Sink, Arc<Mutex<Vec<StreamEvent>>>) {
        let v = Arc::new(Mutex::new(Vec::new()));
        let v2 = v.clone();
        (Arc::new(move |e| v2.lock().unwrap().push(e)), v)
    }

    #[tokio::test]
    async fn summary_is_cached_then_regenerated() {
        let t = setup(&[&["Sum", "mary"], &["New"]]).await;
        let (sink, ev) = collector();
        let tok = CancellationToken::new();
        let s = t
            .svc
            .derivative(t.article, DerivKind::SummaryB1, false, false, &tok, &sink)
            .await
            .unwrap();
        assert_eq!(s.as_deref(), Some("Summary"));
        let s2 = t
            .svc
            .derivative(t.article, DerivKind::SummaryB1, false, false, &tok, &sink)
            .await
            .unwrap();
        assert_eq!(s2.as_deref(), Some("Summary"));
        assert_eq!(t.mock.calls(), 1, "second call served from the cache");
        assert!(ev.lock().unwrap().contains(&StreamEvent::Done {
            cached: true,
            model_id: "mock".into()
        }));
        let s3 = t
            .svc
            .derivative(t.article, DerivKind::SummaryB1, true, false, &tok, &sink)
            .await
            .unwrap();
        assert_eq!(s3.as_deref(), Some("New"));
        assert_eq!(t.mock.calls(), 2);
    }

    #[tokio::test]
    async fn chat_uses_history_and_saves_answers() {
        let t = setup(&[&["Kafka ", "stores events."], &["Yes."]]).await;
        let (sink, _) = collector();
        let tok = CancellationToken::new();
        t.svc
            .add_user_message(t.article, "What do topics hold?".into())
            .await
            .unwrap();
        t.svc.answer_chat(t.article, None, false, &tok, &sink).await.unwrap();
        t.svc.add_user_message(t.article, "Really?".into()).await.unwrap();
        t.svc.answer_chat(t.article, None, false, &tok, &sink).await.unwrap();
        let msgs = t.db.call(move |c| ai_repo::list_chat(c, 1)).await.unwrap();
        let roles: Vec<&str> = msgs.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "user", "assistant"]);
        assert_eq!(msgs[1].content, "Kafka stores events.");
        let reqs = t.mock.requests.lock().unwrap();
        let second = &reqs[1].messages;
        assert_eq!(second[0].role, "system");
        assert!(
            second[0].content.contains("Topics hold events."),
            "relevant paragraph included"
        );
        assert_eq!(second.len(), 1 + 3, "system + history (user, assistant, user)");
    }

    #[tokio::test]
    async fn history_is_limited_to_eight() {
        let t = setup(&[&["ok"]]).await;
        let a = t.article;
        t.db.call(move |c| {
            for i in 0..12 {
                ai_repo::add_chat(
                    c,
                    a,
                    if i % 2 == 0 { "user" } else { "assistant" },
                    &format!("m{i}"),
                    None,
                    "t",
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
        t.svc.add_user_message(a, "last".into()).await.unwrap();
        let (sink, _) = collector();
        t.svc
            .answer_chat(a, None, false, &CancellationToken::new(), &sink)
            .await
            .unwrap();
        assert_eq!(t.mock.requests.lock().unwrap()[0].messages.len(), 1 + 8);
    }

    #[tokio::test]
    async fn summarize_action_reuses_cached_summary() {
        let t = setup(&[&["Cached summary"]]).await;
        let (sink, _) = collector();
        let tok = CancellationToken::new();
        t.svc
            .derivative(t.article, DerivKind::SummaryB1, false, false, &tok, &sink)
            .await
            .unwrap();
        t.svc
            .add_user_message(t.article, QuickAction::Summarize.message().into())
            .await
            .unwrap();
        t.svc
            .answer_chat(t.article, Some(QuickAction::Summarize), false, &tok, &sink)
            .await
            .unwrap();
        assert_eq!(t.mock.calls(), 1, "no second model call");
        let msgs = t.db.call(move |c| ai_repo::list_chat(c, 1)).await.unwrap();
        assert_eq!(msgs.last().unwrap().content, "Cached summary");
    }

    #[tokio::test]
    async fn cancel_before_any_text_saves_nothing() {
        let t = setup(&[&["Half an ", "answer"]]).await;
        let (sink, _) = collector();
        let tok = CancellationToken::new();
        tok.cancel();
        t.svc.add_user_message(t.article, "Q?".into()).await.unwrap();
        t.svc.answer_chat(t.article, None, false, &tok, &sink).await.unwrap();
        let msgs = t.db.call(move |c| ai_repo::list_chat(c, 1)).await.unwrap();
        assert_eq!(msgs.len(), 1, "only the question");
    }

    #[tokio::test]
    async fn stopped_answer_is_saved_with_marker() {
        let t = setup(&[&["Half an ", "answer"]]).await;
        let tok = CancellationToken::new();
        let tok2 = tok.clone();
        // Stop after the first piece of text arrives (like pressing Stop).
        let sink: Sink = Arc::new(move |e| {
            if matches!(e, StreamEvent::Delta { .. }) {
                tok2.cancel();
            }
        });
        t.svc.add_user_message(t.article, "Q?".into()).await.unwrap();
        t.svc.answer_chat(t.article, None, false, &tok, &sink).await.unwrap();
        let msgs = t.db.call(move |c| ai_repo::list_chat(c, 1)).await.unwrap();
        assert_eq!(msgs.last().unwrap().content, "Half an …(stopped)");
    }

    #[tokio::test]
    async fn empty_or_long_questions_are_rejected() {
        let t = setup(&[]).await;
        assert!(t.svc.add_user_message(t.article, "  ".into()).await.is_err());
        assert!(t.svc.add_user_message(t.article, "x".repeat(2001)).await.is_err());
    }

    const TERM_JSON: &str = r#"Sure! ```json
{"meaning_simple":"using a trained model to get answers","meaning_b1":"Inference is when a trained AI model makes predictions.","part_of_speech":"noun","ipa":"ˈɪnfərəns","syllables":"IN·fer·ence","examples":["The laptop runs inference offline.","Inference costs money.","x"],"collocations":["run inference","inference speed"]}
```"#;

    #[test]
    fn define_term_json_is_read_and_limited() {
        let o = parse_define_term(TERM_JSON).unwrap();
        assert_eq!(o.part_of_speech, "noun");
        assert_eq!(o.examples.len(), 3);
        assert_eq!(o.syllables.as_deref(), Some("IN·fer·ence"));
        let long = format!(r#"{{"meaning_simple":"{}","examples":[]}}"#, "a".repeat(300));
        assert_eq!(parse_define_term(&long).unwrap().meaning_simple.chars().count(), 161);
        assert!(parse_define_term("no json here").is_err());
        assert!(parse_define_term(r#"{"meaning_simple":" "}"#).is_err());
        let v = serde_json::to_value(&o).unwrap();
        assert!(v.get("meaningSimple").is_some(), "camelCase for the UI");
    }

    #[tokio::test]
    async fn define_term_uses_a_schema_and_a_cache() {
        let t = setup(&[&[TERM_JSON]]).await;
        let a = t.article;
        let o = t
            .svc
            .define_term("Inference", Some("It runs inference."), Some(a), false)
            .await
            .unwrap();
        assert_eq!(o.meaning_simple, "using a trained model to get answers");
        let req = t.mock.requests.lock().unwrap()[0].clone();
        assert!(req.json_schema.is_some());
        assert!(req.messages[1].content.contains("Article title: Kafka 5"));
        let again = t
            .svc
            .define_term("inference", Some("It runs inference."), Some(a), false)
            .await
            .unwrap();
        assert_eq!(again, o);
        assert_eq!(t.mock.calls(), 1, "second call from the cache");
    }

    #[tokio::test]
    async fn background_define_never_starts_the_model() {
        let t = setup(&[&[TERM_JSON]]).await;
        t.mock.ready.store(false, Ordering::SeqCst);
        assert!(
            t.svc
                .define_term_background("inference", "", "")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(t.mock.calls(), 0);
        t.mock.ready.store(true, Ordering::SeqCst);
        assert!(
            t.svc
                .define_term_background("inference", "", "")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn background_why_only_when_ready() {
        let t = setup(&[&["\"Because you like Kafka.\""]]).await;
        let a = t.article;
        t.db.call(move |c| picks::insert(c, "2026-09-29", picks::STORY, a, "template why", "t"))
            .await
            .unwrap();
        t.mock.ready.store(false, Ordering::SeqCst);
        assert!(
            !t.svc.background_why("2026-09-29".into()).await.unwrap(),
            "model not loaded → skip"
        );
        t.mock.ready.store(true, Ordering::SeqCst);
        assert!(t.svc.background_why("2026-09-29".into()).await.unwrap());
        let row =
            t.db.call(|c| picks::get(c, "2026-09-29", picks::STORY))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            (row.why.as_str(), row.why_source.as_str()),
            ("Because you like Kafka.", "llm")
        );
        assert!(!t.svc.background_why("2026-09-29".into()).await.unwrap(), "only once");
    }
}
