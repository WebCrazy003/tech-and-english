# P2 — Reader, Word Book & Practice (dev spec)

- **Goal:**
  - Read the daily pick (or any article) in a clean, learner-friendly reader.
  - Save words and phrases with context.
  - Practise them with a 10-item quiz scheduled by FSRS.
  - Still **no AI.** Meanings are typed by the user, or left "pending" for P3.
- **SPEC sections:** §8.1, §8.2, §8.3 (Original mode), §8.4 (P2 popup), §9, §10, §13 (P2), §14 (P2), §18 P2.
- **Branch:** `p2/reader-wordbook` → tag `v0.2.0`

## 0. Scope

**In scope:**
- Article body fetch + Readability extraction + sanitizing.
- Paywall detection.
- Difficulty and reading time.
- Pick integration: paywall retry; difficulty shown on the card.
- Reader page (Original mode); read tracking.
- TTS (speechSynthesis) for 🔊 and **Listen** (title + description).
- Voice settings.
- Word Book (add/merge/edit/delete/list/detail/CSV export).
- Selection popup; highlights for known words.
- FSRS scheduling; quiz selection; quiz sessions and scoring.
- Practice UI.
- Due count in the widget.
- Hibernate gating for Word Book and Practice.

**Out of scope:** B1 Summary / Easy English / Explain / Ask AI (P3), and anything related to voice input (P4).

**Changes to the P1 UI:**
- Widget **[Read]** now opens the Reader in the main window. "Open in browser" moves to the **[⋯]** menu.
- **[Listen]** appears.

---

## 1. Dependencies

| Where | Add |
|---|---|
| npm | `@mozilla/readability@^0.6.0`, `dompurify@^3.4.16`; dev: `jsdom` (for Vitest DOM tests) |
| Cargo | `fsrs = "6.6"`, `fastrand = "2"` (seedable RNG for quiz sampling) |
| Resources | `src-tauri/resources/ngsl.txt`: **New General Service List 1.2**, lemmatized forms (~2,800 headwords + inflections). License **CC BY-SA 4.0**. The attribution goes in About and in `docs/licenses.md`. |

> **Why NGSL.** SPEC §8.2 asks for "the most common 3,000 English words" under a redistributable license. NGSL is designed for English learners, has about that size, and its license allows bundling with attribution.
>
> **Fallback** if the lemmatized file is unavailable: use the headword list plus a suffix-stripping lemmatizer (`-s`, `-es`, `-ed`, `-ing`, `-er`, `-est`, `-ly`, `-ies→y`, `-ied→y`).

> **fsrs crate risk.** The `fsrs` crate may pull in heavy training dependencies. At T9 start, check the clean build time and binary size impact.
>
> If it adds more than 60 s to a clean build or more than 5 MB to the binary: port **only the scheduling formulas** (FSRS-6 `next_states` with the default parameters, no training) into `learning/srs.rs`. Then add a test that compares against values precomputed with the crate.

---

## 2. Database — `migrations/0002_vocab.sql`

- Create `vocab_items`, `vocab_contexts`, `quiz_sessions` and `vocab_reviews` exactly as in SPEC §13, **with one change:**
  - `vocab_contexts.conversation_id INTEGER` is created **without** `REFERENCES conversations(id)`, because that table only arrives in P4.
  - P4's migration will not add the foreign key. The app deletes contexts explicitly when a conversation is deleted.
- Add `CREATE INDEX idx_vocab_contexts_item ON vocab_contexts(item_id);` and `CREATE INDEX idx_vocab_reviews_item ON vocab_reviews(item_id, reviewed_at);`.
- **Retention extension:** articles referenced by `vocab_contexts.article_id` are never deleted.

---

## 3. Body extraction

### 3.1 Flow

