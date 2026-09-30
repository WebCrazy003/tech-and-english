//! The `tutor_turn` task (P5 dev spec §6): output schema, parsing and the prompt.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ai::provider::ChatMsg;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CorrectionPolicy {
    Low,
    Medium,
    #[default]
    High,
}

impl CorrectionPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            CorrectionPolicy::Low => "low",
            CorrectionPolicy::Medium => "medium",
            CorrectionPolicy::High => "high",
        }
    }

    fn rules(self) -> &'static str {
        match self {
            CorrectionPolicy::Low => "- Only correct mistakes that change the meaning or make it hard to understand.",
            CorrectionPolicy::Medium => {
                "- Correct important grammar mistakes (verb tense, subject–verb agreement, word order, wrong word) \
                 and any mistake the learner has made before in this session."
            }
            CorrectionPolicy::High => "- Correct any clear grammar or word-choice mistake.",
        }
    }
}

/// The model writes snake_case JSON; the UI and the stored turn meta use camelCase.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct Correction {
    pub original: String,
    pub corrected: String,
    #[serde(default)]
    pub explanation: String,
    #[serde(default)]
    pub ask_repeat: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Term {
    pub text: String,
    #[serde(default)]
    pub explanation: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct TutorTurnOut {
    pub reply: String,
    #[serde(default)]
    pub correction: Option<Correction>,
    #[serde(default)]
    pub unknown_terms: Vec<Term>,
    #[serde(default)]
    pub useful_phrases: Vec<String>,
    #[serde(default)]
    pub grammar_note: Option<String>,
}

/// `reply` comes first so it can be spoken while the rest is still being written.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["reply", "correction", "unknown_terms", "useful_phrases"],
        "properties": {
            "reply": { "type": "string", "maxLength": 700 },
            "correction": { "anyOf": [ { "type": "null" }, {
                "type": "object",
                "required": ["original", "corrected", "explanation", "ask_repeat"],
                "properties": {
                    "original": { "type": "string", "maxLength": 300 },
                    "corrected": { "type": "string", "maxLength": 300 },
                    "explanation": { "type": "string", "maxLength": 200 },
                    "ask_repeat": { "type": "boolean" } } } ] },
            "unknown_terms": { "type": "array", "maxItems": 3, "items": { "type": "object",
                "required": ["text", "explanation"],
                "properties": {
                    "text": { "type": "string", "maxLength": 60 },
                    "explanation": { "type": "string", "maxLength": 200 } } } },
            "useful_phrases": { "type": "array", "maxItems": 2, "items": { "type": "string", "maxLength": 80 } },
            "grammar_note": { "anyOf": [ { "type": "null" }, { "type": "string", "maxLength": 200 } ] }
        }
    })
}

fn trim_to(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>().trim_end().to_string()
    }
}

/// Read the model's JSON (tolerating text around it) and enforce the limits.
pub fn parse(text: &str) -> AppResult<TutorTurnOut> {
    let bad = || AppError::ai("ai_error", "The tutor's answer could not be read.");
    let (start, end) = (text.find('{').ok_or_else(bad)?, text.rfind('}').ok_or_else(bad)?);
    let mut o: TutorTurnOut = serde_json::from_str(text.get(start..=end).ok_or_else(bad)?).map_err(|_| bad())?;
    o.reply = trim_to(&o.reply, 700);
    if o.reply.is_empty() {
        return Err(bad());
    }
    o.correction = o.correction.and_then(|mut c| {
        c.original = trim_to(&c.original, 300);
        c.corrected = trim_to(&c.corrected, 300);
        c.explanation = trim_to(&c.explanation, 200);
        // A "correction" that changes nothing is noise.
        let same = super::similarity::normalize(&c.original) == super::similarity::normalize(&c.corrected);
        (!c.corrected.is_empty() && !same).then_some(c)
    });
    o.unknown_terms = o
        .unknown_terms
        .into_iter()
        .map(|t| Term {
            text: trim_to(&t.text, 60),
            explanation: trim_to(&t.explanation, 200),
        })
        .filter(|t| !t.text.is_empty())
        .take(3)
        .collect();
    o.useful_phrases = o
        .useful_phrases
        .iter()
        .map(|p| trim_to(p, 80))
        .filter(|p| !p.is_empty())
        .take(2)
        .collect();
    o.grammar_note = o.grammar_note.map(|g| trim_to(&g, 200)).filter(|g| !g.is_empty());
    Ok(o)
}

