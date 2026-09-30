//! End-of-session review (P5 dev spec §9): the LLM groups the session's observations into
//! suggestions with preselection; a rule-based fallback does the same without the LLM.
//! Nothing reaches the Word Book until the learner picks it.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ai::provider::ChatMsg;
use crate::db::repo::voice::{Conversation, Observation, Turn};
use crate::error::{AppError, AppResult};
use crate::learning::vocab::{NewContext, NewItem, text_key};

pub const MAX_SUGGESTIONS: usize = 15;
const KINDS: &[&str] = &["word", "phrase", "term", "correction", "sentence"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub id: String,
    /// word | phrase | term | correction | sentence
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub meaning_simple: Option<String>,
    /// Corrections: the short reason. Others: optional.
    #[serde(default)]
    pub note: Option<String>,
    pub preselected: bool,
    #[serde(default)]
    pub observation_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStats {
    pub speaking_minutes: f64,
    pub new_words: usize,
    pub corrections: usize,
    pub drills: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionReview {
    pub conversation_id: i64,
    pub article_title: Option<String>,
    pub started_at: String,
    /// pending | done | skipped
    pub review_status: String,
    pub stats: ReviewStats,
    pub suggestions: Vec<Suggestion>,
    /// "llm" or "fallback"
    pub source: String,
}

pub fn stats(conv: &Conversation, obs: &[Observation]) -> ReviewStats {
    let distinct = |kinds: &[&str]| {
        obs.iter()
            .filter(|o| kinds.contains(&o.kind.as_str()))
            .map(|o| text_key(&o.text))
            .collect::<HashSet<_>>()
            .len()
    };
    ReviewStats {
        speaking_minutes: (conv.user_speaking_seconds as f64 / 60.0 * 10.0).round() / 10.0,
        new_words: distinct(&["unknown_word", "unknown_phrase"]),
        corrections: distinct(&["grammar"]),
        drills: obs.iter().filter(|o| o.kind == "pronunciation").count(),
    }
}

/// The same (kind, text) noted several times → one line, with all ids.
fn dedup(obs: &[Observation]) -> Vec<(Vec<i64>, &Observation)> {
    let mut order: Vec<(Vec<i64>, &Observation)> = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    for o in obs {
        let k = (o.kind.clone(), text_key(&o.text));
        match index.get(&k) {
            Some(&i) => order[i].0.push(o.id),
            None => {
                index.insert(k, order.len());
                order.push((vec![o.id], o));
            }
        }
    }
    order
}

fn detail_str(o: &Observation, key: &str) -> Option<String> {
    o.detail
        .as_ref()
        .and_then(|d| d.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ---------------------------------------------------------------- LLM review

pub fn prompt(level: u8, title: Option<&str>, obs: &[Observation], turns: &[Turn]) -> Vec<ChatMsg> {
    let notes: Vec<String> = dedup(obs)
        .into_iter()
        .map(|(ids, o)| {
            let extra = match o.kind.as_str() {
                "grammar" => format!(
                    " (the learner said: \"{}\"; reason: {})",
                    detail_str(o, "original").unwrap_or_default(),
                    detail_str(o, "explanation").unwrap_or_default()
                ),
                "pronunciation" => format!(
                    " (passed: {})",
                    o.detail.as_ref().and_then(|d| d.get("passed")).unwrap_or(&json!(false))
                ),
                _ => detail_str(o, "explanation")
                    .map(|e| format!(" — {e}"))
                    .unwrap_or_default(),
            };
            let ids: Vec<String> = ids.iter().map(i64::to_string).collect();
            format!("[{}] {}: {}{}", ids.join(","), o.kind, o.text, extra)
        })
        .collect();
    let said: Vec<String> = turns
        .iter()
        .filter(|t| t.role == "user")
        .map(|t| format!("- {}", t.text))
        .collect();
    vec![
        ChatMsg::system(format!(
            "You help an English learner review a speaking practice session. Their level: {}.\n\
             From the notes, choose what is worth saving in their Word Book: at most {MAX_SUGGESTIONS} suggestions.\n\
             - kind: word, phrase, term (a technical term), correction (text = the corrected sentence), or sentence (a useful sentence).\n\
             - meaning_simple: a short, simple meaning for words, phrases and terms.\n\
             - note: for corrections, the short reason.\n\
             - observation_ids: the [ids] of the notes each suggestion comes from.\n\
             - preselected = true for words the learner asked about, corrections (especially ones they repeated) and \
             technical terms central to the article. Do not preselect very common words or useful sentences.\n\
             Return JSON only.",
            super::tutor::level_name(level)
        )),
        ChatMsg::user(format!(
            "Article: {}\n\nNotes:\n{}\n\nWhat the learner said:\n{}",
            title.unwrap_or("(free conversation)"),
            notes.join("\n"),
            if said.is_empty() {
                "(nothing)".into()
            } else {
                said.join("\n")
            }
        )),
    ]
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["suggestions"],
        "properties": { "suggestions": { "type": "array", "maxItems": MAX_SUGGESTIONS, "items": {
            "type": "object",
            "required": ["kind", "text", "preselected", "observation_ids"],
            "properties": {
                "kind": { "type": "string", "enum": KINDS },
                "text": { "type": "string", "maxLength": 200 },
                "meaning_simple": { "type": "string", "maxLength": 200 },
                "note": { "type": "string", "maxLength": 200 },
                "preselected": { "type": "boolean" },
                "observation_ids": { "type": "array", "items": { "type": "integer" } } } } } }
    })
}

#[derive(Deserialize)]
struct RawSuggestion {
    kind: String,
    text: String,
    #[serde(default)]
    meaning_simple: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    preselected: bool,
    #[serde(default)]
    observation_ids: Vec<i64>,
}

#[derive(Deserialize)]
struct RawReview {
    suggestions: Vec<RawSuggestion>,
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
}

/// Validate the model's answer. An empty or unusable list is an error (→ retry, then fallback).
pub fn parse(text: &str, obs: &[Observation]) -> AppResult<Vec<Suggestion>> {
    let bad = || AppError::ai("ai_error", "The review could not be read.");
    let (start, end) = (text.find('{').ok_or_else(bad)?, text.rfind('}').ok_or_else(bad)?);
    let raw: RawReview = serde_json::from_str(text.get(start..=end).ok_or_else(bad)?).map_err(|_| bad())?;
    let known: HashSet<i64> = obs.iter().map(|o| o.id).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for r in raw.suggestions {
        let text = r.text.trim().to_string();
        if !KINDS.contains(&r.kind.as_str()) || text.is_empty() || !seen.insert((r.kind.clone(), text_key(&text))) {
            continue;
        }
        let ids: Vec<i64> = r.observation_ids.into_iter().filter(|i| known.contains(i)).collect();
        out.push(Suggestion {
            id: format!("s{}", out.len() + 1),
            preselected: r.preselected && r.kind != "sentence",
            kind: r.kind,
            text,
            meaning_simple: clean(r.meaning_simple),
            note: clean(r.note),
            observation_ids: ids,
        });
        if out.len() == MAX_SUGGESTIONS {
            break;
        }
    }
    if out.is_empty() && !obs.is_empty() {
        return Err(bad());
    }
    Ok(out)
}

// ---------------------------------------------------------------- fallback

/// Suggestions straight from the observations: unknown words/phrases and corrections are
/// preselected; useful sentences are not.
pub fn fallback(obs: &[Observation]) -> Vec<Suggestion> {
    let mut out = Vec::new();
    for (ids, o) in dedup(obs) {
        let (kind, text, meaning, note, pre) = match o.kind.as_str() {
            "unknown_word" | "unknown_phrase" => {
                let kind = if o.kind == "unknown_phrase" || o.text.split_whitespace().count() > 1 {
                    "phrase"
                } else {
                    "word"
                };
                (kind, o.text.clone(), detail_str(o, "explanation"), None, true)
            }
            "grammar" => ("correction", o.text.clone(), None, detail_str(o, "explanation"), true),
            "useful_sentence" => ("sentence", o.text.clone(), None, None, false),
            "pronunciation" => {
                let passed = o.detail.as_ref().and_then(|d| d.get("passed")).and_then(Value::as_bool);
                (
                    "word",
                    o.text.clone(),
                    None,
                    Some("You practised saying it.".into()),
                    passed != Some(true),
                )
            }
            _ => continue,
        };
        if out
            .iter()
            .any(|s: &Suggestion| s.kind == kind && text_key(&s.text) == text_key(&text))
        {
            continue;
        }
        out.push(Suggestion {
            id: format!("s{}", out.len() + 1),
            kind: kind.into(),
            text,
            meaning_simple: meaning,
            note,
            preselected: pre,
            observation_ids: ids,
        });
        if out.len() == MAX_SUGGESTIONS {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------- apply

/// Word Book items for the chosen suggestions, with their context sentence and the
/// conversation/article link. Returns (suggestion, item) pairs to add.
pub fn to_new_items(
    selected: &[Suggestion],
    obs: &[Observation],
    conversation_id: i64,
    article_id: Option<i64>,
) -> Vec<(Suggestion, NewItem)> {
    let by_id: HashMap<i64, &Observation> = obs.iter().map(|o| (o.id, o)).collect();
    selected
        .iter()
        .filter(|s| KINDS.contains(&s.kind.as_str()) && !s.text.trim().is_empty())
        .map(|s| {
            let first = s.observation_ids.iter().find_map(|i| by_id.get(i)).copied();
            let sentence = first
                .and_then(|o| detail_str(o, "context"))
                .or_else(|| Some(s.text.clone()));
            let (notes, meaning) = if s.kind == "correction" {
                let original = first.and_then(|o| detail_str(o, "original")).unwrap_or_default();
                let explanation = s
                    .note
                    .clone()
                    .or_else(|| first.and_then(|o| detail_str(o, "explanation")))
                    .unwrap_or_default();
                (
                    Some(json!({ "original": original, "explanation": explanation }).to_string()),
                    Some(explanation).filter(|e| !e.is_empty()),
                )
            } else {
                (s.note.clone(), s.meaning_simple.clone())
            };
            let item = NewItem {
                kind: s.kind.clone(),
                text: s.text.trim().to_string(),
                meaning_simple: meaning,
                meaning_b1: None,
                part_of_speech: None,
                ipa: None,
                syllables: None,
                examples: vec![],
                collocations: vec![],
                notes,
                context: Some(NewContext {
                    sentence,
                    article_id,
                    conversation_id: Some(conversation_id),
                }),
            };
            (s.clone(), item)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(id: i64, kind: &str, text: &str, detail: Value) -> Observation {
        Observation {
            id,
            kind: kind.into(),
            text: text.into(),
            detail: Some(detail),
            saved_item_id: None,
            created_at: "t".into(),
        }
    }

    fn sample() -> Vec<Observation> {
        vec![
            o(
                1,
                "unknown_word",
                "deployment",
                json!({"explanation": "putting software live", "context": "What does deployment mean?"}),
            ),
            o(
                2,
                "grammar",
                "Yesterday I deployed the app.",
                json!({"original": "Yesterday I deploy the app.", "explanation": "Past tense.", "context": "Yesterday I deploy the app."}),
            ),
            o(3, "useful_sentence", "It depends on the data.", json!({})),
            o(4, "unknown_word", "Deployment", json!({"context": "again"})),
            o(
                5,
                "pronunciation",
                "scalability",
                json!({"target": "scalability", "attempts": [{"heard": "stability"}], "passed": false}),
            ),
            o(
                6,
                "unknown_phrase",
                "rate limiting",
                json!({"explanation": "limit on requests"}),
            ),
        ]
    }

    #[test]
    fn fallback_groups_and_preselects() {
        let s = fallback(&sample());
        let kinds: Vec<(&str, &str, bool)> = s
            .iter()
            .map(|x| (x.kind.as_str(), x.text.as_str(), x.preselected))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("word", "deployment", true),
                ("correction", "Yesterday I deployed the app.", true),
                ("sentence", "It depends on the data.", false),
                ("word", "scalability", true),
                ("phrase", "rate limiting", true),
            ]
        );
        assert_eq!(s[0].observation_ids, vec![1, 4], "duplicates merged");
        assert_eq!(s[0].meaning_simple.as_deref(), Some("putting software live"));
        assert_eq!(s[1].note.as_deref(), Some("Past tense."));
    }

    #[test]
    fn llm_output_is_validated() {
        let obs = sample();
        let text = r#"{"suggestions":[
            {"kind":"word","text":"deployment","meaning_simple":"putting software live","preselected":true,"observation_ids":[1,99]},
            {"kind":"word","text":"Deployment","preselected":true,"observation_ids":[4]},
            {"kind":"idiom","text":"x","preselected":true,"observation_ids":[]},
            {"kind":"sentence","text":"It depends on the data.","preselected":true,"observation_ids":[3]},
            {"kind":"term","text":"  ","preselected":true,"observation_ids":[]}]}"#;
        let s = parse(text, &obs).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].observation_ids, vec![1], "unknown ids dropped");
        assert!(!s[1].preselected, "sentences are never preselected");
        assert_eq!((s[0].id.as_str(), s[1].id.as_str()), ("s1", "s2"));
        assert!(
            parse(r#"{"suggestions":[]}"#, &obs).is_err(),
            "empty with notes → retry"
        );
        assert!(parse(r#"{"suggestions":[]}"#, &[]).unwrap().is_empty());
        assert!(parse("nope", &obs).is_err());
    }

    #[test]
    fn prompt_lists_notes_once_with_ids() {
        let m = prompt(2, Some("Local AI"), &sample(), &[]);
        assert!(
            m[1].content
                .contains("[1,4] unknown_word: deployment — putting software live")
        );
        assert!(
            m[1].content
                .contains("the learner said: \"Yesterday I deploy the app.\"")
        );
        assert!(m[0].content.contains("at most 15"));
    }

    #[test]
    fn corrections_become_items_with_notes_and_context() {
        let obs = sample();
        let sel = fallback(&obs);
        let items = to_new_items(&sel[..2], &obs, 7, Some(3));
        let (_, word) = &items[0];
        assert_eq!(word.kind, "word");
        let ctx = word.context.as_ref().unwrap();
        assert_eq!(
            (ctx.sentence.as_deref(), ctx.conversation_id, ctx.article_id),
            (Some("What does deployment mean?"), Some(7), Some(3))
        );
        let (_, corr) = &items[1];
        assert_eq!(
            (corr.kind.as_str(), corr.meaning_simple.as_deref()),
            ("correction", Some("Past tense."))
        );
        let notes: Value = serde_json::from_str(corr.notes.as_deref().unwrap()).unwrap();
        assert_eq!(notes["original"], "Yesterday I deploy the app.");
    }

    #[test]
    fn stats_count_distinct_items() {
        let conv = Conversation {
            id: 1,
            article_id: None,
            article_title: None,
            settings: json!({}),
            started_at: "t".into(),
            ended_at: None,
            user_speaking_seconds: 135,
            review_status: "pending".into(),
            corrections: 1,
        };
        let s = stats(&conv, &sample());
        assert_eq!(
            s,
            ReviewStats {
                speaking_minutes: 2.3,
                new_words: 2,
                corrections: 1,
                drills: 1
            }
        );
    }
}
