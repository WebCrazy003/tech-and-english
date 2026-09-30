//! Learning score: is this article a tutorial, guide or explainer? (SPEC §7.11). Pure, no LLM.

use std::sync::LazyLock;

use regex::Regex;

use super::topics::{bounded_regex, prep};

/// An article counts as learning material from this score on.
pub const LESSON_THRESHOLD: f64 = 0.5;
/// Only the start of the description is checked (the rest is often boilerplate).
const DESCRIPTION_CHARS: usize = 300;
const LONG_BODY_WORDS: u32 = 1200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LessonKind {
    DeepDive,
    Tutorial,
    Guide,
    Explainer,
}

impl LessonKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::DeepDive => "Deep dive",
            Self::Tutorial => "Tutorial",
            Self::Guide => "Guide",
            Self::Explainer => "Explainer",
        }
    }
}

/// The one list of learning patterns (regex source, on text with hyphens turned into spaces).
const LEARNING: &[(&str, LessonKind)] = &[
    (r"deep\s+dives?", LessonKind::DeepDive),
    (r"how\s+to", LessonKind::Tutorial),
    (r"tutorials?", LessonKind::Tutorial),
    (r"step\s+by\s+step", LessonKind::Tutorial),
    (r"from\s+scratch", LessonKind::Tutorial),
    (r"hands\s+on", LessonKind::Tutorial),
    (r"walkthrough", LessonKind::Tutorial),
    (r"guides?", LessonKind::Guide),
    (r"introduction\s+to", LessonKind::Guide),
    (r"intro\s+to", LessonKind::Guide),
    (r"beginners?", LessonKind::Guide),
    (r"101", LessonKind::Guide),
    (r"best\s+practices", LessonKind::Guide),
    (r"primer", LessonKind::Guide),
    (r"cheat\s?sheets?", LessonKind::Guide),
    (r"fundamentals", LessonKind::Guide),
    (r"explained", LessonKind::Explainer),
    (r"explainer", LessonKind::Explainer),
    (r"how\s.{1,40}\sworks?", LessonKind::Explainer),
    (r"what\s+is", LessonKind::Explainer),
    (r"what\s+are", LessonKind::Explainer),
    (r"understanding", LessonKind::Explainer),
    (r"patterns", LessonKind::Explainer),
    (r"lessons\s+learned", LessonKind::Explainer),
    (r"vs\.?", LessonKind::Explainer),
];

/// News announcements and opinion pieces (checked on the title only: tutorials often mention a
/// release in their description).
const NEWS: &[&str] = &[
    r"announc(?:e|es|ed|ing)",
    r"launch(?:es|ed|ing)?",
    r"raises",
    r"acquires?",
    r"acquired",
    r"funding",
    r"now\s+available",
    r"generally\s+available",
    r"(?:is|now)\s+ga",
    r"introducing",
    r"released",
    r"release\s+notes",
    r"weekly\s+roundup",
    r"roundup",
    r"this\s+week\s+in",
    // opinion and talk, not a lesson (found in the P3 live check)
    r"the\s+future\s+of",
    r"podcast",
    r"episode",
];

/// Always course or account spam.
const SPAM: &[&str] = &[
    r"certification\s+training",
    r"job\s+guarantee",
    r"placement\s+(?:assistance|guarantee|support)",
    r"with\s+placements?",
    r"who['’]?s\s+hiring",
    r"(?:buy|buying|sell|selling)\s.{0,40}accounts?",
];
/// Spam only together with a place: "Training in Hyderabad" (but not "Distributed training in JAX").
const COURSE_WORDS: &str = r"training|courses?|classes|coaching|bootcamps?|institutes?|academy|certifications?";
const PLACES: &str = r"(?:in|at)\s+(?:hyderabad|bangalore|bengaluru|chennai|pune|noida|delhi|new\s+delhi|mumbai|kolkata|ahmedabad|gurgaon|gurugram|kochi|coimbatore|jaipur|lucknow|chandigarh|indore|nagpur|kerala|india|dubai|lahore|karachi|dhaka)|near\s+me";

