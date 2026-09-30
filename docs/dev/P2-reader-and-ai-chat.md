# P2 — Reader & AI chat (dev spec)

> **Status (2026-09-29):** built. See `docs/qa/P2.md` for verification, deviations, and remaining manual checks.

- **Goal:** read stories inside the app, with a local AI chat panel on the right. The panel can summarize the story, list its key words, explain it simply, and answer questions about it. The model loads only when it is used, unloads when idle, and never runs in Hibernate.
- **SPEC sections:** §8.1–8.4, §11, §6 (LLM rows), §13 (P2), §14 (P2), §16, §18 P2.
- **Branch:** `p2/reader-ai-chat` → tag `v0.2.0`
- **Built from:** the reader half of the old P2 spec plus the old P3 (LLM) spec (SPEC C17). Words, Word Book and quiz moved to [P4](P4-words.md).

## 0. Scope

**In scope:**
- Article body fetch + Readability extraction + sanitizing; paywall detection; difficulty and reading time
- Pick integration (paywall retry; difficulty on the card)
- Reader page: Original / B1 Summary / Easy English tabs; read tracking; **Ask AI about this** on selected text
- Local LLM: model catalog + downloader, `llama-server` sidecar lifecycle, `LlmProvider`, job queue, prompts
- **AI chat panel** (SPEC §8.4): quick actions, free questions, streamed answers, history saved per article
- Settings › AI; AI status indicator; optional AI "why" text for the pick
- Model benchmark and choice of the default model

**Out of scope:**
- Word popup, dictionary, TTS, Word Book, quiz → P4
- `define_term` → P4
- Learning / lesson → P3
- Voice → P5

**Changes to the P1 UI:**
- **Read** (widget, Today, Explore) opens `/reader/:id` in the main window. **Open in browser** moves to the ⋯ menu and to the Reader header.

---

## 1. Dependencies

| Where | Add |
|---|---|
| npm | `@mozilla/readability@^0.6.0`, `dompurify@^3.4.16`; dev: `jsdom` (Vitest DOM tests) |
| Cargo | `sysinfo = "0.39"` (available memory), `sha2 = "0.11"` (download checks), `tokio-util` (`CancellationToken`); `reqwest` gains the `stream` feature |
| Resources | `src-tauri/resources/ngsl.txt`: **New General Service List 1.2** (lemmatized forms). License **CC BY-SA 4.0**. The attribution goes in About and in `docs/licenses.md`. Used for difficulty (§5). |
| Dev machine | `brew install llama.cpp` (provides `llama-server` and `llama-bench`) |

> **Why NGSL.** SPEC §8.2 needs a list of common English words with a license that allows bundling. NGSL is built for learners and has about the right size (~2,800 words). **Fallback** if the lemmatized file isn't available: use the headword list plus a suffix-stripping lemmatizer.

---

## 2. T0 — Spike: confirm the `llama-server` interface (½ day)

Write the results to `docs/dev/notes/P2-llama-server.md` **before** coding T8 and T9.

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
- If `ok`: compute difficulty (§4) and store `canonical_url`. *(Deferred: merging with another article that has the same canonical URL, because picks, chats and interactions would have to move. See `docs/qa/P2.md`.)*
- Wake the pick waiters (§5).
- Emit `article://body {articleId, status}`.

---

## 4. Difficulty — `news/difficulty.rs` (pure)

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

## 5. Pick integration (`news/pick.rs` `post_select`)

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

- **Top:** title, source, published date, `Difficulty · N min read`, topic chips, and buttons **[Open in browser] [Save] [Not interested]**.
- **Mode tabs:** **Original**, **B1 Summary**, **Easy English** (§14.1).
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

### 6.4 Selected text → Ask AI

When the user selects text in the article body (up to 1,000 chars), a small chip **Ask AI about this** appears near the selection. Clicking it opens the AI panel (if collapsed) and puts this into the chat input: `About this part: "…quote…" — ` with the cursor at the end. The word popup and dictionary come in P4.

---

## 7. Model catalog & downloader

### 7.1 `resources/models.json`

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

The real entries are filled in by the T-bench benchmark (§16). Until then, the dev build uses any GGUF path set in Settings.

### 7.2 Downloader (`ai/models.rs`)