// ---------------------------------------------------------------- prompt

pub fn level_name(level: u8) -> &'static str {
    match level {
        1 => "Level 1, very easy (A2)",
        3 => "Level 3, natural (B2)",
        _ => "Level 2, intermediate (B1)",
    }
}

/// Spoken style per level (SPEC §12.3).
fn level_rules(level: u8) -> &'static str {
    match level {
        1 => {
            "- Use very short sentences (at most 10 words) and only the most common words.\n\
             - Explain every technical term in simple words.\n\
             - Often check that the learner understands."
        }
        3 => "- Speak naturally, like a friendly colleague. Explain only rare technical terms.",
        _ => {
            "- Use natural but simple sentences (CEFR B1).\n\
             - Explain each technical term simply the first time you use it."
        }
    }
}

pub fn max_sentences(level: u8) -> u8 {
    match level {
        1 => 3,
        3 => 5,
        _ => 4,
    }
}

/// What the conversation is about.
pub enum Topic<'a> {
    Article { title: &'a str, summary: &'a str },
    Free { topics: &'a [String] },
}

/// The system prompt. It stays byte-identical for the whole session so llama-server can reuse
/// its cache; everything that changes goes into the user message.
pub fn system_prompt(level: u8, policy: CorrectionPolicy, topic: &Topic) -> String {
    let about = match topic {
        Topic::Article { title, summary } => format!("Article: {title}\nSummary:\n\"\"\"\n{summary}\n\"\"\""),
        Topic::Free { topics } => format!(
            "No article. Talk about technology topics the learner likes: {}.",
            if topics.is_empty() {
                "software, AI and data".to_string()
            } else {
                topics.join(", ")
            }
        ),
    };
    format!(
        "You are a friendly English speaking tutor. The learner works in technology. Their level: {level_name}.\n\
         Your priorities, in order:\n\
         1. Help the learner improve their spoken English.\n\
         2. Have an interesting conversation about the article below.\n\
         \n\
         Speaking rules:\n\
         {level_rules}\n\
         - Your reply will be spoken aloud. No lists, no markdown, no emojis. At most {max} sentences.\n\
         - End with ONE question that invites the learner to speak — unless you are asking them to repeat a sentence.\n\
         - If the learner asks about a word or says they don't understand, explain it simply with one example,\n  \
         then return to the question you asked before.\n\
         - If the learner asks you to explain simply, use shorter sentences and more common words.\n\
         \n\
         Correction rules ({policy}):\n\
         {correction_rules}\n\
         - The learner's words come from speech recognition. Ignore punctuation, capitalisation, filler words\n  \
         (um, uh), repeated words and false starts. Never correct those.\n\
         - Correct at most ONE mistake per turn — the most important one.\n\
         - When you correct: start the reply with the natural sentence, give a very short reason, then end the\n  \
         reply with \"Please say: <corrected sentence>\". Nothing comes after it: no question. Set ask_repeat = true.\n\
         \n\
         Fields:\n\
         - correction: null, or original = the learner's whole sentence, corrected = the whole corrected sentence.\n\
         - unknown_terms: words the learner asked about or clearly did not understand (with your simple explanation).\n\
         - useful_phrases: up to 2 natural phrases from YOUR reply that are worth learning.\n\
         \n\
         {about}",
        level_name = level_name(level),
        level_rules = level_rules(level),
        max = max_sentences(level),
        policy = policy.as_str().to_uppercase(),
        correction_rules = policy.rules(),
    )
}

pub const OPENING_INSTRUCTION: &str =
    "Start the conversation: introduce the article in 2–3 sentences, then ask one easy question.";
pub const OPENING_INSTRUCTION_FREE: &str =
    "Start the conversation: greet the learner in one sentence, then ask one easy question about one of their topics.";
pub const REPEATED_INSTRUCTION: &str = "The learner repeated correctly. Continue the conversation.";
pub const TRIED_INSTRUCTION: &str = "The learner tried to repeat the sentence. Continue the conversation.";