struct Patterns {
    learning: Vec<(Regex, LessonKind)>,
    news: Vec<Regex>,
    spam: Vec<Regex>,
    course: Regex,
    place: Regex,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| {
    let re = |p: &str| bounded_regex(p).expect("valid learning pattern");
    Patterns {
        learning: LEARNING.iter().map(|(p, k)| (re(p), *k)).collect(),
        news: NEWS.iter().map(|p| re(p)).collect(),
        spam: SPAM.iter().map(|p| re(p)).collect(),
        course: re(COURSE_WORDS),
        place: re(PLACES),
    }
});

pub struct LearningInput<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
    /// Any feed of the article has `learning = true`.
    pub feed_learning: bool,
    /// Body word count, once extracted.
    pub word_count: Option<u32>,
}

fn desc_start(desc: Option<&str>) -> String {
    prep(&desc.unwrap_or("").chars().take(DESCRIPTION_CHARS).collect::<String>())
}

pub fn is_course_spam(text: &str) -> bool {
    let t = prep(text);
    let p = &*PATTERNS;
    p.spam.iter().any(|re| re.is_match(&t)) || (p.course.is_match(&t) && p.place.is_match(&t))
}

pub fn is_news(title: &str) -> bool {
    let t = prep(title);
    PATTERNS.news.iter().any(|re| re.is_match(&t))
}

/// (kind, in title?) of every distinct learning pattern that matches.
fn matches(i: &LearningInput) -> Vec<(LessonKind, bool)> {
    let title = prep(i.title);
    let desc = desc_start(i.description);
    PATTERNS
        .learning
        .iter()
        .filter_map(|(re, kind)| {
            if re.is_match(&title) {
                Some((*kind, true))
            } else if re.is_match(&desc) {
                Some((*kind, false))
            } else {
                None
            }
        })
        .collect()
}

/// 0..1. The first learning pattern in the title counts 0.50, every other distinct pattern 0.20
/// (patterns only in the description: at most 0.40), capped at 0.75. Then +0.35 for a learning
/// source, +0.10 for a long body, −0.30 for a news title. Spam is always 0.
pub fn learning_score(i: &LearningInput) -> f64 {
    let full = format!("{} {}", i.title, i.description.unwrap_or(""));
    if is_course_spam(&full) {
        return 0.0;
    }
    let m = matches(i);
    let in_title = m.iter().filter(|(_, t)| *t).count() as f64;
    let in_desc = m.len() as f64 - in_title;
    let patterns = if in_title > 0.0 {
        0.5 + 0.2 * (in_title - 1.0 + in_desc)
    } else {
        (0.2 * in_desc).min(0.4)
    };
    let mut score = patterns.min(0.75);
    if i.feed_learning {
        score += 0.35;
    }
    if i.word_count.is_some_and(|w| w >= LONG_BODY_WORDS) {
        score += 0.10;
    }
    if is_news(i.title) {
        score -= 0.30;
    }
    score.clamp(0.0, 1.0)
}

