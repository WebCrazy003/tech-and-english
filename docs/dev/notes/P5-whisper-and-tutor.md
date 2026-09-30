# P5 T0 — whisper-server + tutor schema spike (2026-09-30)

Machine: Mac mini M1, 16 GB, macOS 14.7.

## whisper-server

**Install:** `brew install whisper.cpp` → **1.9.4**. Homebrew has no bottle for macOS 14, so it builds from source (it also built `llama.cpp` 0.5.0 and `ggml` 0.25.3 as dependencies, which needs `cmake`).
- It must be run from outside `~/Documents`: Homebrew's build sandbox cannot read the working directory there ("getcwd: Operation not permitted"). From `~` it works.
- The first start compiles the Metal shaders (about 30 s, once). After that they are cached and loading takes 0.015 s.

**Binaries:** `/opt/homebrew/bin/whisper-server`, `whisper-cli` (plus `whisper-bench`, `whisper-stream`, …) ✅

**Flags (all present):** `-m`, `--host`, `--port`, `-t` (default 4), `-l` (default `en`) ✅. Also useful: `-nf` (no temperature fallback), `-sns` (suppress non-speech tokens), `--prompt`, `--inference-path`, `-nth` (no-speech threshold, default 0.6).

**`/inference` (multipart):** `file`, `temperature`, `response_format=json` ✅ → `{"text": " Yesterday I deployed the application.\n"}`. The text has a leading space, and a `\n` after each segment (long clips have several lines). Without `file` → HTTP 400.

**Ready:** the server loads the model **before** it listens. While loading, connections are refused (like llama-server). `GET /health` → `{"status":"ok"}`. Time to ready: **0.25 s** (base.en), **0.63 s** (small.en).

**Authentication:** none. An `Authorization` header is ignored. → See Risks: bind to 127.0.0.1, random port, only during sessions.

**Silence:** 2 s of digital silence → `" [BLANK_AUDIO]\n"`. The bracket filter (§5) removes it.

### Speed and memory (`-t 4`, Metal)

Clips were made with `say -r 120–140 --data-format=LEI16@16000`.

| Clip | base.en | small.en |
|---|---|---|
| 2.9 s | 0.13 s (0.68 s the very first request) | 0.38 s |
| 6.4 s | 0.19 s | 0.56 s |
| 17.8 s | 0.36 s | 1.0–1.1 s |
| 2 s silence | 0.14 s | 0.39 s |
| RSS | ≈ 330–350 MB | ≈ 350 MB (weights are memory-mapped) |

→ **base.en stays the default.** STT is a small part of the latency budget.

### Does whisper "fix" the learner's grammar?

Ten sentences with typical mistakes, spoken by the macOS `say` voice, base.en, temperature 0, no prompt:

| Said | Heard |
|---|---|
| Yesterday I deploy the application. | Yesterday I deploy**ed** the application. ❌ |
| He don't use Docker at work. | ✅ verbatim |
| The model run on my laptop. | ✅ |
| I have used Python since three years. | ✅ |
| It depends of the data size. | ✅ |
| We discussed about the new release. | ✅ |
| I didn't understood the last part. | ✅ |
| I am agree with this idea. | ✅ |
| Yesterday we fix the bug and deploy it. | we fix**ed** the bug and deploy it ❌ (1 of 2) |
| Um, I, I think the, the model is fast. | ✅ fillers and repeats kept |