/// The user message for one turn. `mistakes` lives here (not in the system prompt) so the
/// cached prefix stays valid when a new mistake is noted.
pub fn user_message(instruction: &str, transcript: &str, mistakes: &[String]) -> ChatMsg {
    let mut s = String::new();
    if !mistakes.is_empty() {
        s.push_str(&format!(
            "(Mistakes already noted in this session: {}.)\n",
            mistakes.join("; ")
        ));
    }
    if !instruction.is_empty() {
        s.push_str(instruction);
        s.push('\n');
    }
    s.push_str(&format!("Learner said: \"{transcript}\""));
    ChatMsg::user(s)
}

/// A short label for `session_mistakes`, e.g. "deploy → deployed".
pub fn mistake_label(c: &Correction) -> String {
    let (a, b) = super::similarity::changed_words(&c.original, &c.corrected);
    let label = match (a.is_empty(), b.is_empty()) {
        (true, true) => c.corrected.clone(),
        _ => format!("{} → {}", a.join(" "), b.join(" ")),
    };
    trim_to(&label, 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_limits() {
        let raw = r#"Sure! {"reply": "Nice. Please say: Yesterday I deployed the app.",
            "correction": {"original": "Yesterday I deploy the app.", "corrected": "Yesterday I deployed the app.",
                           "explanation": "Past tense.", "ask_repeat": true},
            "unknown_terms": [{"text": "deploy", "explanation": "put software live"}, {"text": " ", "explanation": ""}],
            "useful_phrases": ["It depends on", "a", "b"], "grammar_note": ""}"#;
        let o = parse(raw).unwrap();
        assert!(o.reply.starts_with("Nice."));
        assert!(o.correction.as_ref().unwrap().ask_repeat);
        assert_eq!(o.unknown_terms.len(), 1);
        assert_eq!(o.useful_phrases.len(), 2);
        assert_eq!(o.grammar_note, None);
        assert!(parse(r#"{"reply": ""}"#).is_err());
        assert!(parse("no json").is_err());
    }

    #[test]
    fn a_correction_that_changes_nothing_is_dropped() {
        let o = parse(
            r#"{"reply":"Good.","correction":{"original":"I think it's fast","corrected":"I think it is fast.",
                "explanation":"","ask_repeat":true},"unknown_terms":[],"useful_phrases":[]}"#,
        )
        .unwrap();
        assert_eq!(o.correction, None);
    }

    #[test]
    fn system_prompt_is_stable_and_complete() {
        let t = Topic::Article {
            title: "Local AI",
            summary: "Models run on laptops.",
        };
        let a = system_prompt(2, CorrectionPolicy::High, &t);
        assert_eq!(a, system_prompt(2, CorrectionPolicy::High, &t));
        assert!(a.contains("Level 2") && a.contains("At most 4 sentences") && a.contains("Models run on laptops."));
        assert!(a.contains("Correction rules (HIGH)") && a.contains("any clear grammar"));
        assert!(!a.contains("{{"), "no unfilled placeholder");
        let f = system_prompt(
            1,
            CorrectionPolicy::Low,
            &Topic::Free {
                topics: &["Rust".into()],
            },
        );
        assert!(f.contains("likes: Rust.") && f.contains("At most 3 sentences") && f.contains("change the meaning"));
    }

    #[test]
    fn user_message_carries_mistakes_and_instruction() {
        let m = user_message(REPEATED_INSTRUCTION, "ok", &["deploy → deployed".into()]);
        assert_eq!(m.role, "user");
        assert!(
            m.content
                .starts_with("(Mistakes already noted in this session: deploy → deployed.)")
        );
        assert!(m.content.ends_with("Learner said: \"ok\""));
        assert_eq!(user_message("", "hi", &[]).content, "Learner said: \"hi\"");
    }

    #[test]
    fn mistake_labels_show_the_changed_words() {
        let c = Correction {
            original: "Yesterday I deploy the application.".into(),
            corrected: "Yesterday I deployed the application.".into(),
            explanation: String::new(),
            ask_repeat: true,
        };
        assert_eq!(mistake_label(&c), "deploy → deployed");
    }

    #[test]
    fn schema_keeps_reply_first() {
        let s = schema();
        let keys: Vec<&String> = s["properties"].as_object().unwrap().keys().collect();
        assert_eq!(keys[0], "reply");
    }
}