- Models are stored in `app_data_dir/models/<file>`. While downloading, the file is `<file>.part`.
- **Resume:** if a `.part` file exists, send `Range: bytes=<len>-`. If the server replies `200` instead of `206`, restart from zero.
- The file is streamed to disk. The hash is updated incrementally with `sha2::Sha256`.
- `ai://download {modelId, bytes, total}` is emitted at most 4 times per second.
- **On completion:** if the sha256 doesn't match, delete the file and return an error. Otherwise, rename `.part` to the final name.
- `cancel_download` stops the transfer and keeps the `.part` file.
- **Before starting**, check free disk space ≥ size + 1 GB.
- The license name and link are shown in the UI **before** the download starts. The user must press "Download" after seeing them.

---

## 8. AiManager (`ai/manager.rs`)

### 8.1 States & transitions

```
Unloaded ──ensure_ready()──► Starting ──health 200──► Ready ◄──► Busy
   ▲                            │ timeout 60 s / exit            │
   │                            ▼                                │
   └──────── Stopping ◄── idle timeout / hibernate / quit / unload_now / model change
```

- `ensure_ready()` is idempotent. Concurrent callers await the same start, through a `tokio::sync::watch` channel of state.
- Every state change emits `ai://status { component: "llm", state, modelId?, error? }`.

### 8.2 Launch

```
llama-server -m <models>/<active.gguf> --host 127.0.0.1 --port <free port>
             --api-key <32 random hex> -c <ctx, default 8192> -ngl 99 -np 1 --jinja
```

- `-np 1` means one slot, so requests are served one at a time; the job queue (§10) orders them.
- **Free port:** bind a `TcpListener` to `127.0.0.1:0`, read the port, then drop the listener.
- **Health:** poll `GET /health` every 250 ms, for up to 60 s.
- **Stdout/stderr:** read them into a ring buffer of the last 200 lines. On a startup failure, show that tail (in debug logs only, never with user content).
- **PID:** store it in `app_state.llm_pid`.
- **At app start (orphan check):** if `llm_pid` is set, that process is alive, and its name contains `llama-server` → kill it, then clear the PID.

### 8.3 Binary resolution (dev and bundled share one code path)

The first match wins:

1. `settings.ai.llamaServerPath` (Settings › AI › Advanced)
2. `<current_exe dir>/llama-server` (the bundled sidecar in P6)
3. `<app data>/bin/*/llama-server` (the official nightly build unpacked there; newest folder first; added after the spike, because Homebrew has no bottle here)
4. `which llama-server` (PATH)
5. `/opt/homebrew/bin/llama-server`

If nothing is found, the status is `error: "llama-server not found"`, and Settings › AI shows how to fix it (`brew install llama.cpp`).

### 8.4 Memory guard

- Before launching, read `sysinfo::System::available_memory()`.
- If it is less than `model.sizeBytes + 1.5 GB`, emit `ai://status {state: "needsConfirm", reason: "low_memory", availableGb, neededGb}` and wait for `confirm_ai_start {proceed}`, for up to 2 min.
- A background job never asks; it simply skips.

### 8.5 Stop

- Send SIGTERM, wait up to 5 s, then SIGKILL. Wait for the process to exit, then clear `llm_pid`.
- **Idle timer:** 10 min by default (Settings, 2–60 min). It resets at the end of every job. The timer is paused while `hold()` guards are alive; P5's voice session holds one.
- `ModeManager::on_enter_hibernate` → `AiManager::shutdown()`, which cancels running jobs (drops the streams) and stops the process.
- App quit → `shutdown()` with a 3 s budget.

### 8.6 Testability

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

## 9. LlmProvider (`ai/provider.rs`, `ai/llama.rs`)

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

## 10. Job queue (`ai/jobs.rs`)

- There are two priorities: **Interactive** (Reader tabs, chat panel; P5 adds the tutor) and **Background** (AI "why" text; P4 adds meaning auto-fill).
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

## 11. Prompt templates (`src-tauri/src/ai/prompts/*.md`)

- Each file starts with a front-matter line such as `version: 1`, followed by `### system` and `### user` sections.
- Placeholders look like `{{name}}`. A missing placeholder is an error, and a test renders every template with sample data.
- `prompt_version` is stored with each derivative. Bumping the version invalidates cached outputs lazily: a mismatched version is treated as a cache miss.

**Shared level block** (`{{level_rules}}`), produced from the `englishLevel` setting (default Level 2):

| Level | Rules text |
|---|---|
| 1 | "Use very short sentences (max 10 words). Use only very common words. Explain every technical term in simple words." |
| 2 | "Use clear, simple sentences (CEFR B1). Keep important technical terms, but explain each one simply the first time you use it." |
| 3 | "Use natural English. Explain only rare technical terms." |

### 11.1 `summarize_b1.md` (streamed, plain Markdown)

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

### 11.2 `simplify_easy.md` (streamed, plain text)

