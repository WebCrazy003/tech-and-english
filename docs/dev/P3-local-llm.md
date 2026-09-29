# P3 — Local LLM (dev spec)

- **Goal:** add a local LLM (llama.cpp) that loads only when needed. It powers:
  - B1 Summary and Easy English in the Reader
  - Explain in the selection popup
  - an Ask AI panel
  - background filling of "Meaning pending" items
  - an optional AI-written "why interesting"

  The model unloads when idle and never runs in Hibernate.
- **SPEC sections:** §6 (LLM rows), §8.3, §8.4 (P3), §9.3 (auto-fill), §11, §13 (P3), §14 (P3), §16, §18 P3.
- **Branch:** `p3/local-llm` → tag `v0.3.0`

## 0. Scope

**In scope:**
- Model catalog and downloader
- `llama-server` sidecar lifecycle (AiManager)
- `LlmProvider` + `LocalLlamaProvider` + `MockProvider`
- Prompt templates for: `summarize_b1`, `simplify_easy`, `define_term`, `why_interesting`, `ask_about_article`
- Job queue with priorities and streaming to the UI
- Derivatives cache
- Settings › AI page
- AI status indicator
- Model benchmark and choice of the default model

**Out of scope:**
- `tutor_turn` and `session_review` (P4). This phase does write a draft `tutor_turn` schema, used only for the JSON-validity benchmark.
- Whisper (P4)
- Cloud providers

---

## 1. Dependencies

| Where | Add |
|---|---|
| Cargo | `sysinfo = "0.39"` (available memory), `sha2 = "0.11"` (download checks); `reqwest` gains the `stream` feature |
| Dev machine | `brew install llama.cpp` (0.5.0 at the time of writing; provides `llama-server` and `llama-bench`) |

---

## 2. T0 — Spike: confirm the `llama-server` interface (½ day)

Write the results to `docs/dev/notes/P3-llama-server.md` **before** coding T2 and T3.

