//! Prompt templates (P2 dev spec §11). Bump `PROMPT_VERSION` when a template changes:
//! cached outputs with an older version are regenerated.

use std::collections::HashSet;

use super::provider::ChatMsg;

pub const PROMPT_VERSION: &str = "1";
// "Explain simply" answers are only cached in memory, so changing that prompt needs no version bump.

/// Article text per request, measured in chars (≈ 1,800 tokens: about 9 s of prompt reading on an M1).
pub const ARTICLE_BUDGET_CHARS: usize = 7_200;
/// Relevant paragraphs sent with each chat question.
pub const CHAT_CONTEXT_CHARS: usize = 3_000;
/// Chat messages sent as history.
pub const CHAT_HISTORY: u32 = 8;

pub fn level_rules(level: u8) -> &'static str {
    match level {
        1 => {
            "Use very short sentences (max 10 words). Use only very common words. Explain every technical term in simple words."
        }
        3 => "Use natural English. Explain only rare technical terms.",
        _ => {
            "Use clear, simple sentences (CEFR B1). Keep important technical terms, but explain each one simply the first time you use it."
        }
    }
}

pub fn paragraphs(body: &str) -> Vec<&str> {
    body.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).collect()
}

const STOP: &[&str] = &[
    "the", "a", "an", "of", "to", "in", "for", "on", "and", "or", "with", "is", "are", "was", "were", "it", "this",
    "that", "what", "why", "how", "does", "do", "can", "you", "me", "i", "about", "be", "as", "at", "by", "from",
    "tell", "explain",
];

fn content_words(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !STOP.contains(w))
        .map(str::to_string)
        .collect()
}

/// Paragraphs in original order: the first ones fill 60 % of the budget; the rest is filled with
/// later paragraphs that contain `keywords` (P2 dev spec §11.5).
pub fn article_excerpt(body: &str, keywords: &[String], budget: usize) -> String {
    let paras = paragraphs(body);
    if body.len() <= budget {
        return paras.join("\n\n");
    }
    let mut chosen = vec![false; paras.len()];
    let mut used = 0;
    for (i, p) in paras.iter().enumerate() {
        if used + p.len() > budget * 6 / 10 {
            break;
        }
        chosen[i] = true;
        used += p.len() + 2;
    }
    let kws: Vec<String> = keywords.iter().map(|k| k.to_lowercase()).collect();
    for (i, p) in paras.iter().enumerate() {
        if chosen[i] || used + p.len() > budget {
            continue;
        }
        let lower = p.to_lowercase();
        if kws.iter().any(|k| lower.contains(k.as_str())) {
            chosen[i] = true;
            used += p.len() + 2;
        }
    }
    let mut out: Vec<&str> = Vec::new();
    for (i, p) in paras.iter().enumerate() {
        if chosen[i] {
            out.push(p);
        }
    }
    if out.is_empty() {
        // One huge paragraph: cut it.
        return body.chars().take(budget).collect();
    }
    out.join("\n\n")
}

/// First paragraph + paragraphs sharing the most words with `question`, in original order.
pub fn relevant_paragraphs(body: &str, question: &str, budget: usize) -> String {
    let paras = paragraphs(body);
    if paras.is_empty() {
        return String::new();
    }
    let q = content_words(question);
    let mut scored: Vec<(usize, usize)> = paras
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, p)| (i, content_words(p).intersection(&q).count()))
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut chosen = vec![0usize];
    let mut used = paras[0].len();
    for (i, score) in scored {
        if q.is_empty() || score == 0 {
            // No question words: continue with the next paragraphs in order.
            if used + paras[i].len() > budget {
                continue;
            }
        } else if used + paras[i].len() > budget {
            continue;
        }
        chosen.push(i);
        used += paras[i].len() + 2;
        if used >= budget {
            break;
        }
    }
    chosen.sort_unstable();
    chosen.into_iter().map(|i| paras[i]).collect::<Vec<_>>().join("\n\n")
}

