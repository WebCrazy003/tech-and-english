//! Pulls complete sentences of the `reply` string out of the tutor's streamed JSON, so speech can
//! start before the whole answer is written (P5 dev spec §6.3).

use super::tutor::{self, TutorTurnOut};
use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Scanning,
    InString,
    After,
}

/// Abbreviations whose dot does not end a sentence.
const ABBREVIATIONS: &[&str] = &["e.g.", "i.e.", "etc.", "vs.", "mr.", "mrs.", "ms.", "dr."];

pub struct ReplyExtractor {
    raw: String,
    /// Byte offset in `raw` up to which the text has been handled.
    pos: usize,
    state: State,
    sentence: String,
    reply: String,
}

impl Default for ReplyExtractor {
    fn default() -> Self {
        Self::new()
    }
}

fn hex4(s: &str) -> Option<u16> {
    s.get(..4).and_then(|h| u16::from_str_radix(h, 16).ok())
}

enum Step {
    Char(char, usize),
    /// The string ended; bytes consumed.
    End(usize),
    /// Need more input.
    Wait,
}

/// Decode one unit of a JSON string body.
fn step(s: &str) -> Step {
    let mut chars = s.chars();
    let Some(c) = chars.next() else {
        return Step::Wait;
    };
    match c {
        '"' => Step::End(1),
        '\\' => {
            let Some(e) = chars.next() else {
                return Step::Wait;
            };
            let simple = |ch| Step::Char(ch, 2);
            match e {
                '"' => simple('"'),
                '\\' => simple('\\'),
                '/' => simple('/'),
                'n' => simple('\n'),
                't' => simple('\t'),
                'r' => simple('\r'),
                'b' | 'f' => simple(' '),
                'u' => {
                    let Some(hi) = hex4(&s[2..]) else {
                        return if s.len() < 6 {
                            Step::Wait
                        } else {
                            Step::Char('\u{FFFD}', 2)
                        };
                    };
                    if (0xD800..0xDC00).contains(&hi) {
                        // A surrogate pair: 😀
                        if s.len() < 12 {
                            return Step::Wait;
                        }
                        let lo = (s.get(6..8) == Some("\\u"))
                            .then(|| s.get(8..).and_then(hex4))
                            .flatten();
                        match lo {
                            Some(lo) if (0xDC00..0xE000).contains(&lo) => {
                                let cp = 0x10000 + ((hi as u32 - 0xD800) << 10) + (lo as u32 - 0xDC00);
                                Step::Char(char::from_u32(cp).unwrap_or('\u{FFFD}'), 12)
                            }
                            _ => Step::Char('\u{FFFD}', 6),
                        }
                    } else {
                        Step::Char(char::from_u32(hi as u32).unwrap_or('\u{FFFD}'), 6)
                    }
                }
                other => Step::Char(other, 1 + other.len_utf8()),
            }
        }
        _ => Step::Char(c, c.len_utf8()),
    }
}

impl ReplyExtractor {
    pub fn new() -> Self {
        Self {
            raw: String::new(),
            pos: 0,
            state: State::Scanning,
            sentence: String::new(),
            reply: String::new(),
        }
    }

    /// The reply text decoded so far.
    pub fn reply(&self) -> &str {
        &self.reply
    }

    fn ends_sentence(&self) -> bool {
        // A closing quote or bracket may follow the punctuation: Please say: "I did it."
        let t = self.sentence.trim_end().trim_end_matches(['"', '\'', ')', '”', '’']);
        if !t.ends_with(['.', '?', '!']) {
            return false;
        }
        let last = t.rsplit(' ').next().unwrap_or("").to_lowercase();
        !ABBREVIATIONS.contains(&last.as_str())
    }

    fn take_sentence(&mut self, out: &mut Vec<String>) {
        let s = self.sentence.trim();
        if !s.is_empty() {
            out.push(s.to_string());
        }
        self.sentence.clear();
    }