The rules: sentences of at most 12 words; one idea per sentence; the most common ~2,000 English words plus technical terms, each explained in brackets the first time; keep the article's order; at most 400 words; no bullet lists.

### 11.3 `why_interesting.md` (plain text, max 40 words)

Input: title, description, matched topics, HN points.

The prompt: "Tell the reader in 1–2 short sentences why this story may interest them. Address them as 'you'. Mention their topics. No hype."

### 11.4 `article_chat.md` (streamed chat)

```
### system
You help a technology professional understand an article and learn English at the same time.
{{level_rules}}
Answer from the article below. If the article does not contain the answer, say so, then give a short
general answer and mark it with "(not from the article)".
Keep answers under 120 words unless the user asks for more. Use short paragraphs. No tables.

Article: {{title}} ({{source}})
Summary:
"""
{{summary_b1}}
"""
Relevant parts:
"""
{{relevant_paragraphs}}
"""
```

- **History:** the last ≤ 8 messages from `article_chats` are sent as chat turns.
- **Quick actions** are sent as user messages with fixed text, so they appear naturally in the history:

| Action | Message sent | Notes |
|---|---|---|
| Summarize | "Summarize this article." | Answered from the cached `summary_b1` without calling the model again, if it exists; otherwise the summary is generated (and cached), then posted |
| Key words | "List 5 important technical words from this article. For each: word — simple meaning." | |
| Explain simply | "Explain the main idea of this article in very simple English." | |

- **`relevant_paragraphs`:** the body is split into paragraphs, and paragraphs are scored by word overlap with the user's question (lowercase, stop words removed). Take the best ones until the budget (§11.5) is used; always include the first paragraph. With no question (quick actions), use the first paragraphs.

### 11.5 Body truncation

- The token budget for the body is `ctx − 1500` (for the prompt and output), estimated as chars / 4.
- If the body is too long, keep the first 60 % of the budget from the start of the article. Fill the remaining 40 % with later paragraphs that contain the article's matched topic keywords, in their original order.

---

---

## 12. AI chat panel (frontend)

`src/features/reader/AiPanel.tsx`, right side of `/reader/:id`, about 360 px wide, collapsible (the state is remembered in settings `reader.aiPanelOpen`).

**Layout:**
- **Header:** "AI" + status dot (§14.4) + ⋯ (Clear chat).
- **Quick-action chips:** Summarize · Key words · Explain simply.
- **Message list:** user messages on the right, AI messages on the left. Markdown rendering is limited to paragraphs, **bold**, lists and `code`. Streaming shows a typing cursor. **Stop** cancels a running answer (`cancel_job`).
- **Input:** multi-line; Enter sends, Shift+Enter adds a new line. Disabled while an answer is streaming.

**States** (SPEC §8.4):

| State | Panel shows |
|---|---|
| No model downloaded | Setup card: recommended model, size, license link, [Download] with progress |
| Model loading | "Starting AI… (about 10 s)" |
| Hibernate | "AI is off in Hibernate mode." [Switch to Standard] |
| Low memory (needsConfirm) | "Your Mac has little free memory. Start AI anyway?" [Start] [Cancel] |
| Error | Message + [Retry] |

**Persistence:**
- `list_article_chat {articleId}` loads the history when the Reader opens.
- User messages are saved immediately. AI messages are saved when complete; a partial answer that was stopped is saved with " …(stopped)".

**Tests** (Vitest + mocked API):
- Enter / Shift+Enter behaviour
- quick action sends the fixed text
- a stream appends deltas
- Stop calls `cancel_job`
- the Hibernate state shows the switch button

---

## 13. Database — `migrations/0002_reader_ai.sql`

```sql
CREATE TABLE article_derivatives (          -- SPEC §13 (P2)
  article_id INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('summary_b1','easy_english','why_interesting')),
  model_id TEXT NOT NULL,
  prompt_version TEXT NOT NULL,
  text TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (article_id, kind)
);
CREATE TABLE article_chats (
  id INTEGER PRIMARY KEY,
  article_id INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('user','assistant')),
  content TEXT NOT NULL,
  model_id TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_article_chats_article ON article_chats(article_id, id);
```

**Retention:** articles that have `article_chats` rows are never deleted (the P1 retention query gains a `NOT EXISTS` check). The body columns already exist from 0001.

---

## 14. Features & UI

### 14.1 Reader tabs

- **B1 Summary** and **Easy English** are enabled.
- On selecting a tab: `get_derivative {articleId, kind, channel}`.
  - On a cache hit → a single `delta` with the full text, then `done {cached: true}`.
  - On a miss → the stream is shown with a typing cursor. It is saved to `article_derivatives` **only when complete**.
