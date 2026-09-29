use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;

use crate::error::{AppError, AppResult};

pub const USER_AGENT: &str = concat!("TechEnglish/", env!("CARGO_PKG_VERSION"), " (+local desktop app)");

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub timeout: Duration,
    pub max_bytes: usize,
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            timeout: Duration::from_secs(15),
            max_bytes: 5 * 1024 * 1024,
        }
    }
    pub fn header(mut self, k: &str, v: impl Into<String>) -> Self {
        self.headers.push((k.to_string(), v.into()));
        self
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    /// Header names are lowercased.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub final_url: String,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }
}

#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn get(&self, req: HttpRequest) -> AppResult<HttpResponse>;
}

pub struct ReqwestClient {
    client: reqwest::Client,
}

impl ReqwestClient {
    pub fn new() -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::limited(5))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
        Ok(Self { client })
    }
}

fn map_err(e: reqwest::Error) -> AppError {
    if e.is_timeout() {
        AppError::Network("Timed out".into())
    } else if e.is_connect() {
        AppError::Network("Could not connect".into())
    } else {
        AppError::Network(e.to_string())
    }
}

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn get(&self, req: HttpRequest) -> AppResult<HttpResponse> {
        let mut rb = self.client.get(&req.url).timeout(req.timeout);
        for (k, v) in &req.headers {
            rb = rb.header(k, v);
        }
        let mut resp = rb.send().await.map_err(map_err)?;
        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let headers = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_string())))
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(map_err)? {
            if body.len() + chunk.len() > req.max_bytes {
                return Err(AppError::Invalid("Response too large".into()));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
            final_url,
        })
    }
}
