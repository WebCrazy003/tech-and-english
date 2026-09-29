//! Template "why you may find it interesting" text (P1). P3 may replace it with LLM text.

/// `topics`: (name, relevance), any order.
pub fn template(topics: &[(String, f64)], hn_points: Option<i64>, source_affinity: f64) -> String {
    let mut parts = Vec::new();
    let mut matched: Vec<&(String, f64)> = topics.iter().filter(|(_, r)| *r >= 0.3).collect();
    matched.sort_by(|a, b| b.1.total_cmp(&a.1));
    if !matched.is_empty() {
        let names: Vec<&str> = matched.iter().take(2).map(|(n, _)| n.as_str()).collect();
        parts.push(format!("Matches your topics: {}", names.join(", ")));
    }
    if let Some(p) = hn_points.filter(|p| *p >= 50) {
        parts.push(format!("{p} points on Hacker News"));
    }
    if source_affinity >= 0.3 {
        parts.push("From a source you read often".to_string());
    }
    if parts.is_empty() {
        "A fresh story from your feeds".to_string()
    } else {
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_parts() {
        let t = vec![
            ("LLMs".to_string(), 0.5),
            ("AI Agents".to_string(), 0.9),
            ("Apple".to_string(), 0.2),
        ];
        assert_eq!(
            template(&t, Some(412), 0.4),
            "Matches your topics: AI Agents, LLMs · 412 points on Hacker News · From a source you read often"
        );
        assert_eq!(template(&[], Some(10), 0.0), "A fresh story from your feeds");
    }
}