- **Regenerate** (in a ⋯ menu) deletes the cached row and streams again.
- If `body_status ≠ ok`, the summary is generated from title + description, with the note "Based on the short description only".
- **Ask AI about this** (§6.4) also works on the summary text.

### 14.2 AI "why" text

When a pick is displayed, the model is already `Ready`, and `settings.ai.llmWhy` is true (default true), run `why_interesting` (§11.3). Store it in `daily_picks.why` with `why_source='llm'`, and emit `pick://changed`. **This never triggers a model load.**

### 14.3 Settings › AI

- **Models:** a list from the catalog, showing size, license (link), and a state of not downloaded / downloading (progress, cancel) / downloaded (delete) / active (radio).
- **Status:** the current state, the model, memory used (RSS of the process), and an [Unload now] button.
- **Options:** idle timeout, context size (4096 / 8192), English level (shared with the tutor), "AI-written reasons for the daily pick".
- **Advanced:** the llama-server path (detected path + override + [Test]) and a custom GGUF path (for development).
- **First-use flow:** if no model is downloaded, the AI panel shows the setup card (§12). Reader tabs B1/Easy show the same card.

### 14.4 AI status indicator

- A small dot in the widget header and the main window's top bar:
  - grey = unloaded
  - pulsing amber = loading
  - green = ready
  - blue = busy
  - red = error
- Its tooltip shows the state and the model.
- In Hibernate the dot is hidden and AI buttons show "Switch to Standard" (`AppError::Hibernating`).

---

---

## 15. Commands & events (P2)

