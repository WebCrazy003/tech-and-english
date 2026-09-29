//! URL and title normalization used for deduplication (SPEC §7.4).

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use url::Url;

use crate::error::{AppError, AppResult};

fn is_tracking_param(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    k.starts_with("utm_") || k.starts_with("mc_") || matches!(k.as_str(), "fbclid" | "gclid" | "ref" | "source")
}

/// Lowercase scheme/host, strip `www.`, drop fragment and tracking params,
/// sort the remaining params and drop a trailing `/`.
pub fn normalize_url(input: &str) -> AppResult<String> {
    let u = Url::parse(input.trim()).map_err(|_| AppError::Invalid(format!("bad url: {input}")))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(AppError::Invalid(format!("unsupported scheme: {}", u.scheme())));
    }
    let host = u
        .host_str()
        .ok_or_else(|| AppError::Invalid("url has no host".into()))?
        .to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();

    let mut pairs: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| !is_tracking_param(k))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    pairs.sort();
    let query = if pairs.is_empty() {
        String::new()
    } else {
        let mut ser = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in &pairs {
            ser.append_pair(k, v);
        }
        format!("?{}", ser.finish())
    };

    let path = u.path().trim_end_matches('/');
    Ok(format!("{}://{}{}{}{}", u.scheme(), host, port, path, query))
}

/// Host without `www.`, e.g. "github.com". Used as the source name for HN links.
pub fn host_of(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    let h = u.host_str()?.to_ascii_lowercase();
    Some(h.strip_prefix("www.").unwrap_or(&h).to_string())
}

static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
static WS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&(#x[0-9a-fA-F]+|#[0-9]+|[a-zA-Z]+);").unwrap());

fn decode_entities(s: &str) -> String {
    ENTITY_RE
        .replace_all(s, |c: &regex::Captures| {
            let e = &c[1];
            let ch = if let Some(hex) = e.strip_prefix("#x").or_else(|| e.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = e.strip_prefix('#') {
                dec.parse().ok().and_then(char::from_u32)
            } else {
                match e {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    "mdash" => Some('—'),
                    "ndash" => Some('–'),
                    "hellip" => Some('…'),
                    "rsquo" => Some('’'),
                    "lsquo" => Some('‘'),
                    "rdquo" => Some('”'),
                    "ldquo" => Some('“'),
                    _ => None,
                }
            };
            ch.map(String::from).unwrap_or_else(|| c[0].to_string())
        })
        .into_owned()
}

/// Strip tags, decode common entities, collapse whitespace.
pub fn strip_html(s: &str) -> String {
    let no_tags = TAG_RE.replace_all(s, " ");
    let decoded = decode_entities(&no_tags);
    WS_RE.replace_all(&decoded, " ").trim().to_string()
}

pub fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", s[..i].trim_end()),
        None => s.to_string(),
    }
}

const KNOWN_PUBLISHERS: &[&str] = &[
    "ars technica",
    "the verge",
    "techcrunch",
    "wired",
    "engadget",
    "zdnet",
    "the register",
    "bloomberg",
    "reuters",
    "cnbc",
    "mit technology review",
    "the new york times",
    "financial times",
    "the guardian",
    "bbc news",
];