pub struct ArticleText<'a> {
    pub title: &'a str,
    pub source: &'a str,
    /// Body if extracted, else the feed description.
    pub text: &'a str,
    pub from_description_only: bool,
}

pub fn summarize_b1(level: u8, a: &ArticleText, keywords: &[String]) -> Vec<ChatMsg> {
    let body = article_excerpt(a.text, keywords, ARTICLE_BUDGET_CHARS);
    vec![
        ChatMsg::system(format!(
            "You help a technology professional learn English. Their English level is B1.\n{}\n\
             Only use information from the article. Do not add facts. Do not use hype words.",
            level_rules(level)
        )),
        ChatMsg::user(format!(
            "Article title: {}\nSource: {}\n\nArticle:\n\"\"\"\n{}\n\"\"\"\n\n\
             Write a summary of at most 250 words:\n\
             1. One sentence: what is this article about?\n\
             2. Two or three short paragraphs with the main points.\n\
             3. A section \"**Key words**\" with 3–5 technical words from the article, one per line, as: word — simple meaning",
            a.title, a.source, body
        )),
    ]
}

pub fn simplify_easy(level: u8, a: &ArticleText, keywords: &[String]) -> Vec<ChatMsg> {
    let body = article_excerpt(a.text, keywords, ARTICLE_BUDGET_CHARS);
    vec![
        ChatMsg::system(format!(
            "You rewrite technology articles in Easy English for a learner.\n{}\n\
             Rules: sentences of at most 12 words. One idea per sentence. Use the most common English words. \
             Keep technical terms, and explain each one in brackets the first time. Keep the article's order. \
             At most 400 words. No bullet lists. Only use information from the article.",
            level_rules(level)
        )),
        ChatMsg::user(format!(
            "Article title: {}\n\nArticle:\n\"\"\"\n{}\n\"\"\"\n\nRewrite it in Easy English.",
            a.title, body
        )),
    ]
}

pub fn why_interesting(title: &str, description: &str, topics: &[String], hn_points: Option<i64>) -> Vec<ChatMsg> {
    let pop = hn_points
        .map(|p| format!("\nHacker News points: {p}"))
        .unwrap_or_default();
    vec![
        ChatMsg::system(
            "You recommend one tech story to a reader. Write 1–2 short sentences (max 40 words) that say why \
             this story may interest them. Address them as \"you\". Mention their topics. No hype, no emojis.",
        ),
        ChatMsg::user(format!(
            "Story: {title}\nDescription: {description}\nTheir topics: {}{pop}",
            topics.join(", ")
        )),
    ]
}

pub fn article_chat_system(level: u8, a: &ArticleText, summary: Option<&str>, relevant: &str) -> String {
    let summary = summary
        .map(|s| format!("Summary:\n\"\"\"\n{s}\n\"\"\"\n"))
        .unwrap_or_default();
    format!(
        "You help a technology professional understand an article and learn English at the same time.\n{}\n\
         Answer from the article below. If the article does not answer the question, say that first; \
         then you may add a short general answer and end it with \"(not from the article)\". \
         Never add that note when your answer comes from the article.\n\
         Keep answers under 120 words unless the user asks for more. Use short paragraphs. No tables.\n\n\
         Article: {} ({})\n{}Relevant parts:\n\"\"\"\n{}\n\"\"\"",
        level_rules(level),
        a.title,
        a.source,
        summary,
        relevant
    )
}

/// "Explain simply" in the word popup (P4 dev spec §3.2). The answer is JSON (see `define_term_schema`).
pub fn define_term(level: u8, term: &str, sentence: &str, title: &str) -> Vec<ChatMsg> {
    let sentence = if sentence.trim().is_empty() {
        "(none)"
    } else {
        sentence.trim()
    };
    let title = if title.trim().is_empty() {
        "(none)"
    } else {
        title.trim()
    };
    vec![
        ChatMsg::system(format!(
            "You are an English dictionary for a B1 learner who works in technology.\n{}\n\
             Explain the meaning the term has in the given sentence. Give 2 or 3 short example sentences; \
             at least one must be about the article's topic. Collocations are common word partners \
             of this term (for the word \"decision\" they would be \"make a decision\", \"final decision\"). Syllables: dots between syllables, the stressed syllable in CAPITALS \
             (e.g. sca·la·BIL·i·ty). Return JSON only.",
            level_rules(level)
        )),
        ChatMsg::user(format!(
            "Term: \"{term}\"\nSentence where it appeared: \"{sentence}\"\nArticle title: {title}\n\n\
             Explain the meaning of the term as it is used in this sentence."
        )),
    ]
}