    /// Feed a chunk; returns the sentences completed by it.
    pub fn feed(&mut self, chunk: &str) -> Vec<String> {
        self.raw.push_str(chunk);
        let mut out = Vec::new();
        loop {
            match self.state {
                State::Scanning => {
                    let rest = &self.raw[self.pos..];
                    let Some(k) = rest.find("\"reply\"") else {
                        return out;
                    };
                    let after = &rest[k + 7..];
                    let t = after.trim_start();
                    let Some(t2) = t.strip_prefix(':') else {
                        if t.is_empty() {
                            return out;
                        }
                        // "reply" appeared somewhere else (a value); keep looking after it.
                        self.pos += k + 7;
                        continue;
                    };
                    let t3 = t2.trim_start();
                    let Some(_) = t3.strip_prefix('"') else {
                        return out;
                    };
                    let consumed = rest.len() - t3.len() + 1;
                    self.pos += consumed;
                    self.state = State::InString;
                }
                State::InString => match step(&self.raw[self.pos..]) {
                    Step::Wait => return out,
                    Step::End(n) => {
                        self.pos += n;
                        self.take_sentence(&mut out);
                        self.state = State::After;
                    }
                    Step::Char(c, n) => {
                        self.pos += n;
                        if c.is_whitespace() && self.ends_sentence() {
                            self.take_sentence(&mut out);
                        }
                        if !(c.is_whitespace() && self.sentence.is_empty()) {
                            self.sentence.push(if c.is_whitespace() { ' ' } else { c });
                        }
                        self.reply.push(c);
                    }
                },
                State::After => {
                    self.pos = self.raw.len();
                    return out;
                }
            }
        }
    }

    /// The stream ended: the last sentence (if the reply never closed) and the parsed turn.
    pub fn finish(&mut self) -> (Vec<String>, AppResult<TutorTurnOut>) {
        let mut out = Vec::new();
        if self.state == State::InString {
            self.take_sentence(&mut out);
        }
        (out, tutor::parse(&self.raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "{\n  \"reply\": \"Nice idea. Version 2.5 is faster, e.g. on an M1! Please say: \\\"I deployed it.\\\"\\nDo you use \\u00e9t\\u00e9 or \\uD83D\\uDE00 emojis? Yes\",\n  \"correction\": null,\n  \"unknown_terms\": [],\n  \"useful_phrases\": [\"Nice idea\"]\n}";

    fn expected() -> Vec<String> {
        vec![
            "Nice idea.".into(),
            "Version 2.5 is faster, e.g. on an M1!".into(),
            "Please say: \"I deployed it.\"".into(),
            "Do you use été or 😀 emojis?".into(),
            "Yes".into(),
        ]
    }

    fn run(chunks: &[&str]) -> (Vec<String>, TutorTurnOut) {
        let mut x = ReplyExtractor::new();
        let mut got = Vec::new();
        for c in chunks {
            got.extend(x.feed(c));
        }
        let (rest, parsed) = x.finish();
        got.extend(rest);
        (got, parsed.unwrap())
    }

    #[test]
    fn same_result_for_every_split() {
        let (whole, parsed) = run(&[OUT]);
        assert_eq!(whole, expected());
        assert_eq!(parsed.useful_phrases, vec!["Nice idea".to_string()]);
        let bounds: Vec<usize> = (0..=OUT.len()).filter(|i| OUT.is_char_boundary(*i)).collect();
        for &cut in &bounds {
            let (a, b) = OUT.split_at(cut);
            let (s, p) = run(&[a, b]);
            assert_eq!(s, expected(), "cut at {cut}");
            assert_eq!(p, parsed, "cut at {cut}");
        }
        // One char at a time.
        let singles: Vec<String> = OUT.chars().map(String::from).collect();
        let refs: Vec<&str> = singles.iter().map(String::as_str).collect();
        assert_eq!(run(&refs).0, expected());
    }

    #[test]
    fn sentences_arrive_before_the_json_ends() {
        let mut x = ReplyExtractor::new();
        assert!(x.feed("{\"reply\": \"First one. Sec").len() == 1);
        assert_eq!(x.reply(), "First one. Sec");
        assert_eq!(x.feed("ond one? ")[0], "Second one?");
    }

    #[test]
    fn truncated_output_still_speaks_what_it_has() {
        let mut x = ReplyExtractor::new();
        let got = x.feed("{\"reply\": \"Half a sentence");
        assert!(got.is_empty());
        let (rest, parsed) = x.finish();
        assert_eq!(rest, vec!["Half a sentence".to_string()]);
        assert!(parsed.is_err());
    }

    #[test]
    fn the_word_reply_in_other_places_is_skipped() {
        let (s, _) = run(&["{\"note\": \"reply\", \"reply\": \"Hi there.\", \"correction\": null}"]);
        assert_eq!(s, vec!["Hi there.".to_string()]);
    }
}
