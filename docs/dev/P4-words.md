# P4 — Words: popup, Word Book & Practice (dev spec)

- **Goal:** select a word in a story (or an AI answer) to see its meaning and pronunciation, hear it, and save it to the Word Book. Then practise saved words with a 10-item quiz scheduled by FSRS.
- **Meanings:**
  - the **macOS built-in dictionary**: instant and offline
  - **Explain simply (AI)**: uses the local model from P2
- **SPEC sections:** §8.5, §9, §10, §13 (P4), §14 (P4), §18 P4.
- **Branch:** `p4/words` → tag `v0.4.0`
- **Status:** ✅ done 2026-09-30. QA: [../qa/P4.md](../qa/P4.md). Changes found while building are marked **(changed)**.
- **Built from:** the Word Book / quiz half of the old P2 spec, plus `define_term` and meaning auto-fill from the old P3 spec (SPEC C16, C17).

## 0. Scope

**In scope:**
- Dictionary lookup through macOS Dictionary Services (Rust FFI) and parsing its output
- Selection popup in the Reader and in AI chat answers
- TTS (🔊, Listen); Settings › Voice
- `define_term` (AI) and background auto-fill of missing meanings
- Word Book (add/merge/edit/delete/list/detail/CSV export); highlights for known words
- FSRS scheduling; quiz selection, sessions and scoring; Practice UI
- Due count in the widget; Hibernate gating

**Out of scope:** voice input (P5).

---

## 1. Dependencies

| Where | Add |
|---|---|
| Cargo | `fsrs = "6.6"`, `fastrand = "2"`, `core-foundation = "0.10"` (CFString for Dictionary Services) |
| System | `CoreServices.framework` (linked; ships with macOS) |

> **fsrs crate risk.** At T5 start, check the clean build time and binary size. If the crate adds more than 60 s or 5 MB, port **only** the FSRS-6 `next_states` scheduling formulas with default parameters into `learning/srs.rs`, and test them against values precomputed with the crate.
>
> **Result (2026-09-30):** fsrs 6.6.2 no longer depends on `burn` (only ndarray, rayon, rand…). Building it and its new dependencies took 40 s; the release binary grew by 0.4 MB for all of P4. The crate is kept.

---

## 2. Database — `migrations/0004_vocab.sql`

- Create `vocab_items`, `vocab_contexts`, `quiz_sessions` and `vocab_reviews` exactly as in SPEC §13, **with one change:**
  - `vocab_contexts.conversation_id INTEGER` is created **without** `REFERENCES conversations(id)`, because that table only arrives in P5.
  - P5's migration will not add the foreign key. The app deletes contexts explicitly when a conversation is deleted.
- Add `CREATE INDEX idx_vocab_contexts_item ON vocab_contexts(item_id);` and `CREATE INDEX idx_vocab_reviews_item ON vocab_reviews(item_id, reviewed_at);`.
- **Retention extension:** articles referenced by `vocab_contexts.article_id` are never deleted.

---

---

## 3. Meanings

### 3.1 macOS dictionary (`learning/dictionary.rs`)

```rust
#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn DCSCopyTextDefinition(dictionary: *const c_void, text: CFStringRef, range: CFRange) -> CFStringRef; // NULL if none
}
pub struct DictEntry { pub headword: String, pub pronunciation: Option<String>, pub part_of_speech: Option<String>,
                       pub senses: Vec<String>, pub raw: String }
pub fn lookup(term: &str) -> Option<DictEntry>          // runs on spawn_blocking
pub fn parse(raw: &str) -> DictEntry                    // pure, unit-tested
```

- Dictionary Services uses the user's default dictionary (on this Mac: New Oxford American Dictionary). It finds inflected forms too ("inferences" → inference).
- **The real format (changed)** on macOS 14.7 with the New Oxford American Dictionary is
  `headword syl·la·bles | pronunciation | [labels] part-of-speech [1] sense: example | example. • sub-sense … 2 …`.
  So `DictEntry` also has `syllables`, `examples` (from the text after `:`) and `parsed` (false = unusual format, `senses[0]` holds the raw start). Subject labels become a prefix: "(Computing) the delay before…". A second part of speech ("… noun an idempotent element") is not mixed into the first.
