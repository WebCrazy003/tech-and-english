//! English difficulty and reading time, without AI (SPEC §8.2).
//! Flesch Reading Ease + the share of words outside the NGSL common-word list.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

/// New General Service List 1.2 (CC BY-SA 4.0), all word forms, lowercase.
static COMMON: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    include_str!("../../resources/ngsl.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect()
});

static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[.!?]+(\s|$)").unwrap());

/// B1 learners read about 150 words per minute.
pub const B1_WORDS_PER_MINUTE: f64 = 150.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Easy,
    Medium,
    Hard,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Easy => "easy",
            Level::Medium => "medium",
            Level::Hard => "hard",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DifficultyReport {
    pub word_count: u32,
    pub reading_minutes: u32,
    pub fre: f64,
    pub rare_ratio: f64,
    pub level: Level,
}

pub fn reading_minutes(word_count: u32) -> u32 {
    ((word_count as f64 / B1_WORDS_PER_MINUTE).round() as u32).max(1)
}

pub fn syllables(word: &str) -> u32 {
    let w: String = word
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase())
        .collect();
    if w.is_empty() {
        return 1;
    }
    let is_vowel = |c: char| "aeiouy".contains(c);
    let chars: Vec<char> = w.chars().collect();
    let mut groups = 0;
    let mut prev = false;
    for &c in &chars {
        let v = is_vowel(c);
        if v && !prev {
            groups += 1;
        }
        prev = v;
    }
    let n = chars.len();
    let ends_le_after_consonant = n >= 3 && w.ends_with("le") && !is_vowel(chars[n - 3]);
    if w.ends_with('e') && !ends_le_after_consonant && groups > 1 {
        groups -= 1;
    }
    groups.max(1)
}

/// Product and technical names don't make a text harder to read for this user.
fn is_name_like(token: &str) -> bool {
    let letters: Vec<char> = token.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return true;
    }
    let all_caps = letters.iter().all(|c| c.is_uppercase());
    if all_caps && (2..=5).contains(&letters.len()) {
        return true; // AWS, LLM, SQL
    }
    // CamelCase: an uppercase letter after a lowercase one (DuckDB, PostgreSQL, GitHub)
    letters.windows(2).any(|w| w[0].is_lowercase() && w[1].is_uppercase())
}

fn is_common(word: &str) -> bool {
    if COMMON.contains(word) {
        return true;
    }
    // Fallback lemmatizer for forms the list may miss.
    let candidates = [
        word.strip_suffix("ies").map(|s| format!("{s}y")),
        word.strip_suffix("ied").map(|s| format!("{s}y")),
        word.strip_suffix("es").map(str::to_string),
        word.strip_suffix('s').map(str::to_string),
        word.strip_suffix("ed").map(str::to_string),
        word.strip_suffix('d').map(str::to_string),
        word.strip_suffix("ing").map(str::to_string),
        word.strip_suffix("ly").map(str::to_string),
        word.strip_suffix("er").map(str::to_string),
        word.strip_suffix("est").map(str::to_string),
    ];
    candidates.into_iter().flatten().any(|c| COMMON.contains(c.as_str()))
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’' || c == '-'))
        .map(|t| t.trim_matches(|c: char| c == '\'' || c == '’' || c == '-'))
        .filter(|t| t.chars().any(|c| c.is_alphanumeric()))
}

pub fn analyze(text: &str) -> DifficultyReport {
    let words: Vec<&str> = tokens(text).collect();
    let word_count = words.len() as u32;
    let alpha: Vec<&str> = words
        .iter()
        .copied()
        .filter(|w| w.chars().any(|c| c.is_alphabetic()))
        .collect();

    let sentences = SENTENCE_END
        .split(text)
        .filter(|s| tokens(s).count() >= 3)
        .count()
        .max(1) as f64;
    let syl: u32 = alpha.iter().map(|w| syllables(w)).sum();
    let wc = (alpha.len().max(1)) as f64;
    let fre = 206.835 - 1.015 * (wc / sentences) - 84.6 * (syl as f64 / wc);

    let checked: Vec<String> = alpha
        .iter()
        .filter(|w| {
            !is_name_like(w)
                && w.chars().count() >= 3
                && w.chars()
                    .all(|c| c.is_alphabetic() || c == '\'' || c == '’' || c == '-')
        })
        .map(|w| w.to_lowercase().replace('’', "'"))
        .collect();
    let rare = checked.iter().filter(|w| !w.split('-').all(is_common)).count();
    let rare_ratio = if checked.is_empty() {
        0.0
    } else {
        rare as f64 / checked.len() as f64
    };

    let level = if fre >= 60.0 && rare_ratio < 0.15 {
        Level::Easy
    } else if fre < 40.0 || rare_ratio >= 0.25 {
        Level::Hard
    } else {
        Level::Medium
    };
    DifficultyReport {
        word_count,
        reading_minutes: reading_minutes(word_count),
        fre,
        rare_ratio,
        level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syllable_counts() {
        for (w, n) in [
            ("cat", 1),
            ("table", 2),
            ("make", 1),
            ("scalability", 5),
            ("inference", 3),
            ("the", 1),
            ("data", 2),
        ] {
            assert_eq!(syllables(w), n, "{w}");
        }
    }

    #[test]
    fn simple_text_is_easy() {
        let r = analyze("We went to the new house. It was big and good. They like to work there every day.");
        assert_eq!(r.level, Level::Easy, "{r:?}");
    }

    #[test]
    fn dense_technical_text_is_hard() {
        let r = analyze(
            "Distributed stream-processing architectures necessitate idempotent materialization semantics, \
             notwithstanding heterogeneous orchestration substrates and asynchronous checkpointing.",
        );
        assert_eq!(r.level, Level::Hard, "{r:?}");
    }

    #[test]
    fn names_and_acronyms_are_not_rare_words() {
        let r = analyze("We use AWS and DuckDB with GitHub every day. It is good for the team.");
        assert!(r.rare_ratio < 0.1, "{r:?}");
    }

    #[test]
    fn reading_time() {
        assert_eq!(reading_minutes(0), 1);
        assert_eq!(reading_minutes(900), 6);
        let r = analyze(&"word ".repeat(300));
        assert_eq!(r.word_count, 300);
        assert_eq!(r.reading_minutes, 2);
    }

    #[test]
    fn word_list_loaded() {
        assert!(COMMON.len() > 10_000);
        assert!(is_common("the") && is_common("working") && is_common("studies"));
        assert!(!is_common("idempotent"));
    }
}
