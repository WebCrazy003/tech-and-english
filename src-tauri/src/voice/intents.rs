//! Requests handled without the LLM (P5 dev spec §7.1). The whole utterance must match, so the
//! phrases don't trigger inside normal sentences.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Repeat,
    Slower,
    Faster,
    End,
    Pronounce(String),
    /// "What does X mean?": logged as an unknown word, then answered by the LLM as usual.
    Define(String),
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid intent regex")
}

static REPEAT: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(can you |could you )?(please )?(say (that|it) again|repeat( that| it)?|pardon|sorry what|what did you say)( please)?$",
    )
});
static SLOWER: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        re(r"^(can you |could you )?(please )?(speak|talk) (more )?slow(ly|er)( please)?$"),
        re(r"^slower( please)?$"),
    ]
});
static FASTER: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        re(r"^(can you |could you )?(please )?(speak|talk) faster( please)?$"),
        re(r"^faster( please)?$"),
    ]
});
static END: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        re(r"^(stop|end|finish)( the)? (conversation|session|talking)( please)?$"),
        re(r"^(let's|lets) stop( here)?$"),
        re(r"^stop( please)?$"),
    ]
});
static PRONOUNCE: LazyLock<Regex> =
    LazyLock::new(|| re(r"^how (do|should|can) (i|you|we) (say|pronounce) (the word )?(?P<w>.+)$"));
static DEFINE: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        re(r"(what does|what's the meaning of|what is the meaning of) (?P<w>.+?)( mean)?$"),
        re(r"^what is (?P<w>[a-z-]+)$"),
    ]
});

/// Lowercase; keep letters, digits, apostrophes and hyphens; collapse spaces.
pub fn clean(text: &str) -> String {
    let lower = text.to_lowercase().replace(['’', '‘'], "'");
    let kept: String = lower
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' || c == '-' {
                c
            } else {
                ' '
            }
        })
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn word(m: &str) -> Option<String> {
    let w = m
        .trim()
        .trim_start_matches("the word ")
        .trim_matches(|c: char| c == '\'' || c == '-' || c.is_whitespace());
    (!w.is_empty() && w.split(' ').count() <= 4).then(|| w.to_string())
}

pub fn detect(text: &str) -> Option<Intent> {
    let t = clean(text);
    if t.is_empty() {
        return None;
    }
    if REPEAT.is_match(&t) {
        return Some(Intent::Repeat);
    }
    if SLOWER.iter().any(|r| r.is_match(&t)) {
        return Some(Intent::Slower);
    }
    if FASTER.iter().any(|r| r.is_match(&t)) {
        return Some(Intent::Faster);
    }
    if END.iter().any(|r| r.is_match(&t)) {
        return Some(Intent::End);
    }
    if let Some(w) = PRONOUNCE.captures(&t).and_then(|c| word(&c["w"])) {
        return Some(Intent::Pronounce(w));
    }
    DEFINE
        .iter()
        .find_map(|r| r.captures(&t))
        .and_then(|c| word(&c["w"]))
        .map(Intent::Define)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Option<Intent> {
        detect(s)
    }

    #[test]
    fn positives() {
        for s in [
            "Say that again.",
            "Could you repeat that, please?",
            "Pardon?",
            "Sorry, what?",
            "Repeat please",
        ] {
            assert_eq!(d(s), Some(Intent::Repeat), "{s}");
        }
        for s in [
            "Speak slower, please.",
            "Please talk more slowly",
            "Slower please",
            "Can you speak slower?",
        ] {
            assert_eq!(d(s), Some(Intent::Slower), "{s}");
        }
        for s in ["Speak faster", "Faster!"] {
            assert_eq!(d(s), Some(Intent::Faster), "{s}");
        }
        for s in [
            "Stop.",
            "End the conversation.",
            "Let's stop here.",
            "Finish the session",
        ] {
            assert_eq!(d(s), Some(Intent::End), "{s}");
        }
        assert_eq!(
            d("How do I say \"scalability\"?"),
            Some(Intent::Pronounce("scalability".into()))
        );
        assert_eq!(
            d("How can I pronounce the word Kubernetes?"),
            Some(Intent::Pronounce("kubernetes".into()))
        );
        assert_eq!(
            d("What does deployment mean?"),
            Some(Intent::Define("deployment".into()))
        );
        assert_eq!(
            d("Sorry, what's the meaning of rate limiting?"),
            Some(Intent::Define("rate limiting".into()))
        );
        assert_eq!(d("What is inference?"), Some(Intent::Define("inference".into())));
    }

    #[test]
    fn negatives_inside_longer_sentences() {
        for s in [
            "I want to repeat that experiment tomorrow.",
            "We should stop the conversation about budgets and talk about code.",
            "The old servers were slower please believe me.",
            "Our team decided to speak faster in meetings",
            "What is the best way to learn Rust?",
            "I don't know how do I say this in English but it's hard",
            "",
            "   ",
        ] {
            let got = d(s);
            assert!(
                !matches!(
                    got,
                    Some(Intent::Repeat | Intent::Slower | Intent::Faster | Intent::End | Intent::Pronounce(_))
                ),
                "{s} → {got:?}"
            );
        }
        assert_eq!(
            d("What is the best way to learn Rust?"),
            None,
            "multi-word 'what is' is not a define"
        );
    }
}