pub fn define_term_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["meaning_simple", "meaning_b1", "part_of_speech", "examples", "collocations"],
        "properties": {
            "meaning_simple": { "type": "string", "maxLength": 160 },
            "meaning_b1": { "type": "string", "maxLength": 240 },
            "part_of_speech": { "type": "string",
                "enum": ["noun", "verb", "adjective", "adverb", "phrase", "phrasal verb", "idiom", "other"] },
            "ipa": { "type": "string", "maxLength": 60 },
            "syllables": { "type": "string", "maxLength": 60 },
            "examples": { "type": "array", "minItems": 2, "maxItems": 3, "items": { "type": "string", "maxLength": 160 } },
            "collocations": { "type": "array", "maxItems": 4, "items": { "type": "string", "maxLength": 40 } }
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickAction {
    Summarize,
    KeyWords,
    ExplainSimply,
}

impl QuickAction {
    /// Sent as the user's message, so it shows in the chat history.
    pub fn message(self) -> &'static str {
        match self {
            QuickAction::Summarize => "Summarize this article.",
            QuickAction::KeyWords => {
                "List 5 important technical words from this article. For each: word — simple meaning."
            }
            QuickAction::ExplainSimply => "Explain the main idea of this article in very simple English.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> String {
        (0..20)
            .map(|i| {
                if i == 15 {
                    "Kafka topics store events for streaming consumers. ".repeat(8)
                } else {
                    format!("Paragraph {i} talks about general things in data teams. ").repeat(8)
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    #[test]
    fn excerpt_keeps_start_and_keyword_paragraphs_within_budget() {
        let b = body();
        let e = article_excerpt(&b, &["Kafka".to_string()], 2_000);
        assert!(e.len() <= 2_000 + 50);
        assert!(e.starts_with("Paragraph 0"));
        assert!(e.contains("Kafka topics"), "keyword paragraph pulled in");
        assert!(!e.contains("Paragraph 19"));
        let short = "One.\n\nTwo.";
        assert_eq!(article_excerpt(short, &[], 2_000), "One.\n\nTwo.");
    }

    #[test]
    fn relevant_paragraphs_follow_the_question() {
        let b = body();
        let r = relevant_paragraphs(&b, "What do Kafka topics store?", 1_500);
        assert!(r.starts_with("Paragraph 0"), "first paragraph always included");
        assert!(r.contains("Kafka topics"));
        assert!(r.len() <= 1_500 + 450);
    }

    #[test]
    fn templates_render() {
        let a = ArticleText {
            title: "T",
            source: "S",
            text: "Body.",
            from_description_only: false,
        };
        let m = summarize_b1(2, &a, &[]);
        assert_eq!(m.len(), 2);
        assert!(m[0].content.contains("CEFR B1"));
        assert!(m[1].content.contains("**Key words**"));
        assert!(simplify_easy(1, &a, &[])[0].content.contains("max 10 words"));
        assert!(
            why_interesting("T", "D", &["AI".into()], Some(10))[1]
                .content
                .contains("Hacker News points: 10")
        );
        let sys = article_chat_system(2, &a, Some("Sum"), "Rel");
        assert!(sys.contains("(not from the article)") && sys.contains("Sum") && sys.contains("Rel"));
        assert!(QuickAction::KeyWords.message().contains("5 important"));
        let d = define_term(2, "inference", "The model runs inference.", "Local AI");
        assert!(d[0].content.contains("Return JSON only") && d[1].content.contains("\"inference\""));
        assert_eq!(define_term_schema()["required"][0], "meaning_simple");
    }
}