- **Phrases (changed):** Dictionary Services returns the first word's entry for a phrase ("on the fly" → "fly"). `entry_for(term, raw)` accepts the entry only when the headword matches (ignoring spaces and hyphens: "trade off" = "trade-off"); otherwise it looks for the phrase in the PHRASES section (with its own pronunciation) and returns part of speech `phrase`, or nothing.
- **Parsing** the plain-text format (as first planned) `headword | pronunciation | part of speech 1 sense … 2 sense …`:
  1. Split the first two ` | ` separators to get the headword and the pronunciation.
  2. The part of speech is the first word(s) of the rest (`noun`, `verb`, `adjective`, `adverb`, `phrasal verb`, …).
  3. The senses are split on ` N ` numbered markers. Keep at most 2, each cut to 200 chars at a sentence end.
  4. Drop the `PHRASES` / `DERIVATIVES` / `ORIGIN` sections.
  5. If the format is unusual, keep `raw`, cut to 300 chars, as the only sense.
- **Tests:** parse saved outputs in `tests/fixtures/dictionary/*.txt`: 8 real ones (inference, latency, run, trade-off, API, rule of thumb, pipeline, idempotent) and 3 made-up ones (no pronunciation, non-English, unknown format). Also an ignored live test that calls `lookup("inferences")` (238 ms cold, 2 ms warm).
- Command: `dictionary_lookup { term }` → `DictEntry | null`. It works in **both modes** (Hibernate too), because it is cheap and needs no AI.

### 3.2 `define_term.md` (JSON, AI)

```
### system
You are an English dictionary for a B1 learner who works in technology.
{{level_rules}}
Return JSON only.

### user
Term: "{{term}}"
Sentence where it appeared: "{{sentence}}"
Article title: {{title}}

Explain the meaning of the term as it is used in this sentence.
```

Schema, mirrored by the Rust struct `DefineTermOut`:

```json
{
  "type": "object",
  "required": ["meaning_simple", "meaning_b1", "part_of_speech", "examples", "collocations"],
  "properties": {
    "meaning_simple": { "type": "string", "maxLength": 160 },
    "meaning_b1":     { "type": "string", "maxLength": 240 },
    "part_of_speech": { "type": "string", "enum": ["noun","verb","adjective","adverb","phrase","phrasal verb","idiom","other"] },
    "ipa":            { "type": "string", "maxLength": 60 },
    "syllables":      { "type": "string", "maxLength": 60, "description": "dots between syllables, stressed syllable in CAPITALS, e.g. sca·la·BIL·i·ty" },
    "examples":       { "type": "array", "minItems": 2, "maxItems": 3, "items": { "type": "string", "maxLength": 160 } },
    "collocations":   { "type": "array", "maxItems": 4, "items": { "type": "string", "maxLength": 40 } }
  }
}
```

- At least one example must relate to the article's topic; the prompt says so.
- `ipa` and `syllables` are shown with the label **"hint"** (SPEC §12.6).
- **(changed)** The prompt's collocation example is about another word ("decision"): with "run inference" as the example, the model copied it into unrelated words. Live on Qwen3.5 4B: 11–15 s per term, valid JSON every time (`live_ai::define_term_on_real_model`). The model does not give IPA; the dictionary does.

- Used by **Explain simply** in the popup. Results are cached in memory for the session, keyed by `(term_key, sentence)`.

### 3.3 Auto-fill of missing meanings (background job)

Every scheduler tick (60 s), in Standard Mode, take up to 10 Word Book items with no meaning (oldest first; only word, phrase, term and pronunciation kinds):
- If the dictionary has an entry, fill `meaning_simple` from its first sense, `ipa`, part of speech, syllables and examples. **(changed:** this needs no model, so it runs even when the AI is not loaded.)
- Otherwise, when the model is `Ready` and nothing else is running, run `define_term` with the item's latest context.

