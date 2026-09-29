use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub struct HnStats {
    pub id: i64,
    pub points: i64,
    pub comments: i64,
}

/// An item as returned by a feed source, before normalization.
#[derive(Debug, Clone)]
pub struct RawItem {
    pub url: String,
    pub title: String,
    /// Plain text (HTML stripped), at most 1000 chars.
    pub description: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub hn: Option<HnStats>,
}

#[derive(Debug, Default)]
pub struct FetchResult {
    pub items: Vec<RawItem>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub not_modified: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScoreBreakdown {
    pub topic_relevance: f64,
    pub freshness: f64,
    pub popularity: f64,
    pub source_preference: f64,
    pub novelty: f64,
    pub user_history: f64,
    /// 0..100
    pub total: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    Opened,
    Read,
    Skipped,
    Saved,
    Discussed,
    Liked,
    NotInterested,
}

impl InteractionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Opened => "opened",
            Self::Read => "read",
            Self::Skipped => "skipped",
            Self::Saved => "saved",
            Self::Discussed => "discussed",
            Self::Liked => "liked",
            Self::NotInterested => "not_interested",
        }
    }
}
