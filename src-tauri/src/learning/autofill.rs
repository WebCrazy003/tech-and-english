//! Fill missing meanings in the background (P4 dev spec §3.3). The macOS dictionary is tried
//! first (offline, cheap); the AI only when the model is already loaded and idle.

use crate::ai::service::AiService;
use crate::db::repo::vocab::Fields;
use crate::error::AppResult;
use crate::learning::dictionary::{self, DictEntry};
use crate::learning::vocab::VocabService;

pub const BATCH: u32 = 10;

/// Only these kinds have a dictionary-style meaning (sentences and corrections come from P5).
fn lookup_kind(kind: &str) -> bool {
    matches!(kind, "word" | "phrase" | "term" | "pronunciation")
}

pub fn fields_from_dictionary(e: &DictEntry) -> Option<Fields> {
    if !e.parsed {
        return None;
    }
    Some(Fields {
        meaning_simple: e.senses.first().cloned(),
        part_of_speech: e.part_of_speech.clone(),
        ipa: e.pronunciation.clone(),
        syllables: e.syllables.clone(),
        examples: e.examples.clone(),
        ..Default::default()
    })
}

/// Returns how many items got a meaning.
pub async fn run(vocab: &VocabService, ai: Option<&AiService>) -> AppResult<usize> {
    if !vocab.can_write() {
        return Ok(0);
    }
    let mut filled = 0;
    let mut ai_available = ai.is_some();
    for (item, ctx) in vocab.pending(BATCH).await? {
        if !lookup_kind(&item.kind) {
            continue;
        }
        let term = item.text.clone();
        let dict = tokio::task::spawn_blocking(move || dictionary::lookup(&term)).await?;
        let fields = match dict.as_ref().and_then(fields_from_dictionary) {
            Some(f) => Some(f),
            None if ai_available => {
                let sentence = ctx.as_ref().and_then(|c| c.sentence.clone()).unwrap_or_default();
                let title = ctx.as_ref().and_then(|c| c.article_title.clone()).unwrap_or_default();
                match ai.unwrap().define_term_background(&item.text, &sentence, &title).await {
                    Ok(Some(o)) => Some(Fields {
                        meaning_simple: Some(o.meaning_simple),
                        meaning_b1: Some(o.meaning_b1).filter(|m| !m.is_empty()),
                        part_of_speech: Some(o.part_of_speech).filter(|p| !p.is_empty()),
                        ipa: o.ipa,
                        syllables: o.syllables,
                        examples: o.examples,
                        collocations: o.collocations,
                        notes: None,
                    }),
                    Ok(None) => {
                        // The model is not loaded or busy: try again at a later tick.
                        ai_available = false;
                        None
                    }
                    Err(e) => {
                        tracing::debug!(item = item.id, error = %e, "AI meaning failed");
                        ai_available = false;
                        None
                    }
                }
            }
            None => None,
        };
        if let Some(f) = fields
            && vocab.fill(item.id, f).await?
        {
            filled += 1;
        }
    }
    if filled > 0 {
        tracing::info!(filled, "missing meanings filled");
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::MockProvider;
    use crate::clock::FakeClock;
    use crate::db::Db;
    use crate::events::RecordingEventSink;
    use crate::learning::vocab::{NewContext, NewItem};
    use crate::mode::{Mode, ModeManager};
    use crate::settings::SettingsStore;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    fn item(text: &str, meaning: Option<&str>) -> NewItem {
        NewItem {
            kind: "word".into(),
            text: text.into(),
            meaning_simple: meaning.map(str::to_string),
            meaning_b1: None,
            part_of_speech: None,
            ipa: None,
            syllables: None,
            examples: vec![],
            collocations: vec![],
            notes: None,
            context: Some(NewContext {
                sentence: Some(format!("We talk about {text} here.")),
                article_id: None,
                conversation_id: None,
            }),
        }
    }

    const JSON: &str = r#"{"meaning_simple":"a made-up word for tests","meaning_b1":"b1","part_of_speech":"noun","examples":["a","b"],"collocations":[]}"#;

    #[tokio::test]
    async fn fills_only_empty_meanings_and_never_loads_the_model() {
        let db = Db::open_in_memory().unwrap();
        let events = Arc::new(RecordingEventSink::default());
        let clock = Arc::new(FakeClock::at("2026-09-30T09:00:00Z"));
        let settings = SettingsStore::load(db.clone()).await.unwrap();
        let mode = ModeManager::load(db.clone(), events.clone()).await.unwrap();
        let vocab = VocabService::new(
            db.clone(),
            clock.clone(),
            settings.clone(),
            events.clone(),
            mode.clone(),
        );
        let mock = Arc::new(MockProvider::with_answers(&[&[JSON]]));
        let ai = AiService::new(db.clone(), clock, settings, events, mock.clone());

        // A word the macOS dictionary knows, a made-up one, and one that already has a meaning.
        let known = vocab.add(item("latency", None)).await.unwrap().item.id;
        let made_up = vocab.add(item("zorblaxing", None)).await.unwrap().item.id;
        let done = vocab.add(item("throughput", Some("mine"))).await.unwrap().item.id;

        mock.ready.store(false, Ordering::SeqCst);
        run(&vocab, Some(&ai)).await.unwrap();
        assert_eq!(mock.calls(), 0, "the model is not loaded: no AI call");
        let m = |id| {
            let vocab = &vocab;
            async move { vocab.detail(id).await.unwrap().item }
        };
        assert!(
            m(known).await.meaning_simple.is_some(),
            "filled from the macOS dictionary"
        );
        assert!(m(known).await.ipa.is_some());
        assert!(m(made_up).await.meaning_simple.is_none());

        mock.ready.store(true, Ordering::SeqCst);
        assert_eq!(run(&vocab, Some(&ai)).await.unwrap(), 1);
        let filled = m(made_up).await;
        assert_eq!(filled.meaning_simple.as_deref(), Some("a made-up word for tests"));
        assert!(
            mock.requests.lock().unwrap()[0].messages[1]
                .content
                .contains("We talk about zorblaxing here.")
        );
        assert_eq!(
            m(done).await.meaning_simple.as_deref(),
            Some("mine"),
            "existing meanings are kept"
        );

        let late = vocab.add(item("blorptastic", None)).await.unwrap().item.id;
        mode.set(Mode::Hibernate).await.unwrap();
        assert_eq!(run(&vocab, Some(&ai)).await.unwrap(), 0, "nothing in Hibernate");
        assert!(m(late).await.meaning_simple.is_none());
    }
}
