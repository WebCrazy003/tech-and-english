//! Local checks with no LLM (P5 dev spec §7.2, §8.4): did the learner repeat the corrected
//! sentence, and did whisper hear the drill word?

/// Contractions expanded before comparing, so "it's" and "it is" count as the same words.
const CONTRACTIONS: &[(&str, &str)] = &[
    ("it's", "it is"),
    ("that's", "that is"),
    ("there's", "there is"),
    ("what's", "what is"),
    ("he's", "he is"),
    ("she's", "she is"),
    ("let's", "let us"),
    ("don't", "do not"),
    ("doesn't", "does not"),
    ("didn't", "did not"),
    ("isn't", "is not"),
    ("aren't", "are not"),
    ("wasn't", "was not"),
    ("weren't", "were not"),
    ("haven't", "have not"),
    ("hasn't", "has not"),
    ("hadn't", "had not"),
    ("can't", "cannot"),
    ("couldn't", "could not"),
    ("won't", "will not"),
    ("wouldn't", "would not"),
    ("shouldn't", "should not"),
    ("i'm", "i am"),
    ("you're", "you are"),
    ("we're", "we are"),
    ("they're", "they are"),
    ("i've", "i have"),
    ("you've", "you have"),
    ("we've", "we have"),
    ("they've", "they have"),
    ("i'll", "i will"),
    ("you'll", "you will"),
    ("we'll", "we will"),
    ("they'll", "they will"),
    ("it'll", "it will"),
    ("i'd", "i would"),
    ("you'd", "you would"),
    ("we'd", "we would"),
    ("they'd", "they would"),
];

const FILLERS: &[&str] = &["um", "uh", "er", "erm", "uhm", "hmm", "ah"];

/// Lowercase, expand contractions, strip punctuation, drop fillers and stutters ("I I think").
pub fn normalize(s: &str) -> String {
    tokens(s).join(" ")
}

fn tokens(s: &str) -> Vec<String> {
    let lower = s.to_lowercase().replace(['’', '‘'], "'");
    let mut out: Vec<String> = Vec::new();
    for raw in lower.split(|c: char| c.is_whitespace() || c == '-' || c == '/') {
        let w: String = raw.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect();
        let w = w.trim_matches('\'');
        if w.is_empty() || FILLERS.contains(&w) {
            continue;
        }
        let expanded = CONTRACTIONS
            .iter()
            .find(|(c, _)| *c == w)
            .map(|(_, e)| e.to_string())
            .unwrap_or_else(|| w.replace('\'', ""));
        for part in expanded.split(' ') {
            if out.last().map(String::as_str) != Some(part) {
                out.push(part.to_string());
            }
        }
    }
    out
}

fn levenshtein(a: &[String], b: &[String]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(x != y)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// 1 − word edit distance / the longer length (1.0 = the same words).
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (tokens(a), tokens(b));
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    1.0 - levenshtein(&a, &b) as f64 / longest as f64
}

pub const REPEAT_PASS: f64 = 0.85;

pub fn repeat_passes(heard: &str, target: &str) -> bool {
    similarity(heard, target) >= REPEAT_PASS
}

/// The words that differ between two sentences (after the common start and end).
pub fn changed_words(a: &str, b: &str) -> (Vec<String>, Vec<String>) {
    let (a, b) = (tokens(a), tokens(b));
    let start = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let end = a[start..]
        .iter()
        .rev()
        .zip(b[start..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    (a[start..a.len() - end].to_vec(), b[start..b.len() - end].to_vec())
}

/// `heard` is the target word, or its plural / -ed / -ing form.
fn word_variant(heard: &str, target: &str) -> bool {
    if heard == target {
        return true;
    }
    let Some(rest) = heard.strip_prefix(target) else {
        // "use" → "using", "try" → "tried"/"tries"
        let stem_e = target.strip_suffix('e').map(|s| format!("{s}ing"));
        let stem_y = target.strip_suffix('y');
        return stem_e.as_deref() == Some(heard)
            || stem_y.is_some_and(|s| heard == format!("{s}ied") || heard == format!("{s}ies"));
    };
    matches!(rest, "s" | "es" | "d" | "ed" | "ing")
}

/// Did whisper hear the drill word (one or more words) somewhere in the transcript?
pub fn contains_word(transcript: &str, target: &str) -> bool {
    let (heard, want) = (tokens(transcript), tokens(target));
    if want.is_empty() || heard.len() < want.len() {
        return false;
    }
    heard
        .windows(want.len())
        .any(|w| w.iter().zip(&want).all(|(h, t)| word_variant(h, t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_contraction_variants_pass() {
        assert!(repeat_passes(
            "Yesterday I deployed the application.",
            "Yesterday I deployed the application."
        ));
        assert!(repeat_passes(
            "It's running on my laptop",
            "It is running on my laptop."
        ));
        assert!(repeat_passes(
            "I don't agree with this idea",
            "I do not agree with this idea."
        ));
    }

    #[test]
    fn a_missing_ed_fails_in_a_five_word_sentence() {
        // 1 of 5 words differs → 0.8 < 0.85
        let s = similarity("Yesterday I deploy the app", "Yesterday I deployed the app.");
        assert!((s - 0.8).abs() < 1e-9);
        assert!(!repeat_passes(
            "Yesterday I deploy the app",
            "Yesterday I deployed the app."
        ));
    }

    #[test]
    fn fillers_punctuation_and_stutters_are_ignored() {
        assert!(repeat_passes(
            "Um, yesterday I, I deployed the uh application",
            "Yesterday I deployed the application."
        ));
        assert_eq!(normalize("Hello, WORLD!  It’s ok."), "hello world it is ok");
        assert_eq!(similarity("", ""), 1.0);
        assert_eq!(similarity("hello", ""), 0.0);
    }

    #[test]
    fn changed_words_are_found() {
        let (a, b) = changed_words(
            "I have used Python since three years.",
            "I have used Python for three years.",
        );
        assert_eq!((a, b), (vec!["since".to_string()], vec!["for".to_string()]));
        let (a, b) = changed_words("I am agree", "I agree");
        assert_eq!((a, b), (vec!["am".to_string()], vec![]));
    }

    #[test]
    fn drill_word_matching() {
        assert!(contains_word("Scalability.", "scalability"));
        assert!(contains_word("I said scalability now", "scalability"));
        assert!(contains_word("deployed", "deploy"));
        assert!(contains_word("containers", "container"));
        assert!(contains_word("using", "use"));
        assert!(contains_word("queries", "query"));
        assert!(contains_word("real time systems", "real-time"));
        assert!(!contains_word("stability", "scalability"));
        assert!(!contains_word("", "scalability"));
    }
}