```
Frontend (lib/extract.ts) ensureBody(articleId)
  ├─ article.bodyStatus != "none" → return cached
  ├─ invoke fetch_article_html { articleId } → { html, finalUrl, paywallHint }
  ├─ doc = new DOMParser().parseFromString(html, "text/html"); insert <base href=finalUrl>
  ├─ if !isProbablyReaderable(doc) → save_article_body { status:"failed" }
  ├─ r = new Readability(doc, { charThreshold: 300 }).parse()
  ├─ clean = DOMPurify.sanitize(r.content, { ALLOWED_TAGS: [p,h2,h3,h4,ul,ol,li,blockquote,pre,code,em,strong,a,img,figure,figcaption,table,thead,tbody,tr,th,td,br],
  │                                          ALLOWED_ATTR: [href, src, alt, title] })
  └─ invoke save_article_body { articleId, text: r.textContent, html: clean, canonicalUrl, paywallHint }
```

- A per-article in-flight map prevents double extraction.
- The Reader window runs extraction when an article is opened. The widget runs it when it receives `article://needs-body`.

### 3.2 `fetch_article_html` (Rust)

- GET with the app User-Agent, 15 s timeout, following up to 5 redirects, stopping at **3 MB**.
- Only `text/html` is accepted. Anything else → `Invalid("not an HTML page")`.
- Returns `finalUrl` (after redirects) and `canonicalUrl` if a `<link rel="canonical">` is found with a regex on the `<head>`.
- **`paywallHint = true`** if the raw HTML contains any of:
  - `"isAccessibleForFree":false` / `"isAccessibleForFree": "False"` (JSON-LD)
  - `class="paywall`
  - `subscribe to continue`
  - `subscribers only`
  - `create a free account to continue`
  - `this article is for subscribers`

  The check is case-insensitive. The list is kept in `news/paywall.rs`.

### 3.3 `save_article_body` (Rust)

- `word_count` = the number of Unicode word tokens in `text`.
- `body_status`:
  - `failed` if the frontend says so, or if `word_count < 80`.
  - Otherwise `paywalled` if `paywallHint && word_count < 400`, or if `word_count < 150 && description.len() > 200`.
  - Otherwise `ok`.
- If `ok`: compute difficulty (§4) and store `canonical_url`. If another article already has the same **canonical** URL, merge this one into it (the P1 dedupe merge path) and return the surviving id.
- Wake the pick waiters (§5).
- Emit `article://body {articleId, status}`.

---

## 4. Difficulty — `learning/difficulty.rs` (pure)

```rust
pub struct DifficultyReport { pub word_count: u32, pub reading_minutes: u32, pub fre: f64, pub rare_ratio: f64, pub level: Level }
pub fn analyze(text: &str, common: &CommonWords) -> DifficultyReport
```

- **Sentences:** split on `[.!?]+` followed by whitespace or the end of the text. Ignore sentences of fewer than 3 words.
- **Syllables per word (heuristic):**
  1. Lowercase; keep only a–z.
  2. Count vowel groups (`[aeiouy]+`).
  3. Subtract 1 for a trailing silent `e` (but not `le` after a consonant).
  4. The minimum is 1.
- `FRE = 206.835 − 1.015 × (words / sentences) − 84.6 × (syllables / words)`
- `rare_ratio` = the share of alphabetic tokens (length ≥ 3) that are **not** in `common`, after lemmatizing. Numbers, ALL-CAPS acronyms of 2–5 letters, and CamelCase tokens are excluded from both the numerator and the denominator, because they are product and technical names.
- `reading_minutes = max(1, round(word_count / 150))`
- `level` uses the thresholds in SPEC §8.2.
- `CommonWords` is loaded once from `resources/ngsl.txt` into a `HashSet<String>`.

**Tests:**
- Known sample texts: a simple paragraph → Easy; a dense technical paragraph → Hard.
- Syllable counts: `cat` 1, `table` 2, `make` 1, `scalability` 5, `inference` 3.
- The acronym `AWS` is excluded from the ratio.

---

## 5. Pick integration (`news/pick.rs` P2 `post_select`)

