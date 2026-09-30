//! Plain data-access functions over `&Connection`. Business rules live in services.

pub mod ai;
pub mod app_state;
pub mod articles;
pub mod feeds;
pub mod interactions;
pub mod notifications;
pub mod picks;
pub mod topics;
pub mod vocab;
pub mod voice;

use rusqlite::types::Value as SqlValue;

pub(crate) fn json_list(v: &[String]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

pub(crate) fn parse_json_list(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

/// `?,?,?` placeholder list for `IN (...)` clauses.
pub(crate) fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

pub(crate) fn ids_to_values(ids: &[i64]) -> Vec<SqlValue> {
    ids.iter().map(|i| SqlValue::Integer(*i)).collect()
}
