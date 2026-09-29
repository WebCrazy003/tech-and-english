//! Keyword topic matching (SPEC §7.5).

use regex::Regex;

use crate::db::repo::topics::Topic;

pub struct CompiledTopic {
    pub id: i64,
    pub name: String,
    pub priority: u8,
    keywords: Vec<Regex>,
    excluded: Vec<Regex>,
}

/// Hyphens and spaces are treated as equal ("tool-calling" == "tool calling").
fn prep(text: &str) -> String {
    text.replace('-', " ")
}

fn keyword_regex(kw: &str) -> Option<Regex> {
    let k = prep(kw.trim());
    if k.is_empty() {
        return None;
    }
    let escaped = regex::escape(&k).replace(' ', r"\s+");
    Regex::new(&format!(r"(?i)(?:^|[^\p{{L}}\p{{N}}]){escaped}(?:$|[^\p{{L}}\p{{N}}])")).ok()
}

pub fn priority_factor(priority: u8) -> f64 {
    match priority {
        1 => 0.6,
        3 => 1.0,
        _ => 0.8,
    }
}

pub fn compile(topics: &[Topic]) -> Vec<CompiledTopic> {
    topics
        .iter()
        .filter(|t| t.enabled)
        .map(|t| CompiledTopic {
            id: t.id,
            name: t.name.clone(),
            priority: t.priority,
            keywords: t.keywords.iter().filter_map(|k| keyword_regex(k)).collect(),
            excluded: t.excluded_keywords.iter().filter_map(|k| keyword_regex(k)).collect(),
        })
        .collect()
}

/// `(topic_id, relevance)` for every topic with relevance > 0.
/// `desc` should be the description plus (when known) the first 500 chars of the body.
pub fn match_topics(topics: &[CompiledTopic], title: &str, desc: Option<&str>) -> Vec<(i64, f64)> {
    let title = prep(title);
    let desc = prep(desc.unwrap_or(""));
    let mut out = Vec::new();
    for t in topics {
        if t.excluded.iter().any(|re| re.is_match(&title) || re.is_match(&desc)) {
            continue;
        }
        let title_hits = t.keywords.iter().filter(|re| re.is_match(&title)).count() as f64;
        let desc_hits = t.keywords.iter().filter(|re| re.is_match(&desc)).count() as f64;
        let raw = (0.6 * title_hits + 0.25 * desc_hits).min(1.0);
        let relevance = raw * priority_factor(t.priority);
        if relevance > 0.0 {
            out.push((t.id, relevance));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(id: i64, kws: &[&str], excl: &[&str], priority: u8) -> Topic {
        Topic {
            id,
            name: format!("t{id}"),
            keywords: kws.iter().map(|s| s.to_string()).collect(),
            excluded_keywords: excl.iter().map(|s| s.to_string()).collect(),
            priority,
            enabled: true,
            notify: false,
            notify_threshold: None,
        }
    }

    #[test]
    fn word_boundaries() {
        let t = compile(&[topic(1, &["AI"], &[], 3)]);
        assert!(match_topics(&t, "He said we maintain it", None).is_empty());
        assert_eq!(match_topics(&t, "New AI chip", None), vec![(1, 0.6)]);
        assert_eq!(match_topics(&t, "AI: the future", None), vec![(1, 0.6)]);
    }

    #[test]
    fn symbols_and_phrases() {
        let t = compile(&[topic(1, &["C++", ".NET", "tool calling"], &[], 3)]);
        assert_eq!(match_topics(&t, "Why C++ still matters", None).len(), 1);
        assert_eq!(match_topics(&t, "Porting to .NET 10", None).len(), 1);
        assert_eq!(match_topics(&t, "Better tool-calling in LLMs", None).len(), 1);
        assert_eq!(match_topics(&t, "Better tool   calling", None).len(), 1);
    }

    #[test]
    fn scoring_and_priority() {
        let t = compile(&[topic(1, &["kafka", "streaming"], &[], 2)]);
        // title: 1 hit (0.6) + desc: 2 hits (0.5) → capped at 1.0 × 0.8
        let m = match_topics(&t, "Kafka tips", Some("Kafka and streaming"));
        assert!((m[0].1 - 0.8).abs() < 1e-9);
        let low = compile(&[topic(1, &["kafka"], &[], 1)]);
        assert!((match_topics(&low, "x", Some("kafka"))[0].1 - 0.25 * 0.6).abs() < 1e-9);
    }

    #[test]
    fn exclusion_zeroes_only_that_topic() {
        let t = compile(&[topic(1, &["apple"], &["apple cider"], 2), topic(2, &["cider"], &[], 2)]);
        let m = match_topics(&t, "Apple cider recipes", None);
        assert_eq!(m, vec![(2, 0.6 * 0.8)]);
    }

    #[test]
    fn disabled_topics_are_skipped() {
        let mut tp = topic(1, &["rust"], &[], 2);
        tp.enabled = false;
        assert!(match_topics(&compile(&[tp]), "Rust 2.0", None).is_empty());
    }
}
