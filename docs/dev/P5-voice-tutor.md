# P5 — Voice English tutor (dev spec)

- **Goal:** a push-to-talk conversation about an article.
  - The tutor speaks at the learner's level and corrects important mistakes.
  - It asks the learner to repeat corrections, and it explains words without losing the topic.
  - It runs simple pronunciation drills.
  - At the end, it offers a review where the learner chooses what goes into the Word Book.
- **SPEC sections:** §6 (voice rows), §12, §13 (P5), §14 (P5), §16, §18 P5.
- **Branch:** `p5/voice-tutor` → tag `v0.5.0`

## 0. Scope

**In scope:**
- Whisper sidecar (`whisper-server`), with the lifecycle code shared with the LLM
- Microphone capture, resampling, WAV encoding, level meter
- STT client with a hallucination filter
- Streaming `tutor_turn` with a reply extractor
- Local intents; local repeat check; conversation state machine
- Pronunciation drill
- Session log (observations) and `session_review` with a non-LLM fallback
- Talk UI (setup, live session, review, history)
- Hibernate/Quit handling; latency instrumentation; a golden correction test suite

**Out of scope:**
- Phoneme-level pronunciation scoring
- VAD beyond a simple silence auto-stop
- Saving audio (except the debug flag)

---

## 1. Dependencies

