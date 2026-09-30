//! Meanings from the macOS built-in dictionary (Dictionary Services), offline (P4 dev spec §3.1).

use std::ffi::c_void;

use core_foundation::base::{CFRange, TCFType};
use core_foundation::string::{CFString, CFStringRef};

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    /// NULL if the default dictionary has no entry.
    fn DCSCopyTextDefinition(dictionary: *const c_void, text: CFStringRef, range: CFRange) -> CFStringRef;
}

/// Raw plain-text definition of `term` from the user's default dictionary.
pub fn lookup_raw(term: &str) -> Option<String> {
    let term = term.trim();
    if term.is_empty() {
        return None;
    }
    let text = CFString::new(term);
    let range = CFRange::init(0, text.char_len());
    // SAFETY: `text` lives for the call; a non-null result follows the Create rule, so we own it.
    let out = unsafe { DCSCopyTextDefinition(std::ptr::null(), text.as_concrete_TypeRef(), range) };
    if out.is_null() {
        return None;
    }
    let s = unsafe { CFString::wrap_under_create_rule(out) };
    Some(s.to_string())
}

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

/// A parsed dictionary entry (shown in the word popup).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DictEntry {
    pub headword: String,
    /// "in·fer·ence", when the dictionary shows it.
    pub syllables: Option<String>,
    /// As written in the dictionary (IPA-like).
    pub pronunciation: Option<String>,
    /// "noun", "verb", …, or "phrase".
    pub part_of_speech: Option<String>,
    /// At most 2 short definitions.
    pub senses: Vec<String>,
    /// Example sentences from the dictionary (at most 2).
    pub examples: Vec<String>,
    /// False when the text had an unusual format: `senses` then holds the start of the raw text.
    pub parsed: bool,
}

const MAX_SENSE: usize = 200;
const MAX_RAW: usize = 300;
const POS: &str = r"phrasal verb|noun|verb|adjective|adverb|pronoun|preposition|conjunction|exclamation|determiner|abbreviation|prefix|suffix|combining form|symbol";