- 9 of 10 grammar mistakes are kept word for word. The failures are all **"-ed" before a word that starts with /d/ or /ð/** ("deploy the", "fix the"): the sounds merge, so even a person hears "deployed the". small.en behaves the same. "Yesterday I deploy it on the server." → "Yesterday I deployed on the server." with both models (the `say` voice itself runs the words together).
- A priming `--prompt` with mistakes and fillers changed nothing.
- **Consequence:** the golden suite uses typed input (as planned). QA item 4 must be tried with a real voice. If the tutor never sees "deploy", use the typed input, or a sentence like "Yesterday I deploy a new version." Documented as a known limitation.

## llama.cpp JSON schema (Qwen3.5-4B Q4_K_M, llama.cpp b11256, `-c 8192 -np 1`)

1. `anyOf: [null, object]` for `correction` and `[null, string]` for `grammar_note` → converted to a grammar and valid ✅ (6/6 turns valid JSON; `grammar_note` is simply left out when not required).
2. **Keys come out in schema order**, `reply` first ✅ — **but only if the schema we send keeps that order.** `serde_json::Value` sorts object keys by default (`BTreeMap`), which would put `correction` first. → **Enable the `preserve_order` feature of `serde_json`.**
3. The model pretty-prints the JSON (`{\n  "reply": "…`). The reply's first sentence therefore starts after ≈ 6 tokens.

### Latency of a tutor turn (short article summary, history growing from 0 to 5 turns)

| Turn | Prompt tokens read (cached) | First token | Reply's first sentence complete | Whole JSON |
|---|---|---|---|---|
| Opening | 444 (0) | 2.8–3.4 s | 4.5–5.0 s | 9 s |
| 1–5 | 95–170 (440–1,010) | 0.76–1.05 s | **1.9–3.0 s** | 5–9 s |

With STT ≈ 0.2 s and TTS start ≈ 0.1–0.3 s, *end of speech → first audio* is about **2.3–3.5 s** → p50 ≈ 3 s is reachable, and p90 ≤ 5 s holds as long as the cache is reused.

### Prompt cache and history

- A follow-up turn only reads the new user message (≈ 100–170 tokens) when the history is sent **exactly** as before: the same user message text, and the assistant message = the raw JSON the model wrote.
- **Dropping old turns breaks the cache** from the first removed message: after removing 4 exchanges from a 12-exchange history, the model re-read 765 tokens → **first token 4.0 s** (instead of 0.15 s).
- `--cache-reuse N` (KV shifting) does not help: llama-server says *"cache_reuse is not supported by this context"* for Qwen3.5 (a hybrid model).
- → **History window (spec delta):** keep up to 12 exchanges (24 messages) with an 8k context (5 exchanges with 4k). When it is full, drop down to the last 3 exchanges in one step. The slow re-read (≈ 3–4 s extra) then happens about once every 10 turns instead of every turn.
- → **`session_mistakes` goes into the user message** (spec delta): changing the system prompt would make the whole history be re-read.
- Other requests on the single slot (Reader chat, "Explain simply") evict the tutor's cache. → The drill hint uses the macOS dictionary first (syllables + IPA with the stress mark), and calls `define_term` only when the dictionary has nothing.

### Prompt tuning found in the spike

- With the dev spec's prompt, `correction.original` / `corrected` held **only the changed word** ("deploy" → "deployed"). The repeat check needs whole sentences. → Added to the Fields section: *"correction: null, or original = the learner's whole sentence, corrected = the whole corrected sentence."* After the change: whole sentences ✅.
- The tutor sometimes asked a question after "Please say: …". → The correction rule now says *"end the reply with 'Please say: <corrected sentence>'. Nothing comes after it: no question."* Better, but still not always followed.
- Sometimes the tutor gives a correction with `ask_repeat = true` but no "Please say" in the reply. → **Local fallback (spec delta):** when `correction.ask_repeat` is true and the reply doesn't contain "please say", the engine adds the sentence "Please say: <corrected>" itself.
- A disfluent sentence ("Um, I, I think the, the model is fast.") got no correction ✅.

## Microphone permission

- The app asks with AVFoundation (`AVCaptureDevice requestAccessForMediaType:`) before the first recording, and reads the status for the setup page. When the permission is denied, macOS gives a stream of zeros rather than an error, so a clip that is exactly zero also shows a microphone hint.
- In `tauri dev` the binary is not bundled: macOS attributes the prompt to the app that started it (Terminal, IDE). Test the permission flow on the bundled app (QA item 1).

## Golden suite (T7)

30 typed cases (`tests/golden/tutor_corrections.jsonl`). Prompt changes needed to pass: mistake categories per policy, two examples in the system prompt (one correction, one disfluent sentence), "answer what they said", "if your reply says Please say, correction must not be null", temperature 0.3, and the local reconcile. Result: 23/30 → 26/30 → **27/30 (90 %)**, 0 disfluencies corrected; the same on a second run.

## Live session (T14)

`tests/live_voice.rs`: 20 `say` clips through the real engines (see `docs/perf.md`). All phases worked. Found: a late "correction" of an earlier sentence after the learner had already said it right → now dropped.

## Model catalog entries (`models.json`, role `stt`)

| id | file | size | sha256 | license |
|---|---|---|---|---|
| `whisper-base.en` (default) | `ggml-base.en.bin` | 147,964,211 | `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002` | MIT |
| `whisper-small.en` | `ggml-small.en.bin` | 487,614,201 | `c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d` | MIT |

Source: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/<file>` (the models are MIT-licensed, like whisper.cpp).