fn alnum_lower(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn same_source(suffix: &str, source: &str) -> bool {
    let a = alnum_lower(suffix);
    let b = alnum_lower(source);
    if a.len() < 3 {
        return false;
    }
    (!b.is_empty() && (a == b || a.contains(&b) || b.contains(&a)))
        || KNOWN_PUBLISHERS.iter().any(|p| alnum_lower(p) == a)
}

/// Lowercased, punctuation-free title with a trailing " - Source" suffix removed.
pub fn title_key(title: &str, source_name: &str) -> String {
    let mut t = title.trim().to_string();
    for sep in [" - ", " | ", " — ", " – "] {
        if let Some(i) = t.rfind(sep) {
            let suffix = &t[i + sep.len()..];
            if suffix.chars().count() <= 40 && same_source(suffix, source_name) {
                t.truncate(i);
                break;
            }
        }
    }
    let cleaned: String = t
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "of", "to", "in", "for", "on", "and", "or", "with", "is", "are",
];

pub fn word_set(key: &str) -> HashSet<&str> {
    key.split(' ')
        .filter(|w| !w.is_empty() && !STOP_WORDS.contains(w))
        .collect()
}

pub fn jaccard(a: &HashSet<&str>, b: &HashSet<&str>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    inter / union
}

const ENGLISH_HINTS: &[&str] = &[
    "the", "a", "an", "of", "to", "in", "for", "on", "and", "with", "is", "are", "how", "what", "why", "your", "you",
    "i", "we", "it", "this", "that", "from", "by", "at", "as", "be", "can", "do", "does", "my", "our", "without",
    "into", "about", "using", "use", "new", "vs", "its", "not", "now", "more", "when", "will",
];
const FOREIGN_HINTS: &[&str] = &[
    "de", "la", "el", "los", "las", "del", "con", "para", "por", "una", "uno", "que", "y", "sin", "les", "des", "et",
    "une", "pour", "sur", "avec", "der", "die", "das", "und", "mit", "für", "ein", "eine", "não", "uma", "com", "os",
    "você", "il", "di", "che", "dei", "cómo", "qué", "wie", "ist", "est", "como", "sus", "nel", "zu",
];

/// Heuristic language check: the app teaches English, so non-English items are skipped.
/// Short texts and texts without clear signals count as English.
pub fn looks_english(text: &str) -> bool {
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return true;
    }
    let non_latin = letters
        .iter()
        .filter(|c| !c.is_ascii() && !('\u{00C0}'..='\u{024F}').contains(*c))
        .count();
    if non_latin * 10 > letters.len() * 3 {
        return false; // mostly CJK, Cyrillic, Arabic…
    }
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() < 4 {
        return true;
    }
    let en = words.iter().filter(|w| ENGLISH_HINTS.contains(w)).count();
    let foreign = words.iter().filter(|w| FOREIGN_HINTS.contains(w)).count();
    foreign <= en
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_url() {
        assert_eq!(
            normalize_url("HTTPS://WWW.Example.com/a/?utm_source=x&b=2&a=1#top").unwrap(),
            "https://example.com/a?a=1&b=2"
        );
        assert_eq!(normalize_url("https://example.com/").unwrap(), "https://example.com");
        assert_eq!(
            normalize_url("http://example.com:8080/x?fbclid=1").unwrap(),
            "http://example.com:8080/x"
        );
        assert_eq!(
            normalize_url("https://news.ycombinator.com/item?id=123").unwrap(),
            "https://news.ycombinator.com/item?id=123"
        );
    }

    #[test]
    fn rejects_non_http() {
        assert!(normalize_url("ftp://example.com/file").is_err());
        assert!(normalize_url("not a url").is_err());
    }

    #[test]
    fn title_suffix_removed_only_for_source() {
        assert_eq!(
            title_key("Apple releases M5 - The Verge", "The Verge"),
            "apple releases m5"
        );
        assert_eq!(
            title_key("Rust 2.0 | TechCrunch", "Hacker News"),
            "rust 2 0",
            "known publisher"
        );
        assert_eq!(
            title_key("Kafka internals - A Deep Dive", "Confluent Blog"),
            "kafka internals a deep dive"
        );
    }

    #[test]
    fn jaccard_edges() {
        let a = word_set("new local ai model");
        let b = word_set("new local ai model");
        let c = word_set("database release notes");
        assert_eq!(jaccard(&a, &b), 1.0);
        assert_eq!(jaccard(&a, &c), 0.0);
        assert_eq!(jaccard(&a, &HashSet::new()), 0.0);
    }

    #[test]
    fn detects_english() {
        assert!(looks_english("How we built a multi-agent system with MCP"));
        assert!(looks_english("Kafka 5.0 released"));
        assert!(
            looks_english("DuckDB 2.0: faster storage"),
            "short/no signal counts as English"
        );
        assert!(!looks_english(
            "Procesa 50 millones de filas sin OOM: Query Streaming por bloques."
        ));
        assert!(!looks_english(
            "Suscríbete a decenas de tópicos de Kafka con un único patrón regex."
        ));
        assert!(!looks_english(
            "Comment utiliser les agents IA pour le développement et la production"
        ));
        assert!(!looks_english("大規模言語モデルをローカルで動かす方法"));
        assert!(!looks_english("Как запустить LLM локально на Mac"));
    }

    #[test]
    fn strips_html() {
        assert_eq!(
            strip_html("<p>Hello&nbsp;<b>world</b> &amp; you&#39;re</p>"),
            "Hello world & you're"
        );
        assert_eq!(truncate_chars("abcdef", 3), "abc…");
        assert_eq!(truncate_chars("abc", 3), "abc");
    }
}