Only empty fields are filled. Then emit `vocab://changed`. This never triggers a model load.

---

## 4. Selection popup (`src/features/words/WordPopup.tsx`)

It opens on `mouseup` / `keyup` inside the Reader body, the B1/Easy tab text, or an AI chat answer.

**Text:**
- `window.getSelection().toString().trim()`: 1–8 words, ≤ 80 chars. Longer selections keep the P2 "Ask AI about this" chip instead.
- **Context sentence:** the closest block's `textContent`, split with `Intl.Segmenter("en", { granularity: "sentence" })`. Take the sentence that contains the selection start.

**Popup content** (anchored to `range.getBoundingClientRect()`, stays on screen):

```
inference   /ˈinf(ə)rəns/   noun            🔊
1. a conclusion reached on the basis of evidence and reasoning
2. the process of reaching such a conclusion
[Explain simply (AI)]   [Ask AI]
Type: [ word ▾ ]  (1 word → word, 2+ → phrase; or "technical term")
Context: "…the model performs inference on your laptop…"
[Add to Word Book]   (already saved → "Add this context")
```

- The dictionary result shows within 300 ms. While it loads, show a skeleton.
- **Explain simply (AI):** `define_term` → shows the simple meaning, B1 meaning, examples, collocations and syllables (labelled "hint"). If the AI is not available (no model, or Hibernate), the button is disabled and its tooltip explains why.
- **Ask AI:** posts `What does "<term>" mean in this story?` to the chat panel.
- **Add to Word Book** saves:
  - `meaning_simple`: the AI simple meaning if requested, else dictionary sense 1
  - `meaning_b1`: from the AI
  - `ipa`: dictionary pronunciation or the AI's
  - `part_of_speech`
  - `examples` and `collocations`: from the AI, if any
  - the context: sentence + article
- Esc or clicking outside closes the popup. In Hibernate, **Add** is disabled with the tooltip "Switch to Standard to add words" (SPEC §6 keeps the Word Book read-only in Hibernate).

### 4.1 Highlights for known words

- After the body renders, use the **CSS Custom Highlight API** (`CSS.highlights.set("known-vocab", new Highlight(...ranges))`) to underline words and phrases whose `text_key` matches the Word Book. This does not change the DOM.
- Style: `::highlight(known-vocab) { text-decoration: underline dotted 1.5px; }`
- If `CSS.highlights` is missing, skip highlighting.
- Matching:
  1. Tokenize the text nodes.
  2. Compare lowercased tokens and 2–4-token n-grams against a `Set` loaded by `list_vocab_keys()`.
  3. Stop after 2,000 ranges.

---

## 5. TTS (`src/lib/tts.ts`)

```ts
export interface TtsOptions { rate: number; voiceURI?: string; volume?: number }
export function listEnglishVoices(): Promise<SpeechSynthesisVoice[]>  // waits for voiceschanged (max 2 s)
export function speak(text: string, opts: TtsOptions): Promise<void>  // resolves on end, rejects on error
export function speakSentences(sentences: string[], opts: TtsOptions & { pauseMs: number }): Promise<void>
export function stop(): void
```

- **Default voice:** the first `lang` starting with `en-` whose name contains `Premium`, then `Enhanced`, then any `en-US`, then any `en-GB`, then any `en-*`.
- **Settings › Voice page:** voice picker (grouped by accent: US / UK / AU / IE / IN / ZA), rate slider 0.5–1.2 (default 0.85), a test button, and a hint:
  > Better voices: System Settings › Accessibility › Spoken Content › System Voice › Manage Voices… (download an English "Premium" voice).