/// The strongest kind of learning pattern: title before description, then
/// deep dive > tutorial > guide > explainer. Used in the lesson's "why" text.
pub fn lesson_kind(i: &LearningInput) -> LessonKind {
    matches(i)
        .into_iter()
        .min_by_key(|(kind, in_title)| (!in_title, *kind))
        .map(|(k, _)| k)
        .unwrap_or(if i.word_count.is_some_and(|w| w >= LONG_BODY_WORDS) {
            LessonKind::DeepDive
        } else {
            LessonKind::Explainer
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Case {
        title: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        feed_learning: bool,
        /// "lesson" | "not" | "spam"
        expect: String,
    }

    fn input(title: &str) -> LearningInput<'_> {
        LearningInput {
            title,
            description: None,
            feed_learning: false,
            word_count: None,
        }
    }

    /// Real titles from the live feeds (2026-09-29/30).
    #[test]
    fn real_titles() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("../../tests/fixtures/learning_titles.json")).unwrap();
        assert!(cases.len() >= 25);
        let mut wrong = Vec::new();
        for c in &cases {
            let i = LearningInput {
                title: &c.title,
                description: c.description.as_deref(),
                feed_learning: c.feed_learning,
                word_count: None,
            };
            let s = learning_score(&i);
            let ok = match c.expect.as_str() {
                "lesson" => s >= LESSON_THRESHOLD,
                "not" => s < LESSON_THRESHOLD,
                "spam" => s == 0.0 && is_course_spam(&c.title),
                other => panic!("bad expect {other}"),
            };
            if !ok {
                wrong.push(format!("{:.2} {} [{}]", s, c.title, c.expect));
            }
        }
        assert!(wrong.is_empty(), "wrong:\n{}", wrong.join("\n"));
    }

    #[test]
    fn score_parts() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(learning_score(&input("How to use DuckDB")), 0.5));
        // how to + from scratch + step by step + guide → 0.5 + 0.6, capped
        assert!(close(
            learning_score(&input("How to build a chatbot from scratch: a step-by-step guide")),
            0.75
        ));
        let with_desc = LearningInput {
            description: Some("A beginner post."),
            ..input("How to use DuckDB")
        };
        assert!(close(learning_score(&with_desc), 0.5 + 0.2));
        let desc_only = LearningInput {
            description: Some("A beginner tutorial with best practices and a cheat sheet."),
            ..input("DuckDB and Parquet")
        };
        assert!(close(learning_score(&desc_only), 0.4), "description alone is capped");
        let source = LearningInput {
            feed_learning: true,
            word_count: Some(1500),
            ..input("Data contracts")
        };
        assert!(close(learning_score(&source), 0.45));
        assert!(close(learning_score(&input("Announcing a guide to Iceberg")), 0.2));
        assert_eq!(learning_score(&input("Nvidia launches a new chip")), 0.0);
    }

    #[test]
    fn word_boundaries_and_hyphens() {
        assert_eq!(learning_score(&input("Guided merge sort")), 0.0, "guided ≠ guide");
        assert_eq!(learning_score(&input("Expert-Guided rubrics")), 0.0);
        assert!(learning_score(&input("Hands-on with Iceberg tables")) >= 0.5);
        assert!(learning_score(&input("Cloud architecture anti-patterns")) >= 0.5);
        assert!(learning_score(&input("How commercial loan underwriting works")) >= 0.5);
        assert!(learning_score(&input("SQL 101")) >= 0.5);
    }

    #[test]
    fn course_spam_needs_a_place_but_ml_training_is_fine() {
        assert!(is_course_spam(
            "Python Full Stack Training in Bangalore | Learnmore Technologies 🚀"
        ));
        assert!(is_course_spam(
            "Unlock Your Future: Mastering AI Classes in Ahmedabad for 2026"
        ));
        assert!(is_course_spam("Top 2 Site To Buy Pinterest Accounts"));
        assert!(!is_course_spam("Distributed training in JAX, explained"));
        assert!(!is_course_spam("Data classes in Python: a guide"));
        assert!(!is_course_spam("A crash course in data modeling"));
        assert!(!is_course_spam("Kubernetes pod placement explained"));
        let spam_desc = LearningInput {
            description: Some("Join the best Snowflake training in Hyderabad with placement assistance."),
            ..input("How to learn Snowflake")
        };
        assert_eq!(learning_score(&spam_desc), 0.0, "spam in the description counts too");
    }

    #[test]
    fn kinds() {
        assert_eq!(
            lesson_kind(&input("Kafka deep dive: how to tune consumers")),
            LessonKind::DeepDive
        );
        assert_eq!(lesson_kind(&input("Iceberg explained: a guide")), LessonKind::Guide);
        assert_eq!(lesson_kind(&input("What is a data lakehouse?")), LessonKind::Explainer);
        let desc = LearningInput {
            description: Some("A step-by-step tutorial."),
            ..input("Understanding dbt tests")
        };
        assert_eq!(lesson_kind(&desc), LessonKind::Explainer, "title first");
        assert_eq!(lesson_kind(&input("Data contracts")).label(), "Explainer");
    }
}