```
candidates = eligible, sorted by score (SPEC §7.8)
for c in candidates.take(3):
    if c.body_status == none:
        register waiter(c.id); emit article://needs-body {c.id}
        status = await waiter with timeout 30 s   (timeout → treat as "unknown")
    if status == paywalled: continue
    return c        // difficulty/reading time filled if status == ok
return candidates[0] if every candidate was paywalled or timed out
```

- `BodyWaiters = Mutex<HashMap<i64, Vec<oneshot::Sender<BodyStatus>>>>` is kept in `NewsService`.
- **Test:** use a fake `EventSink` that simulates the frontend by calling `save_article_body` with fixture text. Cover: first candidate paywalled → the second is picked; a timeout → the first is picked with unknown difficulty.

`DailyPick` / `ArticleListItem` DTOs gain these fields:

```ts
bodyStatus: "none" | "ok" | "failed" | "paywalled";
difficulty: "easy" | "medium" | "hard" | null;
readingMinutes: number | null;
```

The widget line becomes: `Topic · Source · 3h` / `Medium English · 6 min read`.

---

## 6. Reader (`/reader/:id`)

### 6.1 Layout

- **Top:** title, source, published date, `Difficulty · N min read`, topic chips, and buttons **[Open in browser] [Save] [Not interested] [Listen]**.
- **Mode tabs:** **Original** (active). **B1 Summary** and **Easy English** are shown disabled, with the tooltip "Available in a later version" (enabled in P3).
- **Body:** the sanitized HTML, max width 680 px, 18 px font, 1.6 line height.
- **Settings menu (Aa):** font size (16/18/20/22) and line spacing, both persisted.
- **Failed / paywalled:** show the title and description, "We couldn't load the full article", and [Open in browser].

### 6.2 CSP (`tauri.conf.json > app.security.csp`)

```
default-src 'self'; img-src 'self' https: data:; style-src 'self' 'unsafe-inline'; connect-src ipc: http://ipc.localhost
```

- Remote images are allowed and rendered with `loading="lazy"` and `referrerpolicy="no-referrer"`.
- Scripts are never allowed from article content. DOMPurify removes them, and the CSP blocks them anyway.
- Links in the body open in the external browser: intercept the click, then call `openUrl`.

### 6.3 Interactions

- `opened` is recorded when the Reader mounts. P1's `open_article` is still used for "Open in browser".
- `read` is recorded **once** when either:
  - the user has scrolled to ≥ 60 % of the body height, **or**
  - the view has been visible for ≥ 50 % of `readingMinutes` (a timer that pauses when the window is hidden, using `document.visibilityState`).

### 6.4 Selection popup

This appears on `mouseup` / `keyup` inside the body.

**Text:**
- Take `window.getSelection().toString().trim()`. It must be 1–8 words and ≤ 80 chars.
- **Context sentence:** take the closest block element's `textContent` and split it with `Intl.Segmenter("en", { granularity: "sentence" })`. The sentence that contains the selection's start offset is the context.

**Popup content** (anchored to `range.getBoundingClientRect()`):

```
inference                        🔊
[ word ▾ ]   (auto: 1 word → word, 2+ → phrase; user may pick "technical term")
Meaning (optional): [______________]
Context: "…the model performs inference on your laptop…"
[Add to Word Book]      (in the Word Book already → "Add this context")
```

- Esc or clicking outside closes the popup.
- In Hibernate, the Add button is disabled and shows the tooltip "Switch to Standard to add words".

**Highlights for known words:**
- After the body renders, use the **CSS Custom Highlight API** (`CSS.highlights.set("known-vocab", new Highlight(...ranges))`) to underline words and phrases whose `text_key` matches the Word Book. This does not change the DOM.
- Style: `::highlight(known-vocab) { text-decoration: underline dotted 1.5px; }`
- If `CSS.highlights` is missing, skip highlighting.
- Matching:
  1. Tokenize the text nodes.
  2. Compare lowercased tokens and 2–4-token n-grams against a `Set` loaded by `list_vocab_keys()`.
  3. Stop after 2,000 ranges.

---