- **Listen:** speaks the cached B1 summary if there is one (P2), otherwise `title. description`, with `speakSentences`. The button toggles to **Stop** while speaking.
- In Hibernate, TTS is disabled (SPEC §6) and the buttons show the tooltip "Switch to Standard".
- `Settings` gains `tts: { voiceUri: Option<String>, rate: f32 (0.85), wordRate (0.7, changed: the popup and quizzes speak single words slower), volume: f32 (1.0), pauseMs: u32 (400) }` and `learning: { quizSize (10), desiredRetention (0.9), autoPronounce (true) }`.
- **(changed)** WKWebView (checked with a Swift probe) has `speechSynthesis` with 187 voices, `Intl.Segmenter` and the CSS Highlight API. Voice IDs look like `com.apple.voice.enhanced.en-US.Ava`, so "Premium"/"Enhanced" is searched in the ID too, and the robotic `eloquence` voices and the macOS novelty voices ("Bubbles", "Bad News", …) are not offered.
- **Listen (changed):** reads the open B1/Easy tab, else the cached B1 summary (new command `get_cached_derivative`, which never starts the AI), else title + description.
- **Read-along (added 2026-09-30):** while Listen reads, the sentence and the word being spoken are highlighted in the text (`lib/spokenHighlight.ts`: CSS Custom Highlight API + the `boundary` events of speechSynthesis; checked in WKWebView with Flo, Samantha and Daniel — one `word` event per word with `charIndex`/`charLength`). The text is read from the DOM, so what is heard is what is shown: on the Original tab a saved summary is opened in its tab first. Title + description (no summary yet) is read without highlighting. The tutor's bubbles in Talk use the same highlighting.

---

---

## 6. Word Book service (`learning/vocab.rs`)

```rust
pub fn text_key(s: &str) -> String          // trim, collapse whitespace, lowercase, strip surrounding quotes/punctuation
pub struct NewItem { kind: Kind, text: String, meaning_simple: Option<String>, notes: Option<String>,
                     context: Option<NewContext> }         // NewContext { sentence, article_id, conversation_id }
pub enum AddOutcome { Created(Item), MergedContext(Item) }
pub fn add(tx, clock, NewItem) -> Result<AddOutcome>
```

**Merge rules** (SPEC §9.3):
- An existing `(kind, text_key)` → insert a context unless an identical sentence already exists for that item.
- Fill `meaning_simple` if it was empty and a meaning was provided.
- `status = known` → `learning`.

Other rules:
- An empty `text`, or `text` longer than 200 chars → `Invalid`.
- `update` can change `text` only if the new key doesn't collide.
- `delete` cascades to contexts and reviews.

**Commands (P4):**

| Command | Args | Returns |
|---|---|---|
| `add_vocab_item` | `NewItem` (+ optional `pronunciation`, `partOfSpeech`, `meaningSource: "dictionary" \| "ai" \| "user"`) | `{ outcome: "created" \| "merged", item: VocabItem }` |
| `update_vocab_item` | `{ id, patch }` | `VocabItem` |
| `delete_vocab_item` | `{ id }` | `void` |
| `list_vocab` | `{ filter: { query?, kind?, status?, dueOnly?, articleId?, pendingOnly? }, cursor?, limit? }` | `Page<VocabListItem>` |
| `get_vocab_item` | `{ id }` | `VocabItemDetail` (item + contexts with article titles + last 20 reviews) |
| `list_vocab_keys` | – | `string[]` (for highlights) |
| `due_count` | – | `{ due, new, total, ready }` (changed: `total` hides the widget line for an empty Word Book; `ready` = items with a meaning, for "at least 3") |
| `export_vocab_csv` | – | `{ path }` (writes to `~/Downloads/tech-english-wordbook-YYYY-MM-DD.csv`, then reveals it in Finder with `opener`) |

- The CSV columns are: kind, text, meaning_simple, meaning_b1, part_of_speech, examples (joined with ` | `), status, review_count, due_at, first_context, source_article_url.
- **Hibernate:** every write command returns `AppError::Hibernating`. Reads are allowed.

---

---

## 7. SRS (`learning/srs.rs`)