| Command | Args | Returns |
|---|---|---|
| `fetch_article_html` | `{ articleId }` | `{ html, finalUrl, canonicalUrl?, paywallHint }` |
| `save_article_body` | `{ articleId, text, html, canonicalUrl?, paywallHint, failed? }` | `ArticleListItem` (merged id if canonical dup) |
| `get_reader_article` | `{ id }` | `ArticleListItem` + `bodyHtml`, `bodyStatus`, `difficulty`, `readingMinutes`, `wordCount` |
| `ai_status` | – | `{ state, modelId?, rssMb?, error? }` |
| `list_models` | – | `ModelEntry[]` (+ `downloaded`, `active`, `partialBytes`) |
| `download_model` / `cancel_download` / `delete_model` | `{ modelId }` | `void` |
| `set_active_model` | `{ modelId }` | `void` (restarts the sidecar if it's running) |
| `confirm_ai_start` | `{ proceed }` | `void` |
| `unload_ai` | – | `void` |
| `get_derivative` | `{ articleId, kind: "summary_b1" \| "easy_english", regenerate?, channel }` | `{ jobId }` |
| `list_article_chat` | `{ articleId }` | `ChatMessage[]` |
| `send_article_chat` | `{ articleId, text?, action?: "summarize" \| "key_words" \| "explain_simply", channel }` | `{ jobId, userMessage }` |
| `clear_article_chat` | `{ articleId }` | `void` |
| `cancel_job` | `{ jobId }` | `void` |

Events:
- `ai://status`
- `ai://download`
- `article://body {articleId, status}`
- `article://needs-body {articleId}`

---

## 16. T-bench — Model benchmark & default choice (1 day)

1. **Candidates.** Take the newest instruct releases available at spike time from well-known open-weight families (for example Qwen, Gemma, Llama, Phi, Mistral), in the **3–4B** and **7–8B** sizes, as Q4_K_M GGUF from a reputable quantizer. Pick 2–3 per size. The license must allow local use and redistribution of the app without the weights (the weights are downloaded by the user).
2. **Speed:** run `llama-bench -m <gguf> -p 512 -n 128 -ngl 99`. Record prompt-processing t/s and generation t/s.
3. **Time to first token:** a script sends one 1,500-token prompt, then a follow-up turn with `cache_prompt`. Record TTFT for both.
4. **JSON validity:** `cargo test --release -- --ignored bench_json` runs 30 draft `article_chat` prompts. It reports the valid percentage **without** the retry.
5. **Quality:** 10 B1 summaries of fixture articles are rated by hand from 1 to 5 on accuracy, simplicity and keeping the key terms.
6. **Choose:**
   - **Default chat model** = the best average quality that meets gen ≥ 20 t/s, follow-up TTFT ≤ 1.5 s and JSON ≥ 98 %.
   - **Optional "quality" model** = the best 7–8B model that meets gen ≥ 10 t/s.
7. Fill `models.json` (URL, size, sha256, license) and write the results table to `docs/perf.md`.

---

---

## 17. Tasks

| # | Task | Depends on | Done when |
|---|---|---|---|
| T0 | llama-server spike (§2) | – | Notes file written; spec deltas applied |
| T1 | Migration 0002 + repos (derivatives, chats) + retention | – | Migration/repo tests; articles with chats are protected |
| T2 | `fetch_article_html`, `save_article_body`, paywall, canonical merge (§3) | T1 | wiremock tests: HTML ok, non-HTML rejected, 3 MB cap, redirect → finalUrl; status rules table test |
| T3 | `difficulty.rs` + NGSL + license file (§4) | – | §4 tests |
| T4 | `lib/extract.ts` (Readability + DOMPurify) | T2 | Vitest (jsdom) on 3 fixture pages |
| T5 | Pick integration (§5) | T2–T4 | §5 tests; widget shows difficulty + reading time |
| T6 | Reader page + CSP + read tracking + Ask-AI chip (§6) | T4 | Manual: SPEC §18 P2 items 1, 2, 6 |
| T7 | Catalog + downloader + Settings › AI models (§7) | T0 | wiremock tests: resume via 206, 200 fallback, sha mismatch deletes, cancel keeps `.part` |
| T8 | AiManager (§8) | T0 | §8.6 tests |
| T9 | LlmProvider (§9) + MockProvider | T8 | SSE tests (split chunks, `[DONE]`, error mid-stream); JSON retry test |
| T-bench | Benchmark & default model (§16) | T9 | `models.json` filled; `docs/perf.md` updated |
| T10 | Prompts + truncation + relevant paragraphs (§11) | T9 | Render test per template; truncation + paragraph-scoring tests |
| T11 | Job queue + Channel streaming + cancel (§10) | T9 | Priority test; background never loads the model; cancel stops the stream |
| T12 | Reader tabs + derivatives cache (§14.1) | T10, T11 | Cache hit/miss/regenerate tests (MockProvider) |
| T13 | Chat backend: `send_article_chat` (+ quick actions, summary reuse), history (§11.4, §15) | T10, T11 | Tests: history limit 8; Summarize reuses the cached summary without an LLM call; stopped answer saved with marker |
| T14 | AI panel UI (§12) | T13 | Vitest tests (§12); manual SPEC §18 P2 items 4, 5 |
| T15 | Hibernate + status indicator + AI "why" (§14.2–14.4) | T8 | Manual SPEC §18 P2 items 7, 8 |
| T16 | QA + perf | all | `docs/qa/P2.md`; perf rows: model load time, RSS with model, first-token time, summary time, reader-open time |

Suggested order: T0 → (T1 ∥ T3 ∥ T7 ∥ T8) → (T2 ∥ T9) → (T4 ∥ T-bench ∥ T10 ∥ T11) → (T5 ∥ T6 ∥ T12 ∥ T13) → (T14 ∥ T15) → T16

---

## 18. Manual QA checklist (`docs/qa/P2.md`)

- [ ] Read from the widget opens the story in the app; images load; links open in the browser; no scripts run
- [ ] A paywalled story shows the fallback; it is never the daily pick if an alternative exists
- [ ] No model downloaded → the AI panel shows the setup card; the download can be cancelled and resumed; sha verified
- [ ] Summarize streams; asking again (or opening the B1 tab) is instant from the cache
- [ ] Ask "What is the main tool in this article?" → the answer is based on the article; a question the article can't answer is marked "(not from the article)"
- [ ] Close and reopen the story → the chat history is still there; Clear chat empties it
- [ ] Select a paragraph → Ask AI about this → the quote appears in the input
- [ ] Stop during streaming → the answer stops and is saved with "…(stopped)"
- [ ] After 10 min idle, `pgrep llama-server` is empty; memory released
- [ ] Hibernate during streaming → the stream stops with a clear message; the process is gone
- [ ] Force-quit with the model loaded → relaunch → no orphan `llama-server`

---

## 19. Risks

| Risk | Mitigation |
|---|---|
| Readability fails on some sites | `failed` status + Open in browser; the chat still works from the title + description |
| llama-server API or flags change | T0 spike; flags in one `LaunchSpec` builder |
| Small models give weak summaries/answers | Benchmark quality gate; optional 7–8B "quality" model |
| The model invents facts not in the article | The prompt requires "(not from the article)" marking; relevant paragraphs are included; this is tested in the QA checklist |
| Memory pressure | Memory guard; idle unload; 4B default |