1. `brew install llama.cpp`, then `llama-server --help > notes`. Confirm these flags:
   - `-m`, `--host`, `--port`, `--api-key`
   - `-c` (context), `-ngl` (GPU layers), `-np` (parallel slots)
   - `--jinja` (use the model's chat template)
   - how to disable the built-in web UI, if a flag for that exists
2. Start it with a small GGUF. Check the following:
   - `GET /health` returns 503 while loading and 200 when ready.
   - `POST /v1/chat/completions` with `stream: true` sends SSE `data:` chunks and ends with `[DONE]`.
   - **JSON-schema constrained output.** Find the exact request field (`response_format: { type: "json_schema", json_schema: { schema } }` or `json_schema`) and confirm invalid output can't happen.
   - `cache_prompt: true` is accepted, and a follow-up turn has a lower time-to-first-token.
   - Closing the HTTP connection stops generation (check the server log).
   - Requests without the API key get `401`.
3. Record the resident memory of `llama-server` with the model loaded (`ps -o rss=`).

If any of these differ from the assumptions below, update this spec first.

---

## 3. Model catalog & downloader

### 3.1 `resources/models.json`

```json
{
  "version": 1,
  "models": [
    {
      "id": "chat-4b-q4km",
      "role": "chat",
      "displayName": "<family> 4B Instruct (Q4_K_M)",
      "file": "<file>.gguf",
      "url": "https://huggingface.co/<repo>/resolve/main/<file>.gguf",
      "sizeBytes": 0,
      "sha256": "<hex>",
      "contextLength": 8192,
      "license": "<name>",
      "licenseUrl": "https://…",
      "recommendedRamGb": 8
    }
  ]
}
```

The real entries are filled in by the T11 benchmark. Until then, the dev build uses any GGUF path set in Settings.

### 3.2 Downloader (`ai/models.rs`)

- Models are stored in `app_data_dir/models/<file>`. While downloading, the file is `<file>.part`.
- **Resume:** if a `.part` file exists, send `Range: bytes=<len>-`. If the server replies `200` instead of `206`, restart from zero.
- The file is streamed to disk. The hash is updated incrementally with `sha2::Sha256`.
- `ai://download {modelId, bytes, total}` is emitted at most 4 times per second.
- **On completion:** if the sha256 doesn't match, delete the file and return an error. Otherwise, rename `.part` to the final name.
- `cancel_download` stops the transfer and keeps the `.part` file.
- **Before starting**, check free disk space ≥ size + 1 GB.
- The license name and link are shown in the UI **before** the download starts. The user must press "Download" after seeing them.

---

## 4. AiManager (`ai/manager.rs`)

### 4.1 States & transitions

```
Unloaded ──ensure_ready()──► Starting ──health 200──► Ready ◄──► Busy
   ▲                            │ timeout 60 s / exit            │
   │                            ▼                                │
   └──────── Stopping ◄── idle timeout / hibernate / quit / unload_now / model change
```

- `ensure_ready()` is idempotent. Concurrent callers await the same start, through a `tokio::sync::watch` channel of state.
- Every state change emits `ai://status { component: "llm", state, modelId?, error? }`.

### 4.2 Launch

```
llama-server -m <models>/<active.gguf> --host 127.0.0.1 --port <free port>
             --api-key <32 random hex> -c <ctx, default 8192> -ngl 99 -np 1 --jinja
```

- `-np 1` means one slot, so requests are served one at a time; the job queue (§6) orders them.
- **Free port:** bind a `TcpListener` to `127.0.0.1:0`, read the port, then drop the listener.
- **Health:** poll `GET /health` every 250 ms, for up to 60 s.
- **Stdout/stderr:** read them into a ring buffer of the last 200 lines. On a startup failure, show that tail (in debug logs only, never with user content).
- **PID:** store it in `app_state.llm_pid`.
- **At app start (orphan check):** if `llm_pid` is set, that process is alive, and its name contains `llama-server` → kill it, then clear the PID.

### 4.3 Binary resolution (dev and bundled share one code path)

The first match wins:

1. `settings.ai.llamaServerPath` (Settings › AI › Advanced)
2. `<current_exe dir>/llama-server` (the bundled sidecar in P5)
3. `which llama-server` (PATH)
4. `/opt/homebrew/bin/llama-server`

If nothing is found, the status is `error: "llama-server not found"`, and Settings › AI shows how to fix it (`brew install llama.cpp`).

### 4.4 Memory guard

- Before launching, read `sysinfo::System::available_memory()`.
- If it is less than `model.sizeBytes + 1.5 GB`, emit `ai://status {state: "needsConfirm", reason: "low_memory", availableGb, neededGb}` and wait for `confirm_ai_start {proceed}`, for up to 2 min.
- A background job never asks; it simply skips.

### 4.5 Stop

- Send SIGTERM, wait up to 5 s, then SIGKILL. Wait for the process to exit, then clear `llm_pid`.
- **Idle timer:** 10 min by default (Settings, 2–60 min). It resets at the end of every job. The timer is paused while `hold()` guards are alive; P4's voice session holds one.
- `ModeManager::on_enter_hibernate` → `AiManager::shutdown()`, which cancels running jobs (drops the streams) and stops the process.
- App quit → `shutdown()` with a 3 s budget.

### 4.6 Testability

- A `trait ProcessLauncher { fn spawn(&self, cmd: LaunchSpec) -> Result<Box<dyn ChildHandle>>; }`.
- `FakeLauncher` starts a wiremock server that imitates `/health` (configurable delay or failure) and `/v1/chat/completions`.

**Tests:**
- Concurrent `ensure_ready` calls → one spawn.
- A health timeout → Error state, then Unloaded.
- The idle timeout stops the process (`tokio::time::pause` + advance).
- A hold guard prevents the idle stop.
- Hibernate cancels an in-flight stream.
- Orphan-PID cleanup is called at startup.

---

## 5. LlmProvider (`ai/provider.rs`, `ai/llama.rs`)

```rust
pub struct LlmRequest {
    pub messages: Vec<Msg>,                  // role: system | user | assistant
    pub max_tokens: u32,
    pub temperature: f32,
    pub json_schema: Option<serde_json::Value>,
    pub cache_prompt: bool,
}
pub enum LlmChunk { Delta(String), Done { prompt_tokens: u32, completion_tokens: u32 } }

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: LlmRequest) -> Result<String, AppError>;
    async fn stream(&self, req: LlmRequest) -> Result<BoxStream<'static, Result<LlmChunk, AppError>>, AppError>;
    fn model_id(&self) -> String;
}
```

- `LocalLlamaProvider` calls `AiManager::ensure_ready()`, then posts to `/v1/chat/completions` with a `Bearer` API key.
- The SSE parser is line-based:
  - Handles `data:` lines.
  - Ignores comment lines and empty keep-alive lines.
  - Ends on `[DONE]`.
- **JSON tasks:** after completion, run `serde_json::from_str` and validate against the Rust struct. If that fails, retry once with an extra user message: "Return only valid JSON that matches the schema." A second failure → `AppError::Ai("invalid JSON")`.
- `MockProvider` takes scripted responses or chunk sequences.

---

## 6. Job queue (`ai/jobs.rs`)

- There are two priorities: **Interactive** (Reader, Explain, Ask AI; P4 adds the tutor) and **Background** (auto-fill, why text).
- There is one worker. Interactive jobs always run first.
- A background job runs only when **all** of these are true:
  - the model state is `Ready` (**background jobs never trigger a load**)
  - no interactive job is waiting
  - the mode is Standard
- **Cancellation:** every job has a `CancellationToken`. When the UI closes the view (the Channel is dropped, or `cancel_job {jobId}` is called), the stream is dropped.
- **Streaming to the UI** uses a Tauri `ipc::Channel<StreamEvent>`:

```ts
type StreamEvent =
  | { kind: "queued"; position: number }
  | { kind: "loading" }                  // model is starting
  | { kind: "delta"; text: string }
  | { kind: "done"; cached: boolean; modelId: string }
  | { kind: "error"; code: string; message: string };
```

---

## 7. Prompt templates (`src-tauri/src/ai/prompts/*.md`)

- Each file starts with a front-matter line such as `version: 1`, followed by `### system` and `### user` sections.
- Placeholders look like `{{name}}`. A missing placeholder is an error, and a test renders every template with sample data.
- `prompt_version` is stored with each derivative. Bumping the version invalidates cached outputs lazily: a mismatched version is treated as a cache miss.

**Shared level block** (`{{level_rules}}`), produced from the `englishLevel` setting (default Level 2):

| Level | Rules text |
|---|---|
| 1 | "Use very short sentences (max 10 words). Use only very common words. Explain every technical term in simple words." |
| 2 | "Use clear, simple sentences (CEFR B1). Keep important technical terms, but explain each one simply the first time you use it." |
| 3 | "Use natural English. Explain only rare technical terms." |

### 7.1 `summarize_b1.md` (streamed, plain Markdown)

```
### system
You help a technology professional learn English. Their English level is B1.
{{level_rules}}
Only use information from the article. Do not add facts. Do not use hype words.

### user
Article title: {{title}}
Source: {{source}}

Article:
"""
{{body}}
"""

Write a summary of at most 250 words:
1. One sentence: what is this article about?
2. Two or three short paragraphs with the main points.
3. A section "**Key words**" with 3–5 technical words from the article, one per line, as: word — simple meaning
```

### 7.2 `simplify_easy.md` (streamed, plain text)

The rules: sentences of at most 12 words; one idea per sentence; the most common ~2,000 English words plus technical terms, each explained in brackets the first time; keep the article's order; at most 400 words; no bullet lists.

### 7.3 `define_term.md` (JSON)

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

### 7.4 `why_interesting.md` (plain text, max 40 words)

Input: title, description, matched topics, HN points.

The prompt: "Tell the reader in 1–2 short sentences why this story may interest them. Address them as 'you'. Mention their topics. No hype."

### 7.5 `ask_about_article.md` (streamed chat)

- **System:** level rules; the article's title plus its B1 summary (if cached) or the first 3,000 chars of the body.
- Answers are limited to 120 words.
- If the question is about an English word, the answer includes a simple meaning and one example.
- **The history is kept in memory only** (the last 8 messages). It is not persisted in P3.

### 7.6 Body truncation

- The token budget for the body is `ctx − 1500` (for the prompt and output), estimated as chars / 4.
- If the body is too long, keep the first 60 % of the budget from the start of the article. Fill the remaining 40 % with later paragraphs that contain the article's matched topic keywords, in their original order.

---

## 8. Features

### 8.1 Reader tabs

- **B1 Summary** and **Easy English** are enabled.
- On selecting a tab: `get_derivative {articleId, kind, channel}`.
  - On a cache hit → a single `delta` with the full text, then `done {cached: true}`.
  - On a miss → the stream is shown with a typing cursor. It is saved to `article_derivatives` **only when complete**.
- **Regenerate** (in a ⋯ menu) deletes the cached row and streams again.
- If `body_status ≠ ok`, the summary is generated from title + description, with the note "Based on the short description only".
- The selection popup works on the summary text too. The context sentence is taken from the summary, and `article_id` is still set.
- **Listen:** if a B1 summary is cached, speak it; otherwise speak the title + description (as in P2). It never triggers generation.

### 8.2 Explain (popup)

- The **[Explain]** button calls `define_term {term, sentence, articleId}` → `DefineTermOut`.
- The popup shows: meaning_simple (large), meaning_b1, part of speech, `syllables` (hint), examples, collocations.
- **[Add to Word Book]** saves every field, plus the context.
- Results are cached in memory for the session, keyed by `(term_key, sentence)`.

### 8.3 Ask AI

- A panel in the Reader (right side, collapsible).
- Input box with quick chips: "Explain this simply", "What does ___ mean?", "Why is this important?".
- The answer streams in. Text in answers can be selected, which opens the popup.

### 8.4 Background jobs

- **Auto-fill pending meanings:** when the model is `Ready` and idle ≥ 5 s, take up to 10 items with no meaning (oldest first). For each, run `define_term` with its latest context and fill the empty fields only. Emit `vocab://changed`.
- **AI "why":** when a pick is displayed, the model is `Ready`, and `settings.ai.llmWhy` is true (default true) → generate it, store it in `daily_picks.why` with `why_source='llm'`, and emit `pick://changed`.

### 8.5 Settings › AI

- **Models:** a list from the catalog, showing size, license (link), and a state of not downloaded / downloading (progress, cancel) / downloaded (delete) / active (radio).
- **Status:** the current state, the model, memory used (RSS of the process), and an [Unload now] button.
- **Options:** idle timeout, context size (4096 / 8192), English level (shared with the tutor), "AI-written reasons for the daily pick".
- **Advanced:** the llama-server path (detected path + override + [Test]) and a custom GGUF path (for development).
- **First-use flow:** if no model is downloaded and the user clicks an AI feature → a dialog that recommends the default model, with its size and license, and [Download].

### 8.6 AI status indicator

- A small dot in the widget header and the main window's top bar:
  - grey = unloaded
  - pulsing amber = loading
  - green = ready
  - blue = busy
  - red = error
- Its tooltip shows the state and the model.
- In Hibernate the dot is hidden and AI buttons show "Switch to Standard" (`AppError::Hibernating`).

---

## 9. Commands & events (P3)

| Command | Args | Returns |
|---|---|---|
| `ai_status` | – | `{ state, modelId?, rssMb?, error? }` |
| `list_models` | – | `ModelEntry[]` (+ `downloaded`, `active`, `partialBytes`) |
| `download_model` / `cancel_download` / `delete_model` | `{ modelId }` | `void` |
| `set_active_model` | `{ modelId }` | `void` (restarts the sidecar if it's running) |
| `confirm_ai_start` | `{ proceed }` | `void` |
| `unload_ai` | – | `void` |
| `get_derivative` | `{ articleId, kind: "summary_b1" \| "easy_english", regenerate?, channel }` | `{ jobId }` |
| `define_term` | `{ term, sentence, articleId? }` | `DefineTermOut` |
| `ask_about_article` | `{ articleId, messages, channel }` | `{ jobId }` |
| `cancel_job` | `{ jobId }` | `void` |

Events: `ai://status`, `ai://download`.

**Migration `0003_ai.sql`:** `article_derivatives` (SPEC §13 P3), plus `app_state` keys (no DDL).

---

## 10. T11 — Model benchmark & default choice (1 day)

1. **Candidates.** Take the newest instruct releases available at spike time from well-known open-weight families (for example Qwen, Gemma, Llama, Phi, Mistral), in the **3–4B** and **7–8B** sizes, as Q4_K_M GGUF from a reputable quantizer. Pick 2–3 per size. The license must allow local use and redistribution of the app without the weights (the weights are downloaded by the user).
2. **Speed:** run `llama-bench -m <gguf> -p 512 -n 128 -ngl 99`. Record prompt-processing t/s and generation t/s.
3. **Time to first token:** a script sends one 1,500-token prompt, then a follow-up turn with `cache_prompt`. Record TTFT for both.
4. **JSON validity:** `cargo test --release -- --ignored bench_json` runs 50 `define_term` prompts (terms from the fixture list) and 30 draft `tutor_turn` prompts. It reports the valid percentage **without** the retry.
5. **Quality:** 10 B1 summaries of fixture articles are rated by hand from 1 to 5 on accuracy, simplicity and keeping the key terms.
6. **Choose:**
   - **Default chat model** = the best average quality that meets gen ≥ 20 t/s, follow-up TTFT ≤ 1.5 s and JSON ≥ 98 %.
   - **Optional "quality" model** = the best 7–8B model that meets gen ≥ 10 t/s.
7. Fill `models.json` (URL, size, sha256, license) and write the results table to `docs/perf.md`.

---

## 11. Tasks

| # | Task | Depends on | Done when |
|---|---|---|---|
| T0 | llama-server spike | – | Notes file written; any spec deltas applied |
| T1 | Catalog + downloader + Settings › AI (models section) | T0 | wiremock tests: resume via 206, 200 fallback, sha mismatch deletes, cancel keeps `.part`; manual download works |
| T2 | AiManager (launcher, lifecycle, binary resolution, memory guard, orphan cleanup) | T0 | §4.6 tests |
| T3 | LlmProvider (SSE, JSON validation + retry) + MockProvider | T2 | wiremock SSE tests (chunk splits across TCP reads, `[DONE]`, error mid-stream); JSON retry test |
| T11 | Benchmark & default model | T3 | §10 complete; `models.json` filled |
| T4 | Prompt system + templates | T3 | Render test for every template; truncation tests |
| T5 | Job queue + Channel streaming + cancel | T3 | Priority test (background waits behind interactive); background never triggers a load; cancel stops the stream |
| T6 | Reader tabs + derivatives cache + Listen summary | T4, T5 | Cache hit/miss/regenerate tests (MockProvider); manual SPEC §18 P3 item 2 |
| T7 | Explain in popup | T4, T5 | Manual: SPEC §18 P3 item 5 |
| T8 | Ask AI panel | T5 | Manual: streamed answers; history limit |
| T9 | Background jobs | T5 | Tests: auto-fill fills only empty fields; skipped when unloaded; why text stored with `why_source=llm` |
| T10 | Hibernate + status UI + first-use dialog | T2 | Manual: SPEC §18 P3 items 3, 4 |
| T12 | QA + perf | all | `docs/qa/P3.md` complete; perf rows for LLM load time, RSS, summary timing |

Suggested order: T0 → (T1 ∥ T2) → T3 → T11 → (T4 ∥ T5) → (T6 ∥ T7 ∥ T8 ∥ T9 ∥ T10) → T12

---

## 12. Manual QA checklist (`docs/qa/P3.md`)

- [ ] No model downloaded → clicking B1 Summary shows the first-use dialog with size and license
- [ ] Download → pause (cancel) → resume continues from the partial size → sha verified
- [ ] B1 Summary streams; switching tabs and back shows the cached text instantly
- [ ] Explain on a technical word → meaningful JSON fields; Add saves everything; the Word Book detail shows them
- [ ] A pending-meaning item gets filled while the model is loaded and idle
- [ ] After 10 min idle: `pgrep llama-server` is empty; memory released
- [ ] Hibernate while a summary is streaming → the stream stops with a clear message; the process is gone
- [ ] Force-quit the app while the model is loaded → relaunch → no orphan `llama-server` (the old one was killed)
- [ ] Low-memory warning appears when simulated (setting override `ai.debugFakeAvailableGb`)

## 13. Risks

| Risk | Mitigation |
|---|---|
| llama-server API/flag changes | T0 spike; the flags live in one `LaunchSpec` builder |
| Small models produce weak summaries | T11 quality gate; optional 7–8B "quality" model for summaries (Settings: "Use quality model for summaries") |
| JSON errors | Schema-constrained decoding + one retry + a typed error |
| Memory pressure with other apps open | Memory guard; idle unload; the 4B default |