```rust
pub enum Grade { Forgot, Unsure, Remember }            // → FSRS Again / Hard / Good
pub struct SrsState { stability: Option<f64>, difficulty: Option<f64>, last_reviewed_at: Option<DateTime<Utc>>, due_at: Option<DateTime<Utc>> }
pub fn review(state: &SrsState, grade: Grade, now: DateTime<Utc>) -> SrsUpdate   // new stability, difficulty, due_at, srs_state
```

- `desired_retention = 0.9` (a setting, range 0.8–0.95).
- `days_elapsed` = whole days since `last_reviewed_at` (0 for a new item).
- `due_at = now + interval_days`, where `interval_days` is at least 1 day. **Exception:** after Forgot, `due_at = now + 10 min`, so the item can come back later the same day in a new quiz.
- Status transitions follow SPEC §10.1 and are applied in `vocab.rs` after `review`.
- Counters (`review_count`, `*_count`), `last_grade` and `last_reviewed_at` are updated, and a `vocab_reviews` row is inserted. All of this happens in one transaction.
- Verify the exact `fsrs` API (`FSRS::new`, `next_states`, `MemoryState`) against the crate docs at the start of the task.

**Tests:**
- A new item graded Remember → due in ≥ 1 day, status `learning`.
- Remember repeated over simulated days → the interval grows and status reaches `known` at stability ≥ 21.
- Forgot on a `known` item → `learning`, due in 10 min.
- Grade → counter mapping.

---

---

## 8. Quiz (`learning/quiz.rs`)

### 8.1 Selection

This implements SPEC §10.2, with an injected `fastrand::Rng` so tests are deterministic.

```
pool = items with a meaning (meaning_simple or meaning_b1 not null)
b1 = last_grade = forgot and last_reviewed_at ≥ now − 3 d
b2 = last_grade = unsure (not already chosen)
b3 = due_at ≤ now, sorted by due_at asc
b4 = review_count = 0, sorted by created_at asc, max 3
b5 = the rest, random
for each bucket: take from a random sample of the top (2 × remaining) until size is reached
shuffle the result
if pool.len() < 3 → Err(Invalid("need at least 3 words with a meaning"))
```

### 8.2 Commands

| Command | Args | Returns |
|---|---|---|
| `start_quiz` | `{ size?: number }` (default setting 10, range 5–30) | `{ sessionId, cards: QuizCard[] }` |
| `grade_quiz_item` | `{ sessionId, itemId, grade }` | `{ nextDueAt, status }` |
| `finish_quiz` | `{ sessionId }` | `QuizResult { total, remember, unsure, forgot, score, missed: VocabListItem[] }` |
| `list_quiz_history` | `{ limit }` | `QuizResult[]` (for a small chart on the Practice page) |

- `QuizCard = { itemId, kind, text, prompt, original?, answer: { meaning, example?, partOfSpeech?, ipa?, context? } }` (changed: `text` for 🔊, `original` for sentence/correction cards)
  - For `sentence` and `correction` kinds: `prompt` = "How would you say this correctly?" plus the original sentence (from `notes`).
  - For other kinds: `prompt` = `text`.
- `score = (remember + 0.5 × unsure) / graded`.
- A session that is never finished keeps `finished_at = NULL` and is excluded from history. **Its graded items still count** (they were saved when graded).
- In Hibernate: `Hibernating`.

---

---

## 9. UI

| Route / place | Content |
|---|---|
| `/reader/:id` | + selection popup (§4) and known-word highlights (§4.1) |
| `/wordbook` | Search box + filter chips (kind, status, due, pending meaning) + list (text, kind icon, short meaning or *Meaning pending*, status dot, due date) + **Export CSV** |
| `/wordbook/:id` | Detail: text 🔊, editable meaning, part of speech, examples, notes; contexts (sentence with the word highlighted, plus a link to the article); review history; Delete (with confirm) |
| `/practice` | Start screen: due/new counts, size selector, last 5 scores, **[Start]** (disabled with a reason if < 3 items) |
| `/practice/session` | The card flow from SPEC §10.3. Keys: Space = show answer; 1/2/3 = Forgot/Not Sure/Remember; Esc = quit (confirm). 🔊 auto-plays when the answer is shown if "Auto-pronounce" is on (setting, default on). Progress bar. |
| `/practice/result` | Score (big), counts, missed items (click to open), [Practice missed again] (starts a quiz with those items only — `start_quiz { itemIds }`), [Done] |
| Widget footer | `📚 12 due · 3 new · Practice` → opens `/practice`, to the right of the news status. It is hidden if the Word Book is empty. Updated on `vocab://changed` and after quizzes. |
| Sidebar | Word Book (⌘3) and Practice (⌘4); Settings moved to ⌘5 |
| Settings › Voice | §5 |
| Settings › Learning | quiz size, desired retention, auto-pronounce |

