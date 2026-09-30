//! `LlmProvider`: the app's only way to talk to a language model (P2 dev spec §9).
//! `LocalLlamaProvider` streams from llama-server's OpenAI-compatible API.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::{Stream, StreamExt, stream};
use serde::Serialize;
use serde_json::{Value, json};

use super::manager::AiManager;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChatMsg {
    pub role: String,
    pub content: String,
}

impl ChatMsg {
    pub fn system(c: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: c.into(),
        }
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: c.into(),
        }
    }
    pub fn assistant(c: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: c.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub messages: Vec<ChatMsg>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub json_schema: Option<Value>,
}

impl LlmRequest {
    pub fn new(messages: Vec<ChatMsg>, max_tokens: u32) -> Self {
        Self {
            messages,
            max_tokens,
            temperature: 0.4,
            json_schema: None,
        }
    }
}

pub type ChunkStream = Pin<Box<dyn Stream<Item = AppResult<String>> + Send>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stream text deltas. `force` skips the low-memory check when the model must start.
    async fn stream(&self, req: LlmRequest, force: bool) -> AppResult<ChunkStream>;
    /// Model id if loaded (or last used), for caching keys.
    fn model_id(&self) -> String;
    /// True when a request would not need to load the model first.
    fn is_ready(&self) -> bool;

    async fn complete(&self, req: LlmRequest) -> AppResult<String> {
        let mut s = self.stream(req, false).await?;
        let mut out = String::new();
        while let Some(chunk) = s.next().await {
            out.push_str(&chunk?);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- SSE parsing

#[derive(Debug, PartialEq)]
pub enum SseEvent {
    Delta(String),
    Done,
    Error(String),
}

/// Line-based parser for `data: …` server-sent events; handles lines split across reads.
#[derive(Default)]
pub struct SseParser {
    buf: String,
}

impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        let mut out = Vec::new();
        while let Some(i) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=i).collect();
            let line = line.trim();
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                out.push(SseEvent::Done);
                continue;
            }
            match serde_json::from_str::<Value>(data) {
                Ok(v) => {
                    if let Some(err) = v.get("error") {
                        let msg = err.get("message").and_then(Value::as_str).unwrap_or("AI error");
                        out.push(SseEvent::Error(msg.to_string()));
                    } else if let Some(c) = v["choices"][0]["delta"]["content"].as_str()
                        && !c.is_empty()
                    {
                        out.push(SseEvent::Delta(c.to_string()));
                    }
                }
                Err(_) => tracing::debug!("skipping bad SSE line"),
            }
        }
        out
    }
}

// ---------------------------------------------------------------- llama-server provider

pub struct LocalLlamaProvider {
    manager: Arc<AiManager>,
    http: reqwest::Client,
}

impl LocalLlamaProvider {
    pub fn new(manager: Arc<AiManager>) -> Self {
        Self {
            manager,
            http: reqwest::Client::builder().no_proxy().build().expect("http client"),
        }
    }

    pub fn request_body(req: &LlmRequest) -> Value {
        let mut body = json!({
            "messages": req.messages,
            "stream": true,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "cache_prompt": true,
            // Hybrid thinking models (Qwen3.5) otherwise spend the whole budget thinking.
            "chat_template_kwargs": { "enable_thinking": false },
        });
        if let Some(schema) = &req.json_schema {
            body["response_format"] =
                json!({ "type": "json_schema", "json_schema": { "name": "out", "schema": schema } });
        }
        body
    }
}

#[async_trait]
impl LlmProvider for LocalLlamaProvider {
    async fn stream(&self, req: LlmRequest, force: bool) -> AppResult<ChunkStream> {
        let ep = self.manager.ensure_ready(force).await?;
        let busy = self.manager.busy();
        let resp = self
            .http
            .post(format!("{}/v1/chat/completions", ep.base_url))
            .bearer_auth(&ep.api_key)
            .json(&Self::request_body(&req))
            .send()
            .await
            .map_err(|e| AppError::ai("ai_error", format!("AI request failed: {e}")))?;
        if !resp.status().is_success() {
            let code = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::ai(
                "ai_error",
                format!("AI error (HTTP {code}): {}", text.chars().take(200).collect::<String>()),
            ));
        }
        let bytes = resp.bytes_stream();
        // State: byte stream, parser, pending events, finished flag, busy guard (dropped with the stream).
        let s = stream::unfold(
            (
                bytes,
                SseParser::default(),
                std::collections::VecDeque::<SseEvent>::new(),
                false,
                busy,
            ),
            |(mut bytes, mut parser, mut pending, mut done, busy)| async move {
                loop {
                    if let Some(ev) = pending.pop_front() {
                        match ev {
                            SseEvent::Delta(t) => return Some((Ok(t), (bytes, parser, pending, done, busy))),
                            SseEvent::Error(m) => {
                                done = true;
                                return Some((Err(AppError::ai("ai_error", m)), (bytes, parser, pending, done, busy)));
                            }
                            SseEvent::Done => {
                                done = true;
                                pending.clear();
                                continue;
                            }
                        }
                    }
                    if done {
                        return None;
                    }
                    match bytes.next().await {
                        Some(Ok(b)) => pending.extend(parser.push(&b)),
                        Some(Err(e)) => {
                            done = true;
                            return Some((
                                Err(AppError::ai("ai_error", format!("AI stream broke: {e}"))),
                                (bytes, parser, pending, done, busy),
                            ));
                        }
                        None => return None,
                    }
                }
            },
        );
        Ok(Box::pin(s))
    }

    fn model_id(&self) -> String {
        self.manager.status().model_id.unwrap_or_else(|| "unknown".into())
    }

    fn is_ready(&self) -> bool {
        self.manager.is_ready()
    }
}