static SECTION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:PHRASES|PHRASAL VERBS|DERIVATIVES|ORIGIN|USAGE|NOTE)\b").unwrap());
static POS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?:^|\s)({POS})\b")).unwrap());
/// Where a second part of speech starts: ". noun an …" or " noun 1 …".
static NEXT_POS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:\.\s+(?:noun|verb|adjective|adverb|pronoun|exclamation)\s)|(?:\s(?:noun|verb|adjective|adverb)\s+1\s)",
    )
    .unwrap()
});
static NUMBERED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[.)!?]\s+)(\d{1,2})\s+").unwrap());
static LEAD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\s*(?:\[[^\]]*\]|\([^)]*\)|informal|formal|archaic|dated|mainly [A-Z][a-z]+ English|[A-Z][a-z]+ English)\s*)+").unwrap()
});
/// A subject label at the start of a sense: "Computing the delay …".
static DOMAIN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Z][a-z]+)\s+(?:\([^)]*\)\s*)?([a-z(])").unwrap());

fn alnum(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Cut at a sentence end (or a word) before `max` chars.
fn cut(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    match head.rfind(". ").or_else(|| head.rfind("; ")) {
        Some(i) if i > max / 2 => head[..=i].trim().to_string(),
        _ => format!("{}…", head[..head.rfind(' ').unwrap_or(head.len())].trim_end()),
    }
}

/// Remove grammar notes and labels at the start; keep a subject label as "(Computing)".
fn clean_sense(s: &str) -> (String, Option<String>) {
    let s = LEAD_RE.replace(s.trim(), "");
    let s = match DOMAIN_RE.captures(&s) {
        Some(c) if !matches!(&c[1], "The" | "A" | "An") => {
            let rest = &s[c.get(2).unwrap().start()..];
            format!("({}) {}", &c[1], rest)
        }
        _ => s.to_string(),
    };
    // "definition: example | example"
    let (def, example) = match s.split_once(':') {
        Some((d, ex)) => {
            let ex = ex.split(" | ").next().unwrap_or("").trim().trim_end_matches(['.', ' ']);
            (d, (!ex.is_empty() && ex.len() > 3).then(|| ex.to_string()))
        }
        None => (s.as_str(), None),
    };
    let def = def.trim().trim_end_matches(['.', ';', ' ']);
    (cut(def, MAX_SENSE), example.map(|e| cut(&e, MAX_SENSE)))
}

fn raw_entry(raw: &str) -> DictEntry {
    let headword = raw.split_whitespace().next().unwrap_or("").to_string();
    DictEntry {
        headword,
        syllables: None,
        pronunciation: None,
        part_of_speech: None,
        senses: vec![cut(raw, MAX_RAW)],
        examples: vec![],
        parsed: false,
    }
}

/// Parse the plain-text output of Dictionary Services:
/// `headword syl·la·bles | pronunciation | [labels] part-of-speech [1] sense: example • sub-sense … 2 …`.
pub fn parse(raw: &str) -> DictEntry {
    let raw = raw.trim();
    let mut parts = raw.splitn(3, " | ");
    let (head, pron, body) = match (parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(p), Some(b)) => (h.trim(), Some(p.trim()), b),
        _ => {
            // No pronunciation: "word noun …".
            let Some(m) = POS_RE.find(raw) else {
                return raw_entry(raw);
            };
            (raw[..m.start()].trim(), None, &raw[m.start()..])
        }
    };
    let body = match SECTION_RE.find(body) {
        Some(m) => &body[..m.start()],
        None => body,
    };
    let Some(pos) = POS_RE.captures(body).filter(|c| c.get(1).unwrap().start() < 80) else {
        return raw_entry(raw);
    };
    let pos_m = pos.get(1).unwrap();
    let mut rest = &body[pos_m.end()..];
    if let Some(m) = NEXT_POS_RE.find(rest) {
        rest = &rest[..m.start() + 1];
    }
    let rest = LEAD_RE.replace(rest.trim(), "").to_string();

    // Numbered senses ("1 … 2 …") must count up: "42 marathons" is not sense 42.
    let mut groups: Vec<&str> = Vec::new();
    let mut expect = 1;
    let mut start = None;
    for c in NUMBERED_RE.captures_iter(&rest) {
        let n: u32 = c[1].parse().unwrap_or(0);
        if n == expect {
            if let Some(s) = start {
                groups.push(&rest[s..c.get(0).unwrap().start() + 1]);
            }
            start = Some(c.get(0).unwrap().end());
            expect += 1;
        }
    }
    let mut senses = Vec::new();
    let mut examples = Vec::new();
    let mut take = |text: &str| {
        let (d, ex) = clean_sense(text);
        if !d.is_empty() && senses.len() < 2 {
            senses.push(d);
        }
        if let Some(e) = ex
            && examples.len() < 2
        {
            examples.push(e);
        }
    };
    match start {
        Some(s) if expect > 1 => {
            groups.push(&rest[s..]);
            for g in groups.iter().take(2) {
                take(g.split(" • ").next().unwrap_or(g));
            }
        }
        _ => {
            // Unnumbered: the main sense and its first "•" sub-sense.
            for part in rest.split(" • ").take(2) {
                take(part);
            }
        }
    }
    if senses.is_empty() {
        return raw_entry(raw);
    }
    let (headword, syllables) = match head.rsplit_once(' ') {
        Some((h, syl)) if syl.contains('·') => (h.to_string(), Some(syl.to_string())),
        _ => (head.to_string(), None),
    };
    DictEntry {
        headword: headword.trim_end_matches(|c: char| c.is_ascii_digit()).to_string(),
        syllables,
        pronunciation: pron.filter(|p| !p.is_empty()).map(str::to_string),
        part_of_speech: Some(pos_m.as_str().to_string()),
        senses,
        examples,
        parsed: true,
    }
}

/// A phrase from the PHRASES section of the first word's entry ("rule of thumb" inside "rule").
fn find_phrase(raw: &str, phrase: &str) -> Option<DictEntry> {
    let sec = raw.find("PHRASES")?;
    let lower = raw.to_lowercase();
    let needle = format!(" {} ", phrase.to_lowercase());
    let at = lower[sec..].find(&needle)? + sec + needle.len();
    let mut rest = &raw[at..];
    let mut pronunciation = None;
    if let Some(r) = rest.strip_prefix("| ")
        && let Some((p, after)) = r.split_once(" | ")
    {
        pronunciation = Some(p.trim().to_string());
        rest = after;
    }
    let end = rest
        .find(':')
        .or_else(|| rest.find(". "))
        .unwrap_or(rest.len().min(MAX_SENSE));
    let (def, _) = clean_sense(&rest[..end]);
    let example = rest[end..]
        .strip_prefix(':')
        .and_then(|e| e.split(['.', '|']).next())
        .map(|e| cut(e, MAX_SENSE))
        .filter(|e| e.len() > 3);
    (!def.is_empty()).then(|| DictEntry {
        headword: phrase.to_string(),
        syllables: None,
        pronunciation,
        part_of_speech: Some("phrase".into()),
        senses: vec![def],
        examples: example.into_iter().collect(),
        parsed: true,
    })
}

/// The entry for `term`, or `None` if the dictionary has none. For a phrase, Dictionary Services
/// returns the first word's entry; then the phrase is looked up in that entry's PHRASES section.
pub fn entry_for(term: &str, raw: &str) -> Option<DictEntry> {
    let term = term.trim();
    let multi = term.split_whitespace().count() > 1;
    let e = parse(raw);
    if !multi || alnum(&e.headword) == alnum(term) {
        return Some(e);
    }
    find_phrase(raw, term)
}

/// Dictionary lookup (blocking FFI; call it from `spawn_blocking`).
pub fn lookup(term: &str) -> Option<DictEntry> {
    let raw = lookup_raw(term)?;
    entry_for(term, &raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/dictionary/{name}.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    #[test]
    fn a_word_with_one_sense_and_a_sub_sense() {
        let e = parse(&fixture("inference"));
        assert_eq!(e.headword, "inference");
        assert_eq!(e.syllables.as_deref(), Some("in·fer·ence"));
        assert_eq!(e.pronunciation.as_deref(), Some("ˈinf(ə)rəns"));
        assert_eq!(e.part_of_speech.as_deref(), Some("noun"));
        assert_eq!(
            e.senses,
            vec![
                "a conclusion reached on the basis of evidence and reasoning",
                "the process of inferring something"
            ]
        );
        assert_eq!(
            e.examples[0],
            "researchers are entrusted with drawing inferences from the data"
        );
        assert!(e.parsed);
    }

    #[test]
    fn numbered_senses_with_a_subject_label() {
        let e = parse(&fixture("latency"));
        assert_eq!(e.senses.len(), 2);
        assert!(e.senses[0].starts_with("the state of existing but not yet being developed"));
        assert!(
            e.senses[1].starts_with("(Computing) the delay before a transfer of data begins"),
            "{}",
            e.senses[1]
        );
    }

    #[test]
    fn a_verb_with_many_senses_and_grammar_notes() {
        let e = parse(&fixture("run"));
        assert_eq!(e.headword, "run");
        assert_eq!(e.part_of_speech.as_deref(), Some("verb"));
        assert_eq!(e.senses.len(), 2);
        assert!(
            e.senses[0].starts_with("move at a speed faster than a walk"),
            "{}",
            e.senses[0]
        );
        assert!(
            e.senses[1].starts_with("pass or cause to pass quickly"),
            "{}",
            e.senses[1]
        );
        assert!(e.senses.iter().all(|s| s.chars().count() <= MAX_SENSE + 1));
        assert!(!e.senses.iter().any(|s| s.contains("PHRASES") || s.contains("ORIGIN")));
    }

    #[test]
    fn hyphen_abbreviation_and_second_part_of_speech() {
        let t = parse(&fixture("trade-off"));
        assert_eq!(
            (t.headword.as_str(), t.part_of_speech.as_deref()),
            ("trade-off", Some("noun"))
        );
        assert_eq!(
            t.senses,
            vec!["a balance achieved between two desirable but incompatible features; a compromise"]
        );
        let a = parse(&fixture("api"));
        assert!(
            a.senses[0].starts_with("(Computing) a set of functions and procedures"),
            "{}",
            a.senses[0]
        );
        assert!(!a.senses[0].contains("ORIGIN"));
        let i = parse(&fixture("idempotent"));
        assert_eq!(i.part_of_speech.as_deref(), Some("adjective"));
        assert_eq!(i.senses.len(), 1, "the noun part is not mixed in: {:?}", i.senses);
        let p = parse(&fixture("pipeline"));
        assert_eq!(p.senses.len(), 2);
        assert!(
            p.senses[1].starts_with("(Computing) a linear sequence"),
            "{}",
            p.senses[1]
        );
    }

    #[test]
    fn a_phrase_is_found_in_the_first_words_entry() {
        let raw = fixture("rule-of-thumb");
        let e = entry_for("rule of thumb", &raw).expect("phrase entry");
        assert_eq!(e.headword, "rule of thumb");
        assert_eq!(e.part_of_speech.as_deref(), Some("phrase"));
        assert_eq!(e.pronunciation.as_deref(), Some("ˌro͞ol əv ˈTHəm"));
        assert_eq!(
            e.senses,
            vec!["a broadly accurate guide or principle, based on experience or practice rather than theory"]
        );
        assert!(
            entry_for("rule the world", &raw).is_none(),
            "a phrase that is not in the entry"
        );
        assert!(
            entry_for("rule of law", &raw).is_some_and(|e| e.senses[0].starts_with("the restriction of the arbitrary"))
        );
        assert_eq!(
            entry_for("rules", &raw).unwrap().headword,
            "rule",
            "one word: inflections are fine"
        );
        assert!(
            entry_for("trade off", &fixture("trade-off")).is_some(),
            "hyphen vs space"
        );
    }

    #[test]
    fn no_pronunciation_non_english_and_unknown_formats() {
        let e = parse(&fixture("no-pronunciation"));
        assert_eq!(e.headword, "bytecode");
        assert_eq!(e.pronunciation, None);
        assert!(e.senses[0].starts_with("(Computing) a form of instruction code"));
        for name in ["non-english", "unknown"] {
            let e = parse(&fixture(name));
            assert!(!e.parsed, "{name}");
            assert_eq!(e.part_of_speech, None);
            assert!(e.senses[0].chars().count() <= MAX_RAW + 1);
        }
    }

    /// cargo test --lib dictionary::tests::live_lookup -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_lookup() {
        let t = std::time::Instant::now();
        let e = lookup("inferences").expect("in the macOS dictionary");
        println!("{e:?} in {:?} (first lookup)", t.elapsed());
        let words = [
            "latency",
            "throughput",
            "pipeline",
            "robust",
            "deploy",
            "benchmark",
            "leverage",
            "scalability",
        ];
        let t = std::time::Instant::now();
        for w in words {
            assert!(lookup(w).is_some(), "{w}");
        }
        println!("warm lookups: {:?} each", t.elapsed() / words.len() as u32);
        assert_eq!(e.headword, "inference");
        assert!(lookup("asdfqwer").is_none());
        assert!(lookup("on the fly").is_some_and(|e| e.part_of_speech.as_deref() == Some("phrase")));
    }

    /// Prints real outputs (used to write the parser fixtures).
    ///   cargo test --lib dictionary::tests::print_raw -- --ignored --nocapture
    /// Saves real outputs as parser fixtures.
    #[test]
    #[ignore]
    fn write_fixtures() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dictionary");
        for (term, file) in [
            ("inference", "inference"),
            ("latency", "latency"),
            ("run", "run"),
            ("trade-off", "trade-off"),
            ("API", "api"),
            ("rule of thumb", "rule-of-thumb"),
            ("pipeline", "pipeline"),
            ("idempotent", "idempotent"),
        ] {
            std::fs::write(format!("{dir}/{file}.txt"), lookup_raw(term).unwrap()).unwrap();
        }
    }

    #[test]
    #[ignore]
    fn print_raw() {
        for t in [
            "rule of thumb",
            "trade-off",
            "on the fly",
            "API",
            "deploy",
            "benchmark",
            "bottleneck",
            "e.g.",
            "pipeline",
            "robust",
            "leverage",
        ] {
            println!("=== {t}\n{:?}\n", lookup_raw(t));
        }
    }
}