> `start_quiz { itemIds?: number[] }` is added so "Practice missed again" can quiz a given list of items.

New events:
- `vocab://changed {itemId}`
- `article://body {articleId, status}`
- `article://needs-body {articleId}` (declared in P1, used now)

---

---

## 10. Tasks

| # | Task | Depends on | Done when |
|---|---|---|---|
| T1 | Migration 0004 + vocab repos | – | Migration test; repo CRUD tests; retention protects vocab-linked articles |
| T2 | Dictionary FFI + parser + `dictionary_lookup` (§3.1) | – | Fixture parse tests; ignored live test passes on this Mac |
| T3 | `define_term` + auto-fill job (§3.2, §3.3) | T1 | Schema-valid JSON test with MockProvider; auto-fill fills only empty fields; never loads the model |
| T4 | Vocab service + commands + Hibernate gating (§6) | T1 | Merge rules, key collisions, Hibernate rejection tests |
| T5 | SRS (§7) | T1 | Crate-weight check done; §7 tests |
| T6 | Quiz selection + sessions (§8) | T4, T5 | Deterministic bucket-order tests; score formula; < 3 items error; `itemIds` path |
| T7 | TTS lib + Settings › Voice + Listen (§5) | – | Manual: voices listed; rate audible; Stop works; disabled in Hibernate |
| T8 | Selection popup + highlights (§4) | T2–T4, T7 | Manual: SPEC §18 P4 items 1, 2; popup works on chat answers |
| T9 | Word Book UI + CSV export (§9) | T4 | Manual: filter, edit, delete; CSV opens in Numbers |
| T10 | Practice UI + widget footer (§9) | T6 | Manual: SPEC §18 P4 items 3, 4 |
| T11 | QA + perf | all | `docs/qa/P4.md`; dictionary lookup time recorded |

Suggested order: (T1 ∥ T2 ∥ T7) → (T3 ∥ T4 ∥ T5) → (T6 ∥ T8 ∥ T9) → T10 → T11

---

## 11. Manual QA checklist (`docs/qa/P4.md`)

- [ ] Select "inference" in a story → dictionary meaning + pronunciation appear quickly; 🔊 speaks it slowly
- [ ] Select a phrase ("data lakehouse") → "Not in the dictionary" or a phrase entry; **Explain simply** gives an AI meaning
- [ ] Select a word inside an AI chat answer → the popup works there too
- [ ] Add → the item is in the Word Book with meaning, pronunciation and context; re-adding from another story adds a second context
- [ ] Items added in Hibernate are blocked with a clear message; the dictionary still works
- [ ] 10-item quiz: the keyboard shortcuts work; the score matches a hand calculation; the due count changes
- [ ] Forgot → the item can appear again 10+ minutes later
- [ ] An item without a meaning gets filled after the AI has been used
- [ ] CSV export opens correctly

---

## 12. Risks

| Risk | Mitigation |
|---|---|
| The Dictionary Services output format varies by dictionary or macOS version | Tolerant parser + `raw` fallback; fixture tests |
| The user's default dictionary is not English | The parser detects a non-English result (no English POS words) → show `raw` + suggest "Explain simply" |
| `fsrs` crate weight | Port the formulas (§1 note) |
| The CSS Highlight API is missing | Feature-detect; highlights are optional |