## 7. TTS (`src/lib/tts.ts`)

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
- **Listen (P2):** speaks `title. description` with `speakSentences`. The button toggles to **Stop** while speaking.
- In Hibernate, TTS is disabled (SPEC §6) and the buttons show the tooltip "Switch to Standard".
- `Settings` gains `tts: { voiceUri: Option<String>, rate: f32 (0.85), volume: f32 (1.0), pauseMs: u32 (400) }`.

---

## 8. Word Book service (`learning/vocab.rs`)

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

**Commands (P2):**

| Command | Args | Returns |
|---|---|---|
| `add_vocab_item` | `NewItem` | `{ outcome: "created" \| "merged", item: VocabItem }` |
| `update_vocab_item` | `{ id, patch }` | `VocabItem` |
| `delete_vocab_item` | `{ id }` | `void` |
| `list_vocab` | `{ filter: { query?, kind?, status?, dueOnly?, articleId?, pendingOnly? }, cursor?, limit? }` | `Page<VocabListItem>` |
| `get_vocab_item` | `{ id }` | `VocabItemDetail` (item + contexts with article titles + last 20 reviews) |
| `list_vocab_keys` | – | `string[]` (for highlights) |
| `due_count` | – | `{ due: number, new: number }` |
| `export_vocab_csv` | – | `{ path }` (writes to `~/Downloads/tech-english-wordbook-YYYY-MM-DD.csv`, then reveals it in Finder with `opener`) |

- The CSV columns are: kind, text, meaning_simple, meaning_b1, part_of_speech, examples (joined with ` | `), status, review_count, due_at, first_context, source_article_url.
- **Hibernate:** every write command returns `AppError::Hibernating`. Reads are allowed.

---

## 9. SRS (`learning/srs.rs`)

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

## 10. Quiz (`learning/quiz.rs`)

### 10.1 Selection

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

### 10.2 Commands

| Command | Args | Returns |
|---|---|---|
| `start_quiz` | `{ size?: number }` (default setting 10, range 5–30) | `{ sessionId, cards: QuizCard[] }` |
| `grade_quiz_item` | `{ sessionId, itemId, grade }` | `{ nextDueAt, status }` |
| `finish_quiz` | `{ sessionId }` | `QuizResult { total, remember, unsure, forgot, score, missed: VocabListItem[] }` |
| `list_quiz_history` | `{ limit }` | `QuizResult[]` (for a small chart on the Practice page) |

- `QuizCard = { itemId, kind, prompt, answer: { meaning, example?, partOfSpeech?, context? } }`
  - For `sentence` and `correction` kinds: `prompt` = "How would you say this correctly?" plus the original sentence (from `notes`).
  - For other kinds: `prompt` = `text`.
- `score = (remember + 0.5 × unsure) / graded`.
- A session that is never finished keeps `finished_at = NULL` and is excluded from history. **Its graded items still count** (they were saved when graded).
- In Hibernate: `Hibernating`.

---

## 11. UI

| Route / place | Content |
|---|---|
| `/reader/:id` | §6 |
| `/wordbook` | Search box + filter chips (kind, status, due, pending meaning) + list (text, kind icon, short meaning or *Meaning pending*, status dot, due date) + **Export CSV** |
| `/wordbook/:id` | Detail: text 🔊, editable meaning, part of speech, examples, notes; contexts (sentence with the word highlighted, plus a link to the article); review history; Delete (with confirm) |
| `/practice` | Start screen: due/new counts, size selector, last 5 scores, **[Start]** (disabled with a reason if < 3 items) |
| `/practice/session` | The card flow from SPEC §10.3. Keys: Space = show answer; 1/2/3 = Forgot/Not Sure/Remember; Esc = quit (confirm). 🔊 auto-plays when the answer is shown if "Auto-pronounce" is on (setting, default on). Progress bar. |
| `/practice/result` | Score (big), counts, missed items (click to open), [Practice missed again] (starts a quiz with those items only — `start_quiz { itemIds }`), [Done] |
| Widget footer | `📚 12 due · 3 new  [Practice]` → opens `/practice`. It is hidden if the Word Book is empty. Updated on `vocab://changed` and after quizzes. |
| Settings › Voice | §7 |
| Settings › Learning | quiz size, desired retention, auto-pronounce |