// ---------------------------------------------------------------- test double

/// Returns scripted answers (each a list of chunks) and records requests.
#[derive(Default)]
pub struct MockProvider {
    pub answers: Mutex<std::collections::VecDeque<Vec<String>>>,
    pub requests: Mutex<Vec<LlmRequest>>,
    pub ready: std::sync::atomic::AtomicBool,
}

impl MockProvider {
    pub fn with_answers(answers: &[&[&str]]) -> Self {
        let m = Self::default();
        for a in answers {
            m.answers
                .lock()
                .unwrap()
                .push_back(a.iter().map(|s| s.to_string()).collect());
        }
        m.ready.store(true, std::sync::atomic::Ordering::SeqCst);
        m
    }
    pub fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn stream(&self, req: LlmRequest, _force: bool) -> AppResult<ChunkStream> {
        self.requests.lock().unwrap().push(req);
        let chunks = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AppError::ai("ai_error", "no scripted answer"))?;
        Ok(Box::pin(stream::iter(chunks.into_iter().map(Ok))))
    }
    fn model_id(&self) -> String {
        "mock".into()
    }
    fn is_ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::manager::tests::rig;
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, ResponseTemplate};

    const SSE: &str = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n\
: keep-alive\n\n\
data: [DONE]\n\n";

    #[test]
    fn parser_handles_any_split() {
        for cut in 0..SSE.len() {
            let mut p = SseParser::default();
            let mut ev = p.push(&SSE.as_bytes()[..cut]);
            ev.extend(p.push(&SSE.as_bytes()[cut..]));
            assert_eq!(
                ev,
                vec![
                    SseEvent::Delta("Hello".into()),
                    SseEvent::Delta(" world".into()),
                    SseEvent::Done
                ],
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn parser_reports_errors() {
        let mut p = SseParser::default();
        assert_eq!(
            p.push(b"data: {\"error\":{\"message\":\"context too long\"}}\n"),
            vec![SseEvent::Error("context too long".into())]
        );
    }

    #[test]
    fn body_disables_thinking_and_sets_schema() {
        let mut r = LlmRequest::new(vec![ChatMsg::user("hi")], 50);
        r.json_schema = Some(json!({"type":"object"}));
        let b = LocalLlamaProvider::request_body(&r);
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(b["response_format"]["type"], "json_schema");
        assert_eq!(b["cache_prompt"], true);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn streams_from_the_server_with_the_api_key() {
        let r = rig(true).await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header_exists("authorization"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(SSE, "text/event-stream"))
            .mount(&r.server)
            .await;
        let p = LocalLlamaProvider::new(r.manager.clone());
        let text = p
            .complete(LlmRequest::new(vec![ChatMsg::user("hi")], 20))
            .await
            .unwrap();
        assert_eq!(text, "Hello world");
        assert_eq!(r.manager.status().state, "ready", "busy cleared after the stream");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_error_is_an_ai_error() {
        let r = rig(true).await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&r.server)
            .await;
        let p = LocalLlamaProvider::new(r.manager.clone());
        let e = p
            .complete(LlmRequest::new(vec![ChatMsg::user("hi")], 20))
            .await
            .unwrap_err();
        assert_eq!(e.code(), "ai_error");
    }

    #[tokio::test]
    async fn mock_provider_scripts() {
        let m = MockProvider::with_answers(&[&["a", "b"]]);
        assert_eq!(m.complete(LlmRequest::new(vec![], 1)).await.unwrap(), "ab");
        assert!(m.complete(LlmRequest::new(vec![], 1)).await.is_err());
        assert_eq!(m.calls(), 2);
    }
}