| Where | Add |
|---|---|
| Cargo | `cpal = "0.18"`, `rubato = "5.0"`, `hound = "3.5"`; `reqwest` gains the `multipart` feature; `serde_json` gains `preserve_order` (T0: otherwise the schema's keys are sorted and `reply` is not first) |
| Dev machine | `brew install whisper.cpp` (1.9.4 at the time of writing). T0: there is no bottle for macOS 14, so it builds from source (needs `cmake`); run it from `~`, not from inside `~/Documents`. |
| Models (`models.json`, role `stt`) | `ggml-base.en.bin` (~150 MB, default), `ggml-small.en.bin` (~490 MB, optional), from the official whisper.cpp model repository on Hugging Face. sha256 and the license (MIT) are recorded in T0. |
| Bundle | `src-tauri/Info.plist` with `NSMicrophoneUsageDescription` = "Tech English uses the microphone only while you hold the talk button, to practise speaking English. Audio is processed on this Mac and not saved." |

---

## 2. T0 — Spike: whisper-server + tutor schema (½ day)

Record the results in `docs/dev/notes/P5-whisper-and-tutor.md`. **Done 2026-09-30** — the deltas below are marked "(T0)".

**whisper-server:**
1. Confirm the binary names installed by the formula (`whisper-server`, `whisper-cli`).
2. Confirm the flags: `-m`, `--host`, `--port`, `-t`, `-l en`.
3. Confirm the `/inference` multipart fields: `file`, `temperature`, `response_format=json`.
4. Record the health or ready behaviour (does it listen only after the model has loaded?).
5. Note whether it supports any authentication. We expect that it does not; see Risks.
6. Measure the transcription time of 3 s, 8 s and 20 s clips with `base.en` and `small.en`, and the RSS memory.

**llama.cpp JSON schema** (using P2's setup):
1. Confirm that the `anyOf: [null, object]` form for `correction` is supported by the grammar conversion.
2. Confirm that **properties are generated in schema order**, so `reply` streams first. If they are not, split the output into two parts (see §6.3 fallback).

**macOS microphone permission in `tauri dev`:**
1. Record which app the permission prompt is attributed to. We expect the terminal or IDE running the command.
2. Confirm that the bundled `.app` shows its own prompt with the Info.plist text.

---

## 3. Shared sidecar manager (refactor)

- Extract the generic parts of P2's `AiManager` into `sidecar::SidecarManager<S: SidecarSpec>`:
  - launch
  - health check
  - PID file and orphan cleanup
  - idle timer and hold guards
  - stop
  - status events

  The spec for each sidecar provides its binary name, arguments, health probe and memory estimate.
- `LlmSidecar` and `WhisperSidecar` implement `SidecarSpec`. `ai://status` carries `component: "llm" | "stt"`.
- **Whisper launch:** `whisper-server -m <models>/<stt model> --host 127.0.0.1 --port <free> -t 4 -l en` (flags confirmed in T0).
- **Whisper idle timeout:** 5 min after a session ends. A voice session holds guards on **both** sidecars.
- **Binary resolution:** the same order as P2 §8.3, with `settings.ai.whisperServerPath`.
- All P2 AiManager tests must still pass after the refactor, and they are duplicated for the whisper spec.

---

## 4. Audio capture (`voice/capture.rs`)

```rust
pub struct Recorder { /* cpal stream, buffer, device rate/channels */ }
impl Recorder {
    pub fn start(&mut self, level_tx: mpsc::Sender<f32>) -> Result<()>;   // opens input stream
    pub fn stop(&mut self) -> Result<Clip>;                                // closes stream → mic indicator off
}
pub struct Clip { pub samples_16k_mono: Vec<f32>, pub duration: Duration, pub rms: f32 }
```

**Device:**
- Use the default input device and its default config.
- If there is no device → `AppError::Invalid("No microphone found")`.
- A permission error → a typed error code `mic_denied`.

**Processing:**
- Mix down to mono by averaging the channels.
- Resample to 16 kHz with `rubato` (a sinc resampler created once per recording).
- Encode to an in-memory 16-bit PCM WAV with `hound` (a `Cursor<Vec<u8>>`).

**Limits:**
- Auto-stop at 60 s. The session is told `reason: "max_length"`.
- A clip shorter than 0.3 s, or with RMS below the silence threshold (−50 dBFS), → "I didn't hear anything. Hold the button while you speak."
  - The tutor does not respond.
  - No STT call is made.

**Other rules:**
- **Level meter:** RMS per 50 ms → `voice://level {rms}`, at most 20 per second.
- **VAD auto-stop** (setting, default off): once speech has been detected, stop after 1.2 s below −45 dBFS.
- **Privacy:** buffers are dropped after the STT call returns. When `debug.keepAudio` is true, also write `app_data_dir/debug-audio/<conversation>-<seq>.wav` and show a red "Recording saved (debug)" badge.
- The cpal stream lives on its own thread (cpal streams are `!Send` on some platforms). Communicate with it through channels.

---

## 5. STT (`voice/stt.rs`)

```rust
#[async_trait] pub trait SttProvider: Send + Sync {
    async fn transcribe(&self, wav: Vec<u8>) -> Result<Transcript, AppError>;  // { text, duration_ms }
}
```

- `WhisperServerProvider` posts multipart to `/inference` with `temperature=0` and `response_format=json`, and **no prompt**. It times out after 30 s.
- **Post-processing:**
  1. Trim.
  2. Collapse whitespace.
  3. Remove bracketed non-speech tags (`[BLANK_AUDIO]`, `(music)`, `[inaudible]`).
- **Hallucination filter.** If the clip RMS is below −40 dBFS **and** the text (after punctuation is removed) is on the list of known silence hallucinations, return an empty transcript. The list: "thank you", "thanks for watching", "you", "bye", "thank you for watching", "subtitles by …".
- `FakeStt` returns scripted transcripts for tests.

---

## 6. Tutor turn

### 6.1 Schema (`TutorTurnOut`)

```json
{
  "type": "object",
  "required": ["reply", "correction", "unknown_terms", "useful_phrases"],
  "properties": {
    "reply": { "type": "string", "maxLength": 700 },
    "correction": { "anyOf": [ { "type": "null" }, {
        "type": "object",
        "required": ["original", "corrected", "explanation", "ask_repeat"],
        "properties": {
          "original":    { "type": "string", "maxLength": 300 },
          "corrected":   { "type": "string", "maxLength": 300 },
          "explanation": { "type": "string", "maxLength": 200 },
          "ask_repeat":  { "type": "boolean" } } } ] },
    "unknown_terms": { "type": "array", "maxItems": 3, "items": { "type": "object",
        "required": ["text", "explanation"],
        "properties": { "text": { "type": "string", "maxLength": 60 }, "explanation": { "type": "string", "maxLength": 200 } } } },
    "useful_phrases": { "type": "array", "maxItems": 2, "items": { "type": "string", "maxLength": 80 } },
    "grammar_note": { "anyOf": [ { "type": "null" }, { "type": "string", "maxLength": 200 } ] }
  }
}
```

### 6.2 Prompt (`ai/prompts/tutor_turn.md`)

```
### system
You are a friendly English speaking tutor. The learner works in technology. Their level: {{level_name}}.
Your priorities, in order:
1. Help the learner improve their spoken English.
2. Have an interesting conversation about the article below.

Speaking rules:
{{level_rules}}
- Your reply will be spoken aloud. No lists, no markdown, no emojis. At most {{max_sentences}} sentences.
- End with ONE question that invites the learner to speak — unless you are asking them to repeat a sentence.
- If the learner asks about a word or says they don't understand, explain it simply with one example,
  then return to the question you asked before.
- If the learner asks you to explain simply, use shorter sentences and more common words.

Correction rules ({{correction_policy}}):
{{correction_rules}}
- The learner's words come from speech recognition. Ignore punctuation, capitalisation, filler words
  (um, uh), repeated words and false starts. Never correct those.
- Correct at most ONE mistake per turn — the most important one.
- When you correct: start the reply with the natural sentence, give a very short reason, then end the
  reply with "Please say: <corrected sentence>". Nothing comes after it: no question. Set ask_repeat = true.

Fields:
- correction: null, or original = the learner's whole sentence, corrected = the whole corrected sentence.
- unknown_terms: words the learner asked about or clearly did not understand (with your simple explanation).
- useful_phrases: up to 2 natural phrases from YOUR reply that are worth learning.

Article: {{article_title}}
Summary:
"""
{{article_summary}}
"""

### user
(Mistakes already noted in this session: {{session_mistakes}}.)     ← only when there are some
{{turn_instruction}}
Learner said: "{{transcript}}"
```

> **(T0)** `session_mistakes` is in the **user** message: changing the system prompt would make llama-server re-read the whole history. The Fields line for `correction` and "Nothing comes after it" were added because the model otherwise returned only the changed word, or asked a question after "Please say".

**Values filled in:**

| Placeholder | Values |
|---|---|
| `max_sentences` | Level 1: 3 · Level 2: 4 · Level 3: 5 |
| `correction_rules` (Low) | "Only correct mistakes that change the meaning or make it hard to understand." |
| `correction_rules` (Medium) | "Correct important grammar mistakes (verb tense, subject–verb agreement, word order, wrong word) and any mistake the learner has made before in this session." |
| `correction_rules` (High) | "Correct any clear grammar or word-choice mistake." |

- **Opening turn:** `turn_instruction` = "Start the conversation: introduce the article in 2–3 sentences, then ask one easy question." and `transcript` = "".
- **Normal turn:** `turn_instruction` = "".
- **History:** the chat messages carry the last ≤ 12 exchanges (user + tutor), or ≤ 5 with a 4k context. The **system prompt stays byte-identical for the whole session**, so `cache_prompt` can reuse it. **(T0)** History messages are sent exactly as before — the same user message and, for the tutor, the raw JSON it wrote — so only the new message is read. Removing old turns forces a re-read (≈ 4 s for 8 exchanges), and `--cache-reuse` is not supported for Qwen3.5. So when the window is full, drop down to the **last 3 exchanges in one step**; local replies and intents are not part of the LLM history.
- **Article summary:** the cached B1 summary. If there is none, generate it first; this is part of the session start-up. For free talk: "No article. Talk about technology topics the learner likes: {{topics}}."

### 6.3 Streaming reply extraction (`voice/reply_stream.rs`)

`ReplyExtractor` is fed chunks and emits **complete sentences** of the `reply` string as they arrive:

1. **Scanning:** find `"reply"`, skipping whitespace, then `:`, then the opening `"`.
2. **InString:** decode JSON string escapes (`\" \\ \/ \n \t \uXXXX`, including surrogate pairs). Append to a buffer. When a character in `.?!` is followed by a space (or the string ends), emit the buffer as a sentence.
3. **After the string closes:** flush what is left and accumulate the rest of the JSON. At the end, parse the whole text into `TutorTurnOut`.

- Tests feed the same output split at **every** byte position. The sentences and the final struct must be identical.
- Decimal numbers ("version 2.5 is") must not split a sentence, because a sentence end needs a following space.
- **Fallback** (if T0 shows the key order isn't guaranteed): the prompt asks for plain reply text, then a line `@@META@@`, then the JSON without `reply`. The extractor streams until the marker. The schema is not enforced; the JSON is validated, with one retry of the meta part only.

---

## 7. Local logic (`voice/intents.rs`, `voice/similarity.rs`)

### 7.1 Intents

Match on the lowercased transcript, with punctuation removed. **The whole utterance must match**, so the phrases don't trigger inside normal sentences.

| Intent | Patterns (regex, anchored) | Action |
|---|---|---|
| Repeat | `^(can you |could you )?(please )?(say (that|it) again|repeat( that| it)?|pardon|sorry what|what did you say)( please)?$` | Frontend replays the last tutor reply |
| Slower | `^(please )?(speak|talk) (more )?slow(ly|er)( please)?$`, `^slower( please)?$` | Rate −0.1 (min 0.5); replay |
| Faster | `^(please )?(speak|talk) faster( please)?$`, `^faster( please)?$` | Rate +0.1 (max 1.2) |
| End | `^(stop|end|finish)( the)? (conversation|session|talking)( please)?$`, `^(let's|lets) stop( here)?$`, `^stop( please)?$` (SPEC §12.5 lists a bare "stop") | End the session, then review |
| Pronounce | `^how (do|should|can) (i|you|we) (say|pronounce) (the word )?(?P<w>.+)$` | Pronunciation drill (§8) |
| Define (logged only) | `(what does|what's the meaning of|what is the meaning of) (?P<w>.+?)( mean)?$`, `^what is (?P<w>[a-z-]+)$` | Log an `unknown_word` observation, **then** send to the LLM as normal |

### 7.2 Repeat check

```
normalize(s): lowercase; expand contractions (it's→it is, don't→do not, didn't→did not, I'm→I am,
              can't→cannot, won't→will not, we're→we are, they're→they are, isn't→is not …);
              strip punctuation; collapse spaces; drop fillers (um, uh, er)
similarity(a, b) = 1 − word_levenshtein(a, b) / max(len_a, len_b)
pass = similarity ≥ 0.85
```

**Tests:**
- An exact match passes.
- The contraction variant passes.
- A missing "-ed" (the deploy/deployed case) **fails** for a 5-word sentence. (1 of 5 words different → 0.8 < 0.85.)
- Fillers are ignored.

---

## 8. Session engine (`voice/session.rs`)

### 8.1 State

```rust
pub enum Phase { Discuss, AwaitRepeat { target: String, attempts: u8 }, Drill { word: String, hint: Option<String>, attempts: u8 } }
pub struct Session {
    id: i64, article_id: Option<i64>, settings: SessionSettings, phase: Phase,
    history: VecDeque<ChatMsg>,              // last 12 turns
    mistakes: Vec<String>,                   // short labels for {{session_mistakes}}
    speaking: Duration, last_tutor_reply: String,
    _holds: (HoldGuard, HoldGuard),          // llm + stt
}
```

Only one session can be active at a time. It is kept in `VoiceEngine { active: Mutex<Option<Session>> }`.

### 8.2 Turn processing

```
on_user_input(text, source: Voice | Typed):
  save turn(role=user)
  if text is empty → emit notice "I didn't hear anything…"; return
  if intent ∈ {Repeat, Slower, Faster, End, Pronounce} → emit local action; return
  match phase:
    AwaitRepeat{target, attempts}:
      if similarity(text, target) ≥ 0.85 → say "Good. Let's continue."; phase = Discuss;
           then LLM turn with turn_instruction "The learner repeated correctly. Continue the conversation."
      else if attempts == 0 → say "Almost. Listen again: <target>" (frontend: rate − 0.15); attempts = 1
      else → say "Good try. Let's continue."; phase = Discuss; LLM continue turn
      (these short replies are local: no LLM call)
    Drill{…} → §8.4
    Discuss:
      if Define intent → log unknown_word
      LLM tutor_turn (stream) → sentences to the frontend
      on done: save turn(role=tutor, meta=json)
               if correction → log grammar observation; push label to mistakes;
                               if ask_repeat → phase = AwaitRepeat{target: corrected, attempts: 0}
               unknown_terms → log unknown_word / unknown_phrase (with explanation)
               useful_phrases → log useful_sentence
```

- Local replies ("Good. Let's continue.") are saved as tutor turns with `meta = {"local": true}`.
- **(T0)** If `correction.ask_repeat` is true but the reply has no "please say", the engine adds the sentence "Please say: <corrected>" itself (the model does not always follow the rule).
- If an LLM turn errors: say "Sorry, I had a problem. Could you say that again?", keep the phase, and log the error (without content).

### 8.3 Commands (Channel-based)

| Command | Args | Behaviour |
|---|---|---|
| `start_voice_session` | `{ articleId?, settings, channel }` | Takes the holds; starts the LLM and STT sidecars in parallel; ensures the B1 summary exists; creates the `conversations` row; streams the opening turn. Channel events: `loading{component}`, `ready`, then tutor events. |
| `start_recording` | – | Opens the mic. Error `mic_denied` / `no_device`. |
| `stop_recording` | `{ channel }` | Closes the mic → STT → `transcript{text}` → turn processing → tutor events |
| `send_text_turn` | `{ text, channel }` | Same flow without STT (typed fallback; used by tests) |
| `update_session_settings` | `{ rate?, level?, correction? }` | Level and correction changes apply from the next turn |
| `end_voice_session` | `{ reason: "user" \| "hibernate" \| "quit" }` | Sets `ended_at` and speaking seconds; releases the holds. If the reason is `user`, runs the review and returns `SessionReview`; otherwise `review_status` stays `pending`. |
| `get_session_review` | `{ conversationId }` | Builds or returns the review (for pending sessions from history) |
| `apply_session_review` | `{ conversationId, selected: Suggestion[] }` | Creates the Word Book items → `review_status=done`. An empty selection → `skipped`. (As built: the chosen suggestions themselves are sent, not ids, so an LLM review does not have to be rebuilt after a restart.) |
| `list_conversations` / `get_conversation` | – / `{ id }` | History |
| `report_latency` | `{ turn, ttsStartMs }` | Frontend timing (§10); `turn` is the event turn number |
| `voice_setup` | – | As built: default session settings, active session, speech model/engine found, mic permission |
| `start_drill` | `{ word, channel }` | As built: the drill from a tapped word |
| `get_active_session` | – | As built: for a reloaded window and the widget |
| `delete_conversations` | – | As built: Settings › Data |
| `voice_latency` | – | As built: p50/p90 for Settings › AI › Diagnostics |

Channel `VoiceEvent`:

```ts
| { kind: "loading"; component: "llm" | "stt" | "summary" }
| { kind: "ready"; conversationId: number }
| { kind: "transcript"; text: string }
| { kind: "notice"; text: string }                         // e.g. didn't hear anything
| { kind: "tutorSentence"; turn: number; text: string; rateDelta?: number; rate?: number }   // rate: absolute (drill word 0.6)
| { kind: "tutorDone"; turn: number; turnId: number; text: string; correction?: Correction; phase: "discuss" | "awaitRepeat" | "drill";
    repeatTarget?: string; drill?: { word, hint, attempt, maxAttempts }; local: boolean }
| { kind: "localAction"; action: "replay" | "slower" | "faster" | "end"; rate?: number }
| { kind: "error"; code: string; message: string }
```

### 8.4 Pronunciation drill

1. **Start:** from the Pronounce intent (`w` = the word, with quotes, "the word" and trailing punctuation stripped), **or** from UI: tap a word in a tutor bubble → **[Practise saying]** → `start_drill {word}`.
2. **Hint:** the cached `define_term` result for the word, if there is one; **(T0)** otherwise the macOS dictionary (syllables such as `in·fer·ence` plus the pronunciation with its stress mark, e.g. `ˈinf(ə)rəns`); only if both have nothing, one `define_term` call (P4) for `syllables`. (Any other request on llama-server's single slot evicts the tutor's cached prompt.) If it fails, there is no hint.
3. The tutor says "Listen: <word>". The frontend speaks the word at rate 0.6, and the UI shows the hint with the label "hint".
4. The user records. A **pass** means the normalized transcript contains the target word as a token. Plural and `-ed`/`-ing` variants of the target also pass.
   - Pass → "Good!", then back to `Discuss` (continue the conversation).
   - Fail → "I heard '<heard>'. Listen again:" + replay. Up to 3 attempts, then "Good try. Let's continue."
5. Log a `pronunciation` observation with `detail = {target, attempts: [{heard}], passed}`.

---

## 9. Session review

- `session_review` prompt (JSON), with the input:
  - the deduplicated observations (kind, text, detail)
  - the user turns (text only)
  - the article title
- Output schema: `{ suggestions: [{ id, kind: "word"|"phrase"|"term"|"correction"|"sentence", text, meaning_simple?, note?, preselected: bool, observation_ids: number[] }] }`, max 15.
- **Preselection guidance** in the prompt: preselect words the learner asked about, corrections (especially ones they repeated), and technical terms central to the article. Don't preselect very common words.
- **Fallback** (LLM error or invalid output twice): build the suggestions directly from the observations. Preselect all unknown words/phrases and corrections; do not preselect useful sentences.
- **Stats** for the summary screen: speaking minutes, the number of distinct unknown words/phrases, corrections, and pronunciation drills.
- **Apply** creates items through `vocab::add` (P4):
  - `context.sentence` = the user or tutor turn where the item appeared
  - `conversation_id` and `article_id` are set
  - for corrections: `kind=correction`, `text=corrected`, `notes = JSON {original, explanation}`
  - `meaning_simple` comes from the explanation when present; otherwise it stays pending, and P4 auto-fill completes it later
  - each observation's `saved_item_id` is set

---

## 10. Latency instrumentation

Per turn, `Instant`s are recorded in Rust:
- `rec_stop`
- `stt_done`
- `llm_first_token`
- `first_sentence`
- `llm_done`

The frontend reports `tts_start` (the `speechSynthesis` `start` event of the first sentence) with `report_latency`.

- The last 50 turns are kept in memory.
- A **Settings › AI › Diagnostics** panel shows the p50 and p90 of `rec_stop → tts_start` and of each stage.
- The values are written to the logs as numbers only.
- **Target** (SPEC §16): p50 ≤ 3 s, p90 ≤ 5 s.

---

## 11. Database — `migrations/0005_voice.sql`

- `conversations`, `conversation_turns` and `learning_observations` exactly as in SPEC §13.
- `CREATE INDEX idx_turns_conv ON conversation_turns(conversation_id, seq);`
- **Retention:**
  - Articles with conversations are never deleted.
  - Conversations older than `voice.keepTranscriptsDays` (default 90; 0 = forever) are deleted, unless `review_status = 'pending'` and the conversation is younger than 180 days.
  - When a conversation is deleted, the app also deletes its `vocab_contexts` rows with that `conversation_id` (there is no foreign key; see P4 §2).

---

## 12. UI

| Route | Content |
|---|---|
| `/talk` | Setup form (SPEC §12.2) prefilled from settings; the topic chooser defaults to today's pick; **[Start conversation]**; "Pending reviews" list; link to history |
| `/talk/session` | **Header:** article title, level chip, timer, speed −/+, correction frequency menu, **[End]**. **Transcript:** tutor bubbles (words tappable → popup with Explain / Add / Practise saying) and user bubbles. **Correction card:** original with the changed words struck through, corrected with them highlighted, the explanation, 🔊. **AwaitRepeat banner:** "Please say: …". **Drill card:** word, 🔊, hint, attempt dots. **Bottom:** large mic button (hold **Space**, or click to toggle), level meter, status text (Loading AI… / Your turn / Listening… / Transcribing… / Thinking… / Speaking…), collapsible typed input. |
| `/talk/review/:id` | Stats row; three checkbox groups (Words & phrases / Corrections / Useful sentences), each item with its meaning or note; **[Add selected to Word Book]** **[Skip]** |
| `/talk/history` | List (date, article, minutes, corrections, review status); open → read-only transcript (`/talk/history/:id`); "Finish review" for pending ones |
| Widget | While a session is active: the footer shows "🎙 Talking… [Open]" |

**Frontend TTS queue (`features/talk/speechQueue.ts`):**
- Sentences are queued and played in order with `pauseMs` between them. `rateDelta` applies to that sentence only.
- **Barge-in:** pressing the mic stops speech immediately (`speechSynthesis.cancel()`) and clears the queue.
- The mic button is **enabled while speaking** (this allows barge-in).

**Push-to-talk details:**
- `keydown` Space (ignore `repeat`, and ignore when focus is in a text input) → `start_recording`.
- `keyup` → `stop_recording`.
- Click mode: first click starts, second click stops.
- The window losing focus while recording → stop.

---

## 13. Hibernate & Quit

- `set_mode(hibernate)` with an active session → `AppError::SessionActive` (new code), unless `force: true`.
  - The UI shows a confirm dialog: "End the conversation and switch to Hibernate? You can finish the review later."
  - On confirm → `end_voice_session {reason: "hibernate"}`, then `set_mode {force: true}`.
  - The tray's Hibernate item with an active session opens the main window and shows the same dialog.
- **Quit** with an active session: the same confirm, then `end_voice_session {reason: "quit"}`, then the sidecars stop (3 s budget).

---

## 14. Tests

**Unit:**
- `ReplyExtractor` (every-split test, escapes, decimals)
- intents (positives, and negatives inside longer sentences)
- `similarity` and contractions
- session phase transitions with `MockProvider` + `FakeStt`:
  - correction → AwaitRepeat → pass → Discuss
  - fail twice → Discuss
  - Define intent logs + LLM call
  - local intents make no LLM call
  - drill pass/fail/3 attempts
  - empty transcript → notice, no LLM
- review fallback
- apply review creates the right items and contexts
- retention rules

**Golden suite** (`cargo test --release -- --ignored golden_tutor`, real models):
- `tests/golden/tutor_corrections.jsonl` has **30** cases. Each case is `{ "input", "policy", "expect_correction", "expect_contains"? }`.
- Pass criteria: ≥ 90 % agreement on `expect_correction`, and **0** corrections on the disfluency cases.

The seed cases (extend them to 30):

| input | policy | expect_correction | expect_contains |
|---|---|---|---|
| Yesterday I deploy the application. | high | true | deployed |
| I am agree with this idea. | high | true | I agree |
| He don't use Docker at work. | medium | true | doesn't |
| The model run on my laptop. | medium | true | runs |
| I have used Python since three years. | medium | true | for three years |
| It depends of the data size. | high | true | depends on |
| We discussed about the new release. | high | true | discussed the |
| I didn't understood the last part. | medium | true | didn't understand |
| Can you explain what is inference? | high | true | what inference is |
| Can you explain what is inference? | low | false | |
| I think it is very useful for developers. | high | false | |
| The data is stored in a lakehouse. | high | false | |
| Um, I, I think the, the model is fast. | high | false | |
| So yeah it's like faster than the old one. | medium | false | |

---

## 15. Tasks

| # | Task | Depends on | Done when |
|---|---|---|---|
| T0 | Spike: whisper-server, schema order, mic permission | – | Notes written; spec deltas applied; STT models in `models.json` |

> **Status 2026-09-30:** T0–T14 done except the manual checks in `docs/qa/P5.md` (👤). Nav: Talk is ⌘5, Settings moved to ⌘6. The tray's Hibernate/Quit during a talk open `/talk/session?confirm=hibernate|quit`.
| T1 | Migration 0005 + repos + retention | – | Migration/repo/retention tests |
| T2 | SidecarManager refactor + WhisperSidecar | T0 | P2 tests green; whisper lifecycle tests with the fake launcher |
| T3 | Audio capture + Info.plist + permission errors | T0 | Manual: meter moves; the mic indicator is off after stop; 60 s cap; silence notice. Unit: resample length ≈ duration × 16 k |
| T4 | STT client + hallucination filter | T2 | wiremock tests; filter tests |
| T5 | ReplyExtractor | – | §14 tests |
| T6 | Intents + similarity | – | §14 tests |
| T7 | Tutor prompt/schema + golden suite | T0 | Template render test; golden suite passes with the default model (or prompt tuned until it does) |
| T8 | Session engine + commands + channel | T4–T7, T1 | Phase transition tests; typed-mode conversation works end to end |
| T9 | Pronunciation drill | T8 | Drill tests; manual drill |
| T10 | Session review + apply | T8 | Review/fallback/apply tests |
| T11 | Talk session UI + speech queue + PTT + barge-in | T8 | Manual: SPEC §18 P5 items 1–5 |
| T12 | Review + history UI | T10 | Manual: SPEC §18 P5 item 7 |
| T13 | Hibernate/Quit handling + latency diagnostics | T8 | Manual: dialog flows; diagnostics show p50/p90 |
| T14 | QA + perf | all | `docs/qa/P5.md` complete; latency p50/p90 over 20 real turns recorded; no audio files on disk (SPEC §18 P5 item 8) |

Suggested order: T0 → (T1 ∥ T2 ∥ T5 ∥ T6) → (T3 ∥ T4 ∥ T7) → T8 → (T9 ∥ T10 ∥ T11 ∥ T13) → T12 → T14

---

## 16. Manual QA checklist (`docs/qa/P5.md`)

- [ ] First mic use on the bundled app → the macOS prompt with our text; deny → a clear help screen; allow → works
- [ ] Start a session on today's pick → a loading indicator, then the tutor introduces the article and asks a question
- [ ] Hold Space, speak, release → the transcript appears, then the tutor speaks the first sentence in ≤ 3 s (typical)
- [ ] "Yesterday I deploy the application." → correction card + "Please say…" → repeat correctly → "Good. Let's continue." → the topic continues
- [ ] "What does deployment mean?" → a simple explanation + example → the tutor returns to its earlier question
- [ ] "Speak slower please" → the replay is slower, with no delay (no LLM)
- [ ] "How do I say scalability?" → the drill runs with the hint; 3 failed attempts → moves on
- [ ] Press Space while the tutor is speaking → speech stops and recording starts
- [ ] End → review shows grouped suggestions with preselection → Add → the items are in the Word Book with the conversation context
- [ ] Switch to Hibernate during a session → confirm → both sidecars gone; the review is pending in history
- [ ] `find ~/Library/Application\ Support/com.techenglish.app -name "*.wav"` → nothing (default settings)

## 17. Risks

| Risk | Mitigation |
|---|---|
| Latency over budget on M1 | `base.en`; 4B model; prompt caching with a fixed system prompt; sentence streaming; the diagnostics panel shows which stage is slow |
| Whisper "fixes" learner grammar | temperature 0 with no prompt; golden suite built on typed input; accept the limitation and document it |
| Whisper hallucinations on silence | RMS gate + phrase filter (§5) |
| whisper-server has no auth | Binds to 127.0.0.1 on a random port and only runs during sessions. P6 may switch to in-process `whisper-rs` if this becomes a concern. |
| Mic permission attributed to the terminal in dev | Test permission flows on the bundled app only |
| Schema key order not guaranteed | The `@@META@@` fallback (§6.3) |