> `start_quiz { itemIds?: number[] }` is added so "Practice missed again" can quiz a given list of items.

New events:
- `vocab://changed {itemId}`
- `article://body {articleId, status}`
- `article://needs-body {articleId}` (declared in P1, used now)

---

## 12. Tasks

| # | Task | Depends on | Done when |
|---|---|---|---|
| T1 | Migration 0002 + vocab repos | – | Migration test; repo CRUD tests; retention protects vocab-linked articles |
| T2 | `fetch_article_html`, `save_article_body`, paywall, canonical merge | T1 | wiremock tests: HTML ok, non-HTML rejected, 3 MB cap, redirect → finalUrl; status rules table test |
| T3 | `difficulty.rs` + NGSL resource + license file | – | §4 tests |
| T4 | `lib/extract.ts` (Readability + DOMPurify) | T2 | Vitest (jsdom) on 3 fixture pages: normal article, list page (not readerable), page with `<script>` → the script is removed |
| T5 | Pick integration (waiters, paywall retry, card fields) | T2–T4 | §5 tests; the widget shows difficulty and reading time |
| T6 | TTS lib + Voice settings + Listen | – | Manual: voices listed; rate change audible; Stop works; disabled in Hibernate |
| T7 | Reader page + CSP + read tracking | T4 | Manual: SPEC §18 P2 item 1; `read` recorded once |
| T8 | Vocab service + commands + Hibernate gating | T1 | §8 tests (merge rules, key collisions, Hibernate rejection) |
| T9 | SRS | T1 | Crate-weight check done (keep the crate or port the formulas); §9 tests |
| T10 | Quiz selection + sessions | T8, T9 | Deterministic bucket-order tests; score formula; < 3 items error; `itemIds` path |
| T11 | Selection popup + highlights | T7, T8 | Manual: add a word, re-add → "context added"; highlights appear after adding |
| T12 | Word Book UI + CSV export | T8 | Manual: filter, edit, delete; the CSV opens in Numbers with correct columns |
| T13 | Practice UI + widget footer | T10 | Manual: SPEC §18 P2 items 4, 5 |
| T14 | QA + perf | all | `docs/qa/P2.md` complete; the reader-open time and idle memory with the Reader open are added to `docs/perf.md` |

Suggested order: T1 → (T2 ∥ T3 ∥ T6 ∥ T9) → T4 → (T5 ∥ T7 ∥ T8) → (T10 ∥ T11 ∥ T12) → T13 → T14

---

## 13. Manual QA checklist (`docs/qa/P2.md`)

- [ ] Open the daily pick → clean text, images load, links open in the browser, no scripts run
- [ ] Open a known paywalled article → the fallback screen appears; that article is never chosen as the daily pick when an alternative exists
- [ ] Difficulty and reading time appear in the widget and in Explore once the body is loaded
- [ ] Select a word → popup with context; Add → it appears in the Word Book with its context and a link back to the article
- [ ] Select the same word in another article → "context added"; the Word Book item shows 2 contexts
- [ ] Words already saved are underlined in new articles
- [ ] Items without a meaning show *Meaning pending* and never appear in quizzes
- [ ] Practice with 10+ items: the keyboard shortcuts work; the result score matches a hand calculation; the due count changes
- [ ] Forgot on an item → it can appear again in a new quiz 10+ minutes later
- [ ] Hibernate: can browse the Word Book, can't add, edit or practise; TTS is disabled; clear messages
- [ ] CSV export opens correctly

## 14. Risks

| Risk | Mitigation |
|---|---|
| Readability fails on some sites (JS-rendered pages) | `failed` status + Open in browser; the pick moves on after paywall/fail |
| The CSS Highlight API is unavailable in the WebKit version | Feature-detect; highlights are optional |
| `fsrs` crate weight | Port the formulas (§1 note) |
| Selection inside links triggers navigation | Intercept clicks: only navigate on click without a selection |
