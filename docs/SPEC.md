# Tech English — Product & Technical Specification

- **Version:** 0.2 (implementation spec)
- **Date:** 2026-09-29
- **Status:** Draft, ready for Phase 1
- **Dev specs:** [dev/README.md](dev/README.md) (per-phase implementation plans P1–P5)
- **Based on:** the original product spec (v0.1), plus the review changes listed in §3

Words used in this document:
- **MUST** means required.
- **SHOULD** means strongly recommended.
- **MAY** means optional.

Each requirement belongs to a delivery phase (P1–P5, see §18).

---

## 1. Product goal

Build a small macOS desktop app that runs all day. It has two jobs:

1. Find **one genuinely interesting tech story per day** that matches the user's interests.
2. Use that story to teach **English**: reading, vocabulary, speaking practice and correction.

The target user is a **B1 English learner** who works in or is interested in technology.

Core loop:

```
Discover → Read → Understand → Learn vocabulary → Discuss by voice → Correct mistakes → Review later
```

Hard constraints:

- **Local-first.** All AI (LLM, speech-to-text, text-to-speech) runs on the Mac.
- **No mandatory paid APIs**, subscriptions or accounts.
- **Low idle cost.** The app can stay open all day without noticeable CPU or RAM use.

---

## 2. Target environment

| Item | Value |
|---|---|
| Machine | Mac mini (Macmini9,1), Apple M1, 16 GB RAM |
| OS | macOS 14.7 (Sonoma) — minimum supported: macOS 13 |
| Toolchain present | Rust 1.97, Node 26 / npm 11, Homebrew, Xcode CLT |
| Toolchain missing | `cmake` (only needed if llama.cpp / whisper.cpp are built from source) |
| Power | Desktop (no battery); RAM is the main constraint |

Model sizing for 16 GB (see §11 and §12):

- Chat/tutor LLM: 4B-class instruct model, GGUF Q4_K_M (≈2.5–3 GB RAM).
- Optional quality LLM: 7–8B, Q4_K_M (≈5 GB RAM), for summaries.
- Whisper: `base.en` by default, `small.en` as an option.

---

## 3. Changes from v0.1

| # | v0.1 said | v0.2 decision | Reason |
|---|---|---|---|
| C1 | Nothing about article body text | Add a **body extraction** step (Readability) with paywall/failure states | RSS items are often snippets; HN items are links only. The Reader and summaries need full text. |
| C2 | The LLM does "article ranking" | Ranking is **rule-based** (keywords, time, popularity, learned preferences). The LLM MAY only write the "why interesting" text, on demand. | Hibernate Mode keeps ranking running with the LLM off. |
| C3 | Whisper gives pronunciation/stress feedback | Pronunciation practice = **mismatch detection + listen-and-repeat**. Stress hints are text only, from the LLM, and labeled as hints. Real scoring moves to "Later". | Whisper outputs words, not phonetics, and often "fixes" mispronounced words. |
| C4 | llama.cpp / whisper.cpp integration not specified | Run both as **sidecar processes** (`llama-server`, `whisper-server`) on loopback. Unloading a model = stopping its process. | Clean memory release; Hibernate is trivial; crashes can't take the app down. |
| C5 | `AVSpeechSynthesizer` from Rust | Use the web view's **`speechSynthesis`** API (the same macOS voices). Fallback: `say` CLI. | Avoids an Objective-C bridge. Rate, voice and volume are all supported. |
| C6 | Separate Correction + Conversation agents | **One LLM call per spoken turn** returns structured JSON (reply + correction + unknown terms). "Agents" are prompt templates, not separate calls. | Halves voice latency on an M1. |
| C7 | Detecting Mac sleep/wake with OS hooks | **Wall-clock tick scheduler** (every 60 s it checks what is overdue). Wake-up catch-up happens naturally. | Simpler and more robust than OS hooks. |
| C8 | Voice Activity Detection (VAD) only | **Push-to-talk by default.** VAD auto-stop is an option. | B1 learners pause mid-sentence, and VAD cuts them off. |
| C9 | Basic spaced repetition first, FSRS later | Use **FSRS from the start** (`fsrs` crate), mapping the 3 buttons to FSRS grades. | Same effort, better scheduling. |
| C10 | One large MVP | **Phased delivery** (P1–P5), each phase usable on its own. | Reduces risk; each phase can be tested. |
| C11 | Database stored in `data/app.db` in the repo | Data and models stored in `~/Library/Application Support/<bundle-id>/` | Standard macOS location; keeps GBs of models out of the repo. |
| C12 | Daily pick timing not defined | Pick at **08:00 local** (configurable), or at the first opportunity after that time. | Deterministic and testable. |

---

## 4. Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│ Frontend (React + TypeScript, runs in WKWebView)                     │
│  Widget window · Main window (Explore, Reader, Word Book, Practice,  │
│  Talk, Settings) · speechSynthesis (TTS) · Readability extraction    │
└───────────────▲───────────────────────────────┬──────────────────────┘
                │ events                         │ commands (Tauri IPC)
┌───────────────┴───────────────────────────────▼──────────────────────┐
│ Rust core (Tauri 2)                                                  │
│  Scheduler ─ NewsEngine ─ Ranker ─ PickService ─ Notifier            │
│  LearningEngine (Word Book, FSRS, quizzes)                           │
│  AiManager (sidecar lifecycle, LLMProvider, prompt tasks)            │
│  VoiceEngine (mic capture via cpal, STT client, session state)       │
│  ModeManager (Standard / Hibernate) · Settings · SQLite (rusqlite)   │
└───────┬──────────────────────────┬───────────────────────┬───────────┘
        │ HTTPS                    │ HTTP 127.0.0.1         │ HTTP 127.0.0.1
   RSS/Atom, HN API,         llama-server (sidecar)   whisper-server (sidecar)
   article pages,            GGUF LLM                 whisper *.en model
   model downloads
```

Principles:

- **The Rust core owns all state and all network access.** The frontend is a view. It calls commands and listens to events.
- **The AI is behind interfaces:**
  - `LlmProvider` (implementations: `LocalLlamaProvider`, later `OpenAiProvider` / `AnthropicProvider`)
  - `SttProvider` (`WhisperServerProvider`)
  - `TtsProvider` (`WebSpeechProvider`, `SayCliProvider`)
- **"Agents" are named prompt tasks** (§11.4). They share one loaded model.
- **Nothing heavy loads at startup.** Models load on first use and unload after an idle timeout or when Hibernate starts.

---

## 5. Application shell (P1)

### 5.1 Windows

| Window | Spec |
|---|---|
| **Widget** | 340 × ~260 px, frameless, rounded corners, can be dragged. Default position: top-right of the main screen (tauri-plugin-positioner). Remembers its last position. Options: always-on-top (default **on**), normal, collapsed **pill** (≈180 × 36: "Today's pick · 23 due"), hidden. |
| **Main** | 1000 × 700, normal window. Pages: Today, Explore, Reader, Word Book, Practice, Talk, Settings. Opened from the widget or the tray. |

- Closing a window MUST NOT quit the app. Closing the widget hides it. The main window is created when first opened and **destroyed when closed**, which frees its WebKit process (about 70 MB) while the app is idle.
- Only **Quit** from the tray menu (or ⌘Q in the main window, after a confirm) exits.
- The app MUST be single-instance (tauri-plugin-single-instance).
- Dock icon: hidden by default (Accessory activation policy). A setting can show it.
- Theme follows the system light/dark setting.

### 5.2 Widget content

```
┌─────────────────────────────────┐
│ Tech English        ● STANDARD  │   (mode badge; 💤 HIBERNATE)
├─────────────────────────────────┤
│ Today's Pick                    │
│ New local AI model optimized…   │
│ AI Agents · Ars Technica · 2h   │
│ Medium English · 6 min read     │
│ Why: matches AI Agents, LLMs;   │
│      412 points on Hacker News  │
│ [Read] [Listen] [Discuss] [⋯]   │   (⋯ = Save / Not interested)
├─────────────────────────────────┤
│ 📚 23 words due  [Practice]     │   (P2+)
└─────────────────────────────────┘
```

Widget states:
- **pick ready**
- **no pick yet** (before 08:00, shows the top Explore item as "Preview")
- **no eligible stories** ("Nothing matched today — open Explore")
- **first run** (onboarding CTA)

Buttons that belong to later phases are hidden until those phases ship.

### 5.3 Tray (menu-bar) menu

- Show/Hide widget
- Open Tech English
- Today's pick: *title* (opens the Reader)
- Mode ▸ Standard / Hibernate
- Refresh news now
- Launch at login ✓ (tauri-plugin-autostart)
- Quit

### 5.4 First-run onboarding (P1)

1. Choose topics from suggestions (AI, LLMs, AI Agents, Python, Cybersecurity, Cloud/AWS, Backend, Databases, **Data Engineering**, Robotics, Startups, Apple, Machine Learning). Topics can be edited later.

   Seed keywords for Data Engineering: data pipeline, ETL, ELT, data warehouse, lakehouse, data lake, Spark, Kafka, Flink, Airflow, dbt, DuckDB, Iceberg, Delta Lake, Snowflake, Databricks, streaming, batch processing, data quality, orchestration.
2. Confirm the default feeds (§7.2). Feeds can be turned off.
3. Choose the daily pick time (default 08:00) and notification preferences.
4. Choose whether to launch at login.
5. The first fetch runs immediately. **No high-interest notifications are sent from the first fetch** (this prevents a burst of notifications).

---

## 6. Operating modes & resource management (P1, extended P3/P4)

| Component | Standard | Hibernate |
|---|---|---|
| Feed fetching | every 20 min (setting: 15–30) | every 45 min (setting: 30–60) |
| Dedupe, topic matching, ranking, daily pick | on | on |
| Notifications | on | on |
| Body extraction (for the daily pick only) | on | on |
| LLM sidecar | loaded on demand; unloaded after 10 min idle | **stopped**; requests are refused with "Switch to Standard" |
| Whisper sidecar | loaded when a voice session starts; unloaded 5 min after it ends | **stopped** |
| Microphone | open only while push-to-talk is held/active | **closed** |
| TTS | on | off |
| Word Book browsing | on | read-only |
| Quizzes / Talk | on | disabled (UI prompts to switch) |

Switching to Hibernate MUST:
1. Ask for confirmation if a voice session is active, then end it (the review screen is saved as a draft).
2. Cancel in-flight LLM jobs.
3. Stop both sidecars and confirm the processes have exited.
4. Change the fetch intervals.
5. Persist the mode.

Switching to Standard MUST NOT preload models.

The mode persists across restarts.

### 6.1 Sleep / wake

The scheduler ticks every 60 s using the wall clock. After the Mac wakes, the first tick finds that feeds are overdue and fetches them.

If the network is not ready yet, the fetch is retried with backoff (30 s, 60 s, 120 s).

The daily pick check runs on the same tick (§7.8).

---

## 7. News engine (P1)

### 7.1 Topics

Each topic has these fields:
- `name`
- `keywords[]` (words or phrases; case-insensitive; matched on word boundaries)
- `excluded_keywords[]`
- `priority` (low / normal / high)
- `enabled`
- `notify` (bool)
- `notify_threshold` (0–100, or empty to use the global value)

### 7.2 Sources

There is one `FeedSource` trait with two implementations: `RssSource` (RSS 2.0 / Atom / JSON Feed, via the `feed-rs` crate) and `HackerNewsSource`.

```rust
#[async_trait]
trait FeedSource {
    async fn fetch(&self, ctx: &FetchCtx) -> Result<FetchResult>; // items + etag/last-modified
}
```

**Hacker News** (official Firebase API, no authentication):
- Each poll reads `topstories` (top 60), `beststories` (top 30) and `showstories` (top 20). `newstories` is off by default (too noisy) but can be turned on.
- Items are cached. An item's score and comment count are refreshed only if the cached copy is older than 1 hour.
- Items without a `url` (Ask HN) link to the HN discussion page.

**Default feeds.** These are seeded but can be edited. Every URL MUST be verified during implementation, and any that are dead get dropped.

| Name | URL | Kind |
|---|---|---|
| Hacker News | `hn:top,best,show` | hn |
| Ars Technica | https://feeds.arstechnica.com/arstechnica/index | rss |
| The Verge | https://www.theverge.com/rss/index.xml | rss |
| TechCrunch | https://techcrunch.com/feed/ | rss |
| GitHub Blog | https://github.blog/feed/ | rss |
| Cloudflare Blog | https://blog.cloudflare.com/rss/ | rss |
| AWS News Blog | https://aws.amazon.com/blogs/aws/feed/ | rss |
| Hugging Face Blog | https://huggingface.co/blog/feed.xml | rss |
| Simon Willison | https://simonwillison.net/atom/everything/ | rss |
| Krebs on Security | https://krebsonsecurity.com/feed/ | rss |
| DEV Community tags | `https://dev.to/feed/tag/{python,ai,aws,llm,machinelearning,dataengineering}` | rss |

**AI feeds.** Added 2026-09-29; all returned HTTP 200 with recent items on that date.

| Name | URL | Notes |
|---|---|---|
| OpenAI News | https://openai.com/news/rss.xml | Full history (1,200+ items). The ingest age limit (§7.3) applies. |
| Google DeepMind Blog | https://deepmind.google/blog/rss.xml | |
| Google Research Blog | https://research.google/blog/rss/ | |
| MIT Technology Review — AI | https://www.technologyreview.com/topic/artificial-intelligence/feed | Good general-audience English |
| Latent Space | https://www.latent.space/feed | AI engineering |
| Import AI | https://importai.substack.com/feed | Weekly AI research digest |
| Interconnects | https://www.interconnects.ai/feed | LLM training / open models |
| Ahead of AI (Sebastian Raschka) | https://magazine.sebastianraschka.com/feed | Clear LLM explainers; infrequent |

**Data engineering feeds.** Added 2026-09-29; all verified the same way.

| Name | URL | Notes |
|---|---|---|
| Databricks Blog | https://www.databricks.com/feed | |
| Snowflake Blog | https://www.snowflake.com/feed/ | Some product marketing. Default `source_weight` 0.35. |
| Confluent Blog | https://www.confluent.io/rss.xml | Kafka / streaming |
| dbt Labs Blog | https://www.getdbt.com/blog/rss.xml | Analytics engineering |
| DuckDB Blog | https://duckdb.org/feed.xml | |
| Data Engineering Weekly | https://www.dataengineeringweekly.com/feed | Weekly digest |
| Seattle Data Guy | https://seattledataguy.substack.com/feed | Easy English, career-oriented |
| Netflix TechBlog | https://netflixtechblog.com/feed | Data platform & infrastructure |
| Towards Data Science | https://towardsdatascience.com/feed | High volume, mixed quality. Default `source_weight` 0.35. |

These were checked and **excluded** on 2026-09-29:

| Feed | Result |
|---|---|
| Anthropic (`/rss.xml`) | 404; no official feed |
| The Batch (`deeplearning.ai/the-batch/feed/`) | 404 |
| LangChain Blog (`blog.langchain.dev/rss/`) | 200 but 0 items |
| Start Data Engineering | Timed out |

These MAY be re-checked later.

Users can add any RSS/Atom URL. **Test feed** fetches the URL and shows the title and item count before saving.

### 7.3 Fetching

- Up to 4 feeds are fetched at a time. Each request times out after 15 s.
- Every request sends a `User-Agent: TechEnglish/<version> (+local desktop app)` header.
- Conditional GET: store `ETag` and `Last-Modified`; a `304` response counts as success with no new items.
- On failure, the next attempt waits `interval × 2^failures`, capped at 6 h. The error is shown next to the feed in Settings.
- The scheduler spreads feeds out so they don't all fetch in the same tick.
- **English only.** Items whose title + description are clearly not English are skipped (a stop-word heuristic plus a non-Latin-script check), because the app teaches English.
- **Ingest age limit.** Items whose `published_at` is older than 7 days (setting) are ignored when fetched. Some feeds (e.g. OpenAI News) contain their whole history, and without this limit the first fetch would flood the database.

### 7.4 Normalization & deduplication

**URL normalization** (stored in `normalized_url`, which is UNIQUE). The steps:
1. Lowercase the scheme and host.
2. Strip `www.`.
3. Drop the `#fragment`.
4. Remove `utm_*`, `fbclid`, `gclid`, `ref`, `source` and `mc_*` query parameters.
5. Sort the remaining parameters.
6. Drop a trailing `/`.

**Title key** (`title_key`):
1. Lowercase.
2. Strip punctuation.
3. Collapse whitespace.
4. Remove a trailing source suffix (e.g. ` - The Verge`, ` | TechCrunch`).

**Duplicate** means any of these is true:
1. The same `normalized_url`.
2. The same `canonical_url` (known after body extraction).
3. The same `title_key`.
4. The Jaccard similarity of the title word sets is ≥ 0.8 compared with an article from the last 72 h.
5. *(Later)* semantic similarity.

When a duplicate is found, merge it into the earliest record:
- Keep the earliest `discovered_at`.
- Attach HN stats if one copy came from HN (HN ↔ RSS matching is by normalized URL).
- Record the extra source in `article_sources`.

### 7.5 Topic matching

For each enabled topic, compute:

```
title_hits = distinct keywords found in title
desc_hits  = distinct keywords found in description (+ first 500 chars of body, if known)
raw        = min(1, 0.6 × title_hits + 0.25 × desc_hits)
relevance  = raw × priority_factor      (low 0.6 · normal 0.8 · high 1.0)
```

- If any excluded keyword appears in the title or description, that topic's relevance is 0.
- `topic_relevance(article) = max over topics`.
- Pairs with relevance > 0 are stored in `article_topics`. The primary topic is the one with the highest relevance.

### 7.6 Ranking

All components are in [0, 1]. The weights can be changed in Settings; the defaults are:

| Component | Weight | Definition |
|---|---|---|
| topic_relevance | 0.30 | §7.5 |
| freshness | 0.15 | `0.5 ^ (age_hours / 24)`, where age = now − (published_at or discovered_at) |
| popularity | 0.15 | HN: `min(1, log10(1+points)/log10(501)) × 0.8 + min(1, comments/200) × 0.2`. No HN match: 0.4 |
| source_preference | 0.20 | The feed's `source_weight` (user-set 0–1; seeded per source, see below) |
| novelty | 0.10 | `1 − max title-Jaccard` against articles picked or opened in the last 7 days |
| user_history | 0.10 | `0.5 + 0.5 × clamp(topic_affinity(primary) + source_affinity, −1, 1)` |

> **Tuned 2026-09-29 on live feeds** (v0.1 weights were 0.35 / 0.20 / 0.15 / 0.10 / 0.10 / 0.10 with a 12 h half-life). With the old values, minutes-old DEV Community posts filled the whole top 15. Seeded `source_weight`s: 0.6 for primary blogs, Ars Technica and Hacker News; 0.5 for The Verge and TechCrunch; 0.35 for Snowflake and Towards Data Science; 0.3 for DEV Community tags.

```
score = 100 × Σ(weight × component)
```

`score_breakdown` (JSON) is stored for display. Explore shows it in a tooltip.

Scores are recomputed for articles from the last 72 h on each fetch cycle, because freshness changes over time.

### 7.7 Learning from interactions

Interaction kinds:
- `opened`
- `read` (≥ 60 % scrolled, or ≥ 50 % of the estimated reading time)
- `skipped` (the daily pick was not opened by the end of its day)
- `saved`
- `discussed`
- `liked`
- `not_interested`

Affinity updates, stored in `affinities`, each clamped to [−1, 1]:

| Interaction | Δ topic affinity (× relevance) | Δ source affinity |
|---|---|---|
| read | +0.05 | +0.03 |
| saved / liked / discussed | +0.10 | +0.05 |
| skipped | −0.02 | −0.01 |
| not_interested | −0.15 | −0.08 |

`not_interested` also hides the article permanently.

### 7.8 Daily pick

**Trigger.** On each tick, if there is no `daily_picks` row for the local date and now ≥ `pick_time` (default 08:00), a pick is chosen. If the app was asleep or closed at 08:00, the pick is made at the first tick after that.

**Eligible articles:**
- topic_relevance ≥ 0.3
- age ≤ 48 h
- not dismissed or marked not interested
- not picked in the last 14 days
- `body_status ≠ paywalled`

If no article is eligible, the age limit is relaxed to 72 h. If still nothing is eligible, no pick is made and the widget shows the "no eligible stories" state.

**Selection.** The highest score wins. Then:
1. Body extraction runs for the winner (§8.1).
2. If extraction finds a paywall, the next candidate is tried (up to 3 attempts).
3. Difficulty and reading time are computed (§8.2).

**"Why you may find it interesting":**
- **P1:** built from a template, e.g. `Matches your topics: AI Agents, LLMs · 412 points on Hacker News · from a source you read often`.
- **P3:** MAY be replaced by a 1–2 sentence LLM text, generated lazily the first time the pick is shown in Standard Mode.

Card actions:
- Read
- Easy Summary (P3)
- Listen (P3; TTS of the summary, or of the title and description in P2)
- Discuss (P4)
- Save
- Not interested

### 7.9 Notifications

The app uses tauri-plugin-notification. Clicking a notification SHOULD open the Reader for that article. This is best effort: desktop click callbacks are limited, so the fallback is to show the main window on Today when the app is activated right after a notification (P1 dev spec §4.10).

| Kind | Rule |
|---|---|
| Daily pick | Exactly once per day, when the pick is made. Text: `Today's tech story is ready.` / title / `AI Agents · 6 min read` |
| High interest | score ≥ threshold (topic or global, default 85) **and** a topic with `notify = true` **and** max 3/day (setting) **and** ≥ 90 min since the last notification **and** outside quiet hours (default 22:00–08:00) **and** the article was never notified before **and** it is not from the first fetch after install |

Every notification sent is written to `notifications_log`.

> Note: in `tauri dev` builds, macOS may attribute notifications to the terminal or suppress them. Notification behaviour is verified on a `tauri build` bundle.

### 7.10 Retention

Articles older than 60 days are deleted, unless they are saved, linked to vocabulary, or linked to a conversation.

`body_text` is cleared after 14 days for articles that are not saved.

This job runs once a day.

---

## 8. Reader (P2, AI modes in P3)

### 8.1 Body extraction

1. Rust fetches the article HTML (command `fetch_article_html`, 15 s timeout, max 3 MB).
2. The frontend runs **`@mozilla/readability`** on it (with `DOMParser`) to get clean text and simple HTML.
3. The frontend calls `save_article_body(id, text, html, canonical_url)`.

`body_status` values: `none` · `ok` · `failed` · `paywalled`.

The status is `paywalled` if:
- the extracted text is under 150 words while the description suggests a longer article, **or**
- known paywall markers are found.

For the daily pick, the Rust core emits `article://needs-body`. The widget web view does the extraction even while hidden. (It is always loaded in both modes.)

### 8.2 Difficulty & reading time (no LLM)

- `word_count` = number of words in the body.
- Reading time = `word_count / 150` minutes (B1 reading speed).
- `rare_ratio` = the share of words **not** in a bundled list of the most common 3,000 English words. The word list's license MUST allow redistribution.
- Difficulty uses Flesch Reading Ease (FRE) and `rare_ratio`:

| Difficulty | Rule |
|---|---|
| Easy | FRE ≥ 60 and rare_ratio < 0.15 |
| Hard | FRE < 40 or rare_ratio ≥ 0.25 |
| Medium | everything else |

### 8.3 Reader modes

| Mode | Phase | Content |
|---|---|---|
| Original | P2 | Extracted text in a clean reading view, plus an "Open in browser" button. If extraction failed: title, description and the link. |
| B1 Summary | P3 | LLM task `summarize_b1` (§11.4). Keeps the key technical concepts; ≤ 250 words. |
| Easy English | P3 | LLM task `simplify_easy`. Short sentences, one idea per sentence, common words, technical terms explained inline. |

LLM outputs are cached in `article_derivatives`, keyed by article, kind and model id.

### 8.4 Selection popup

Selecting 1–8 words anywhere in the Reader shows a popup:

- The selected text, plus 🔊 (TTS at the current rate).
- **P2:** meaning (optional, typed by the user), then **Add to Word Book**. The sentence around the selection is always saved as context.
- **P3:** **Explain** (LLM task `define_term`, using the sentence as context). It shows a simple meaning, B1 meaning, part of speech, 2–3 examples (one related to the article) and collocations. **Add to Word Book** saves all of these.
- **P3:** **Ask AI** opens a small chat panel scoped to the article.

In the Reader, words that are already in the Word Book SHOULD be underlined lightly.

---

## 9. Word Book (P2)

### 9.1 Item kinds

`word` · `phrase` · `sentence` · `term` (technical term) · `pronunciation` · `correction`

For a `correction` item:
- `text` = the corrected sentence
- `notes` = the original and the explanation

### 9.2 Fields

See `vocab_items` in §13. Saved context (the sentence, plus the article or conversation it came from) lives in `vocab_contexts`, so one item can have many contexts.

### 9.3 Adding items

- Items are deduplicated by `(kind, text_key)`, where `text_key` = lowercase, trimmed, with collapsed whitespace. Adding an existing item adds a new context. If the item's status is `known`, it goes back to `learning`.
- Items with no meaning are allowed. They appear as "Meaning pending" and are **skipped by quizzes** until they have a meaning.
- **P3** auto-fills pending meanings in the background, in Standard Mode, when the LLM is loaded anyway.

### 9.4 Word Book page

- A list with a search box and filters (kind, status, due, source article).
- A detail view: meaning, examples, 🔊, contexts with links to the source, review history, and Edit / Delete.
- Export to CSV (P2); Anki export is for later.

---

## 10. Practice & spaced repetition (P2)

### 10.1 Scheduling: FSRS

The app uses the `fsrs` crate with default parameters. Button mapping:

| Button | Score | FSRS grade |
|---|---|---|
| Forgot | 0 | Again |
| Not Sure | 0.5 | Hard |
| Remember | 1.0 | Good |

Each review updates `stability`, `difficulty`, `due_at`, `srs_state`, the counters and `last_reviewed_at`. It also appends a row to `vocab_reviews`.

Status changes:
- `new` → `learning` after the first review.
- `learning` → `known` when stability ≥ 21 days.
- A **Forgot** on a `known` item moves it back to `learning`.

### 10.2 Quiz selection (default 10 items, setting 5–30)

Items are filled from these buckets in order. Within a bucket there is light randomness (a random pick from the top 2× candidates):

1. Last grade was Forgot, within the last 3 days
2. Last grade was Not Sure
3. Due (`due_at ≤ now`), most overdue first
4. New (never reviewed), oldest first — at most 3 per quiz
5. Well-known items, at random, to fill the rest

The final order is shuffled.

- With fewer than 10 eligible items, the quiz uses all of them.
- With fewer than 3, Practice is disabled and shows "Add a few more words first".

### 10.3 Quiz flow

```
Question 3 / 10
SCALABLE                       (for sentence/correction kinds: "How would you say this correctly?" + original)
Think about the answer.
[Show Answer]  (Space)
────────────
Meaning · example · 🔊 · source context
[Forgot] (1)  [Not Sure] (2)  [Remember] (3)
```

The result screen shows the counts, the score, `(remember + 0.5 × unsure) / total` (as a %), and the list of missed items. The result is stored in `quiz_sessions`.

The widget shows the count of items due today.

---

## 11. AI engine (P3)

### 11.1 LlmProvider

```rust
#[async_trait]
trait LlmProvider {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse>;
    async fn stream(&self, req: LlmRequest) -> Result<BoxStream<'static, Result<LlmChunk>>>;
    fn model_id(&self) -> &str;
}
// LlmRequest { messages, max_tokens, temperature, json_schema: Option<Value>, cache_key: Option<String> }
```

`LocalLlamaProvider` talks to `llama-server`'s OpenAI-compatible `/v1/chat/completions`:
- streaming over SSE
- JSON-schema constrained output for structured tasks
- `cache_prompt` enabled so multi-turn conversations reuse the KV cache

A `MockProvider` is used in tests.

### 11.2 Sidecar lifecycle (AiManager)

Lifecycle:

```
Unloaded → Starting → Ready ⇄ Busy → (idle timeout / Hibernate / Quit) → Stopping → Unloaded
```

- Launch: `llama-server -m <model.gguf> --host 127.0.0.1 --port <free port> --api-key <random per launch> -c 8192 -ngl 99`.
- It is ready when `GET /health` returns OK. The startup timeout is 60 s.
- **Memory guard.** Before launching, check the free RAM plus RAM that macOS can reclaim. If it is less than the model file size + 1.5 GB, show a warning and let the user cancel.
- The idle timeout is 10 min (setting). A voice session keeps the model loaded.
- On app quit, and on Hibernate, send SIGTERM, then SIGKILL after 5 s. Verify the process has exited. There MUST be no orphan processes (the PID is stored in app state and checked at the next launch).
- The frontend gets `ai://status` events (`unloaded` / `loading` / `ready` / `busy` / `error`) so it can show load progress.

**Where the binaries come from:**
- **Development:** Homebrew (`brew install llama.cpp whisper-cpp`), or a path set in Settings.
- **Distribution (P5):** bundled as Tauri `externalBin` sidecars.

### 11.3 Models

- The model catalog is a bundled `models.json`. Each entry has: id, display name, role (`chat`, `quality`, `stt`), download URL (Hugging Face), size, sha256, license name and license URL.
- The in-app downloader:
  - shows the size and license before downloading;
  - can resume interrupted downloads;
  - checks the sha256;
  - stores models in `~/Library/Application Support/<bundle-id>/models/`.
- **Choosing the default chat model (a P3 spike).** Benchmark 2–3 current 4B-class and 7–8B-class instruct GGUFs on this M1. Default = the best quality model that meets all of these:
  - ≥ 20 tokens/s generation
  - ≤ 1.5 s to the first token on a follow-up conversation turn
  - valid JSON on ≥ 98 % of the tutor-turn schema test prompts
- The license of every model MUST be recorded in `models.json` and checked before distribution.

### 11.4 Prompt tasks ("agents")

Each task has a versioned template in `src-tauri/src/ai/prompts/`. Tasks with structured output have a JSON schema. All prompts take the `english_level` setting (Level 1 / 2 / 3, see §12.3).

| Task (v0.1 agent name) | Input | Output |
|---|---|---|
| `why_interesting` (News) | title, description, matched topics | 1–2 sentences |
| `summarize_b1` (Reader) | body (truncated to fit) | ≤ 250-word B1 summary |
| `simplify_easy` (English Tutor) | body or summary | Easy English text |
| `define_term` (Vocabulary) | term, context sentence, article title | JSON: `{meaning_simple, meaning_b1, part_of_speech, ipa?, examples[3], collocations[]}` |
| `tutor_turn` (Voice + Correction) | §12.4 | JSON (§12.4) |
| `session_review` (Learning) | observations + transcript | JSON: suggestions with a `preselected` flag |

Long bodies are trimmed to the context budget, in this order of preference: first paragraphs, then paragraphs containing the topic keywords.

---

## 12. Voice tutor (P4)

### 12.1 Pipeline

```
[Push-to-talk (hold Space / click mic)]
      → cpal capture (48 kHz) → resample to 16 kHz mono (rubato) → in-memory WAV
      → whisper-server /inference  (English model, temperature 0, no prompt priming)
      → transcript
      → local intent check (§12.5)  ──(handled locally)──► TTS
      → tutor_turn (LLM, streamed JSON)
      → TTS, sentence by sentence as the "reply" field streams
```

- **The microphone is open only while recording.** Audio buffers are dropped after transcription. Audio is never written to disk unless the `debug.keep_audio` setting is on (default off).
- A single utterance is limited to 60 s.
- Optional **VAD auto-stop** (setting): stop after 1.2 s of silence. Push-to-talk stays the default.
- `NSMicrophoneUsageDescription` MUST be set in `Info.plist`. If mic permission is denied, the Talk page shows a clear screen explaining how to enable it in System Settings.
- The Whisper sidecar lifecycle is the same as §11.2. It starts when a session starts and stops 5 min after the session ends.

### 12.2 Session setup screen

| Setting | Values | Default |
|---|---|---|
| Topic | Today's pick / any article / free talk | Today's pick |
| English level | Level 1 Very Easy / Level 2 B1 / Level 3 Natural | Level 2 |
| Speech rate | 0.5–1.2 | per level |
| Pause between sentences | 0–1500 ms | 400 ms |
| Correction frequency | Low / Medium / High | High |
| Voice | installed English voices (`speechSynthesis.getVoices()`, accent shown) | best en-US / en-GB voice found |

### 12.3 Levels

| Level | Rate | Style |
|---|---|---|
| 1 Very Easy | 0.7 | ≤ 10-word sentences, the most common words, explain every technical term, check understanding often |
| 2 B1 | 0.85 | Natural but simple sentences, explain technical terms once |
| 3 Natural | 1.0 | Normal conversation, less simplification |

### 12.4 `tutor_turn` contract

Input:
- system prompt (role, level, correction policy)
- the article's B1 summary (≤ 400 words)
- the last ≤ 12 turns
- the current tutor state (`DISCUSS` or `AWAIT_REPEAT` + the target sentence)
- the user's transcript

Output (JSON schema enforced; `reply` MUST be the first key so it can be streamed):

```json
{
  "reply": "What the tutor says. If there is a correction, it comes first, then the discussion continues.",
  "correction": null,
  "unknown_terms": [{ "text": "deployment", "explanation": "putting software where people can use it" }],
  "useful_phrases": ["It depends on..."],
  "grammar_note": null
}
```

`correction`, when present, has this shape:

`{ "original": "Yesterday I deploy the application.", "corrected": "Yesterday I deployed the application.", "explanation": "Use the past form because you are talking about yesterday.", "ask_repeat": true }`

**Correction policy.** There is **at most one correction per turn**. What gets corrected depends on the setting:

| Setting | Corrects |
|---|---|
| Low | only errors that change the meaning |
| Medium | important errors, plus errors the user has made more than once in this session |
| High | any clear grammar or word-choice error |

Disfluencies (um, restarts) are never corrected.

**Conversation state machine:**
- `DISCUSS`: the normal turn.
- If `correction.ask_repeat` → go to `AWAIT_REPEAT(target = corrected)`.
- `AWAIT_REPEAT`: the next transcript is compared with the target **locally**, with no LLM call. The comparison uses normalized word-level similarity ≥ 0.85.
  - Pass → say "Good. Let's continue." and go back to `DISCUSS`. The next LLM turn continues the topic.
  - Fail → say "Almost. Listen again:", replay the target at rate − 0.15, and stay in `AWAIT_REPEAT`.
  - After the 2nd failure → say "Good try. Let's continue." and go back to `DISCUSS`.
- Explaining vocabulary MUST NOT lose the thread. The system prompt tells the tutor to come back to the last discussion question after an explanation.

### 12.5 Local intents (no LLM call)

Transcripts are matched with simple phrase patterns:

| User says (examples) | Action |
|---|---|
| "say that again", "repeat please", "pardon?" | Replay the last reply |
| "speak slower", "slower please" | Rate − 0.1 (min 0.5); replay the last reply |
| "speak faster" | Rate + 0.1 (max 1.2) |
| "stop", "end the conversation" | Go to the end-of-session review |

Everything else goes to `tutor_turn`. Requests such as "what does X mean", "explain simply" and "give me an example" are handled by the LLM, following the level rules.

### 12.6 Pronunciation practice (revised scope)

This starts when the user says "How do I say *X*?" or taps a word in the transcript.

1. The tutor says the word slowly (TTS at rate 0.6). It shows the syllables and the stressed syllable as a **hint**, e.g. `sca·la·BIL·i·ty` (from `define_term`, marked as "hint").
2. The user records the word. If Whisper's transcript does not contain the target word (normalized), the app says: "I heard '*heard*'. Listen again:" and replays it. It allows up to 3 attempts.
3. Each drill is logged as a `pronunciation` observation, with the heard/target pairs.

The app MUST NOT claim to measure stress or accuracy beyond "I heard X".

### 12.7 Session log & end-of-session review

During a session, `learning_observations` rows are written with these kinds:
- `unknown_word`
- `unknown_phrase`
- `pronunciation`
- `grammar` (with the correction)
- `useful_sentence`

**Nothing goes into the Word Book automatically.**

When the session ends:
1. Show a summary: speaking time (sum of the user's recorded seconds), new vocabulary, corrected sentences, pronunciation drills.
2. `session_review` builds the suggestion list with preselected checkboxes, grouped as Words / Phrases / Corrections.
3. **Add Selected** creates `vocab_items`, with `vocab_contexts` linked to the conversation and article, and sets `saved_item_id` on the observations. **Skip** keeps the observations but saves nothing.
4. If a session ended because of Hibernate or Quit, its review can be finished later from Talk → History.

Transcripts are kept (setting: keep for N days, default 90, or forever).

---

## 13. Data model (SQLite)

- Engine: `rusqlite` (bundled SQLite), WAL mode, `foreign_keys=ON`.
- Migrations are embedded SQL files, applied in order and tracked with `PRAGMA user_version`.
- Times are ISO-8601 UTC text. The local date is used only for `daily_picks.date`.
- JSON columns are TEXT.

```sql
-- P1 ─────────────────────────────────────────────
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);           -- JSON values
CREATE TABLE app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);          -- mode, sidecar pids, first_fetch_done…

CREATE TABLE topics (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  keywords TEXT NOT NULL DEFAULT '[]',
  excluded_keywords TEXT NOT NULL DEFAULT '[]',
  priority INTEGER NOT NULL DEFAULT 2 CHECK (priority IN (1,2,3)),
  enabled INTEGER NOT NULL DEFAULT 1,
  notify INTEGER NOT NULL DEFAULT 0,
  notify_threshold INTEGER,
  created_at TEXT NOT NULL
);

CREATE TABLE feeds (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('rss','hn')),
  name TEXT NOT NULL,
  url TEXT NOT NULL UNIQUE,
  source_weight REAL NOT NULL DEFAULT 0.5,
  enabled INTEGER NOT NULL DEFAULT 1,
  etag TEXT, last_modified TEXT,
  last_fetched_at TEXT, last_error TEXT,
  consecutive_failures INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

CREATE TABLE articles (
  id INTEGER PRIMARY KEY,
  url TEXT NOT NULL,
  normalized_url TEXT NOT NULL UNIQUE,
  canonical_url TEXT,
  title TEXT NOT NULL,
  title_key TEXT NOT NULL,
  source_name TEXT NOT NULL,
  author TEXT,
  description TEXT,
  published_at TEXT,
  discovered_at TEXT NOT NULL,
  hn_id INTEGER UNIQUE, hn_points INTEGER, hn_comments INTEGER, hn_checked_at TEXT,
  body_text TEXT, body_html TEXT,
  body_status TEXT NOT NULL DEFAULT 'none' CHECK (body_status IN ('none','ok','failed','paywalled')),
  word_count INTEGER, difficulty TEXT CHECK (difficulty IN ('easy','medium','hard')),
  primary_topic_id INTEGER REFERENCES topics(id) ON DELETE SET NULL,
  score REAL, score_breakdown TEXT, scored_at TEXT,
  read_status TEXT NOT NULL DEFAULT 'unread' CHECK (read_status IN ('unread','opened','read')),
  saved INTEGER NOT NULL DEFAULT 0,
  hidden INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_articles_discovered ON articles(discovered_at);
CREATE INDEX idx_articles_score ON articles(score DESC);
CREATE INDEX idx_articles_title_key ON articles(title_key);

CREATE TABLE article_sources (
  article_id INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
  item_url TEXT NOT NULL,
  seen_at TEXT NOT NULL,
  PRIMARY KEY (article_id, feed_id)
);

CREATE TABLE article_topics (
  article_id INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  topic_id INTEGER NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
  relevance REAL NOT NULL,
  PRIMARY KEY (article_id, topic_id)
);

CREATE TABLE article_interactions (
  id INTEGER PRIMARY KEY,
  article_id INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('opened','read','skipped','saved','discussed','liked','not_interested')),
  created_at TEXT NOT NULL
);

CREATE TABLE affinities (
  kind TEXT NOT NULL CHECK (kind IN ('topic','source')),
  ref_id INTEGER NOT NULL,          -- topics.id or feeds.id
  value REAL NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (kind, ref_id)
);

CREATE TABLE daily_picks (
  date TEXT PRIMARY KEY,            -- local YYYY-MM-DD
  article_id INTEGER NOT NULL REFERENCES articles(id),
  why TEXT NOT NULL,
  why_source TEXT NOT NULL DEFAULT 'template' CHECK (why_source IN ('template','llm')),
  created_at TEXT NOT NULL
);

CREATE TABLE notifications_log (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('daily_pick','high_interest')),
  article_id INTEGER REFERENCES articles(id) ON DELETE SET NULL,
  sent_at TEXT NOT NULL
);

-- P2 ─────────────────────────────────────────────
CREATE TABLE vocab_items (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('word','phrase','sentence','term','pronunciation','correction')),
  text TEXT NOT NULL,
  text_key TEXT NOT NULL,
  meaning_simple TEXT, meaning_b1 TEXT,
  part_of_speech TEXT, ipa TEXT, syllables TEXT,
  examples TEXT NOT NULL DEFAULT '[]', collocations TEXT NOT NULL DEFAULT '[]',
  notes TEXT,
  status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','learning','known')),
  srs_state TEXT, stability REAL, difficulty REAL, due_at TEXT, last_reviewed_at TEXT,
  last_grade TEXT CHECK (last_grade IN ('forgot','unsure','remember')),
  review_count INTEGER NOT NULL DEFAULT 0,
  remember_count INTEGER NOT NULL DEFAULT 0,
  unsure_count INTEGER NOT NULL DEFAULT 0,
  forgot_count INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  UNIQUE (kind, text_key)
);
CREATE INDEX idx_vocab_due ON vocab_items(due_at);

CREATE TABLE vocab_contexts (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL REFERENCES vocab_items(id) ON DELETE CASCADE,
  article_id INTEGER REFERENCES articles(id) ON DELETE SET NULL,
  conversation_id INTEGER REFERENCES conversations(id) ON DELETE SET NULL,
  sentence TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE quiz_sessions (
  id INTEGER PRIMARY KEY,
  started_at TEXT NOT NULL, finished_at TEXT,
  item_count INTEGER NOT NULL,
  remember INTEGER NOT NULL DEFAULT 0, unsure INTEGER NOT NULL DEFAULT 0, forgot INTEGER NOT NULL DEFAULT 0,
  score REAL
);

CREATE TABLE vocab_reviews (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL REFERENCES vocab_items(id) ON DELETE CASCADE,
  quiz_session_id INTEGER REFERENCES quiz_sessions(id) ON DELETE SET NULL,
  grade TEXT NOT NULL CHECK (grade IN ('forgot','unsure','remember')),
  reviewed_at TEXT NOT NULL,
  stability_after REAL, difficulty_after REAL, due_after TEXT
);

-- P3 ─────────────────────────────────────────────
CREATE TABLE article_derivatives (
  article_id INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('summary_b1','easy_english','why_interesting')),
  model_id TEXT NOT NULL,
  prompt_version TEXT NOT NULL,
  text TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (article_id, kind)
);

-- P4 ─────────────────────────────────────────────
CREATE TABLE conversations (
  id INTEGER PRIMARY KEY,
  article_id INTEGER REFERENCES articles(id) ON DELETE SET NULL,
  settings TEXT NOT NULL,            -- level, rate, correction frequency, voice
  started_at TEXT NOT NULL, ended_at TEXT,
  user_speaking_seconds INTEGER NOT NULL DEFAULT 0,
  review_status TEXT NOT NULL DEFAULT 'pending' CHECK (review_status IN ('pending','done','skipped'))
);

CREATE TABLE conversation_turns (
  id INTEGER PRIMARY KEY,
  conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('user','tutor')),
  text TEXT NOT NULL,
  meta TEXT,                         -- tutor_turn JSON (correction, unknown_terms…)
  created_at TEXT NOT NULL,
  UNIQUE (conversation_id, seq)
);

CREATE TABLE learning_observations (
  id INTEGER PRIMARY KEY,
  conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('unknown_word','unknown_phrase','pronunciation','grammar','useful_sentence')),
  text TEXT NOT NULL,
  detail TEXT,                       -- JSON: explanation / original+corrected / heard+target
  saved_item_id INTEGER REFERENCES vocab_items(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL
);
```

Relationships:
- `Article ─< VocabContext >─ VocabItem`
- `Article ─< Conversation ─< ConversationTurn`
- `Conversation ─< LearningObservation ─(saved)→ VocabItem`

---

## 14. IPC surface (Tauri commands & events)

Commands return `Result<T, AppError>`. `AppError` serializes to `{ code, message }`. TypeScript types are kept in `src/lib/api.ts`, in step with the Rust `serde` structs. Codegen via `specta` is optional.

| Phase | Commands |
|---|---|
| P1 | `get_mode`, `set_mode`, `get_settings`, `update_settings`, `list_topics`, `upsert_topic`, `delete_topic`, `list_feeds`, `upsert_feed`, `delete_feed`, `test_feed`, `refresh_now`, `get_today_pick`, `list_articles(filter, cursor)`, `get_article`, `record_interaction`, `set_widget_style`, `complete_onboarding` |
| P2 | `fetch_article_html`, `save_article_body`, `add_vocab_item`, `update_vocab_item`, `delete_vocab_item`, `list_vocab(filter)`, `get_vocab_item`, `due_count`, `start_quiz`, `grade_quiz_item`, `finish_quiz`, `export_vocab_csv` |
| P3 | `ai_status`, `list_models`, `download_model`, `cancel_download`, `set_active_model`, `get_derivative(article_id, kind)`, `define_term`, `ask_about_article` |
| P4 | `start_voice_session`, `start_recording`, `stop_recording`, `send_text_turn` (typed fallback), `end_voice_session`, `get_session_review`, `apply_session_review`, `list_conversations` |

| Event | Payload |
|---|---|
| `news://updated` | `{ new_count }` |
| `pick://changed` | `DailyPick` |
| `mode://changed` | `"standard" \| "hibernate"` |
| `article://needs-body` | `{ article_id }` |
| `ai://status` | `{ component: "llm" \| "stt", state, progress? }` |
| `ai://download` | `{ model_id, bytes, total }` |
| `voice://transcript` | `{ text, final }` |
| `voice://tutor` | `{ delta \| sentence, done }` |

---

## 15. Privacy & storage

- Bundle id: `com.techenglish.app` (placeholder; see §21).
- Data: `~/Library/Application Support/com.techenglish.app/app.db`
- Models: `~/Library/Application Support/com.techenglish.app/models/`
- Logs: `~/Library/Logs/com.techenglish.app/` (daily rotation, 5 files kept). Logs MUST NOT contain transcripts or article bodies.
- Outbound network is limited to: configured feeds, the HN API, article page fetches, and model downloads. **No telemetry.**
- Sidecars bind to `127.0.0.1` only and require a random per-launch API key.
- Raw audio: memory only; discarded after transcription (§12.1).
- Settings → Data:
  - delete conversation history
  - delete Word Book
  - reset news history
  - "Open data folder"

---

## 16. Performance budgets (Mac mini M1, 16 GB)

| Scenario | Budget |
|---|---|
| Cold start → widget visible | ≤ 2 s |
| Idle, no model loaded (either mode) — total RSS of app + WebKit processes | ≤ 250 MB |
| Idle average CPU over 10 min (excluding fetch bursts) | ≤ 0.5 % |
| One fetch cycle, ~35 feeds (the default set) | ≤ 12 s wall time; ≤ 1 CPU-second |
| LLM loaded (4B Q4) | + ≤ 3.5 GB |
| Voice: end of speech → first tutor audio | p50 ≤ 3 s, p90 ≤ 5 s |
| Reader: B1 summary for a 1,500-word article (4B) | first text ≤ 4 s; complete ≤ 20 s |
| DB size after 90 days of normal use | ≤ 200 MB |

Each phase's acceptance includes recording the measured numbers in `docs/perf.md`.

---

## 17. Project structure

```
tech-and-english/
├── docs/
│   ├── SPEC.md                  # this file
│   └── perf.md                  # measured budgets per phase
├── src/                         # React + TypeScript (Vite)
│   ├── app/                     # routes, layout, providers
│   ├── windows/
│   │   ├── widget/              # widget window entry
│   │   └── main/                # main window entry
│   ├── pages/                   # Today, Explore, Reader, WordBook, Practice, Talk, Settings
│   ├── components/
│   ├── features/                # reader/, vocab/, quiz/, talk/ (feature-local hooks + UI)
│   ├── lib/
│   │   ├── api.ts               # typed invoke() wrappers + event listeners
│   │   ├── tts.ts               # TtsProvider (speechSynthesis)
│   │   └── readability.ts       # @mozilla/readability wrapper
│   └── stores/                  # Zustand stores
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json          # two windows, tray, bundle id, Info.plist entries
│   ├── capabilities/            # Tauri 2 permission sets
│   ├── migrations/              # 0001_init.sql, 0002_vocab.sql, …
│   ├── resources/               # models.json, common-words.txt, default feeds/topics
│   └── src/
│       ├── main.rs / lib.rs     # builder, plugins, tray, windows, state
│       ├── commands/            # thin IPC layer, one file per area
│       ├── db/                  # pool, migrations, repositories
│       ├── news/                # source.rs, rss.rs, hackernews.rs, normalize.rs, dedupe.rs,
│       │                        # topics.rs, ranking.rs, pick.rs, retention.rs
│       ├── scheduler.rs         # 60-s tick loop, Clock trait
│       ├── notify.rs            # notification rules + sending
│       ├── mode.rs              # ModeManager
│       ├── learning/            # vocab.rs, srs.rs (fsrs), quiz.rs, difficulty.rs
│       ├── ai/                  # provider.rs, llama.rs, manager.rs, models.rs, prompts/
│       └── voice/               # capture.rs (cpal), stt.rs, session.rs, intents.rs
├── package.json
└── README.md
```

**Frontend libraries:**
- React
- TypeScript
- Vite
- Zustand
- React Router
- CSS Modules with CSS variables (theme)
- `@mozilla/readability` (P2)
- Vitest

**Rust crates:**
- `tauri` 2, with plugins: `notification`, `autostart`, `single-instance`, `positioner`, `opener`. There is no `shell` plugin: sidecars are started with `tokio::process` (see docs/dev/README.md).
- `tokio`, `reqwest` (rustls), `feed-rs`, `rusqlite` (bundled), `serde`/`serde_json`, `chrono`, `url`, `tracing`, `thiserror`
- `fsrs` (P2)
- `cpal`, `rubato`, `hound` (P4)

---

## 18. Delivery plan & acceptance criteria

### P1 — Shell & News (no AI)

**Build:**
- §5: windows, tray, onboarding
- §6: modes, without AI parts
- §7: the whole news engine
- §13: P1 tables
- §14: P1 commands
- Pages: Today, Explore, and Settings (topics, feeds, notifications, mode)

**Acceptance:**
1. `npm run tauri dev` launches. The widget appears top-right and the tray icon has the §5.3 menu.
2. Closing windows hides them. Only Quit exits. A second launch focuses the running instance.
3. Onboarding seeds topics and feeds, and the first fetch fills Explore within 30 s.
4. Topic and feed create/edit/delete works. "Test feed" validates a URL and rejects non-feeds with a clear error.
5. Feeds are fetched at the mode's interval (checked with the logs or a fake clock). After the Mac sleeps for > interval, a fetch happens within 2 min of wake.
6. A story that appears on both HN and an RSS feed shows once, with HN points attached.
7. Explore is sorted by score, and the score breakdown is visible.
8. A daily pick is created at or after `pick_time`, shown in the widget with source, topic and the "why" text. Difficulty and reading time arrive in P2. Exactly one daily-pick notification is sent (verified on a built bundle).
9. High-interest notifications follow the threshold, the daily cap, the 90-min spacing, quiet hours and first-fetch suppression.
10. "Not interested" hides the article and lowers the topic/source affinity. The change is visible in the breakdown of related articles.
11. Switching to Hibernate changes the intervals and the badge, and the mode persists after a restart.
12. The Launch at login toggle adds/removes the app from Login Items.
13. `cargo test` covers: URL normalization, title key, dedupe, topic matching (including exclusions), ranking math, pick eligibility and fallback, notification rules, and retention. Everything passes.
14. The idle RSS and CPU budgets (§16) are measured and recorded in `docs/perf.md`.

### P2 — Reader, Word Book, Practice

**Build:** §8.1, §8.2, the Original reader mode, the §8.4 P2 popup, §9, §10, TTS for 🔊 and for Listen (title + description), and the P2 tables.

**Acceptance:**
1. Opening an article shows the clean extracted text. Failed and paywalled articles show a fallback with an "Open in browser" button.
2. The difficulty and reading time appear on the pick card and in Explore.
3. Selecting text → Add to Word Book saves the item with its sentence context. Re-adding it adds a context instead of a duplicate.
4. A 10-item quiz follows the §10.2 selection order. The buttons update FSRS fields and counters. The result score matches the formula and is stored.
5. The due count in the widget updates after a quiz.
6. Items without a meaning are excluded from quizzes. CSV export works.
7. Unit tests cover quiz selection buckets, FSRS grade mapping, status transitions and difficulty thresholds.

### P3 — Local LLM

**Build:** §11 (provider, sidecar manager, model catalog/downloader, the model benchmark spike), the B1 Summary and Easy English modes, Explain / Ask AI in the popup, the optional LLM "why" text, auto-fill of pending meanings, and the P3 tables.

**Acceptance:**
1. The model downloader shows the size and license, can resume, and checks the sha256.
2. The first summary request loads the LLM with visible progress. The summary streams in, and a cached copy loads instantly the second time.
3. The LLM process exits after the idle timeout and on Hibernate. There are no orphan `llama-server` processes after Quit or after a crash-restart.
4. In Hibernate, AI buttons show "Switch to Standard" and never start the sidecar.
5. `define_term` returns valid schema JSON. Explain → Add saves all fields.
6. The benchmark results and the chosen default model are recorded in `docs/perf.md`.

### P4 — Voice tutor

**Build:** §12 in full, the P4 tables, and the Talk page (setup, live transcript, history, review).

**Acceptance:**
1. The mic permission prompt appears on first use. The denied state is handled.
2. Push-to-talk → transcript → spoken reply works end to end, within the §16 latency budget.
3. The level presets change the speech rate and the reply style. Speed changes by voice ("slower please") work without an LLM call.
4. "Yesterday I deploy the application." triggers a correction, then a repeat request. A correct repeat returns to the discussion. The discussion continues on the same topic.
5. Asking "What does *deployment* mean?" gets a simpler explanation, then the tutor goes back to the previous question.
6. The pronunciation drill works as described in §12.6.
7. The end-of-session review lists the observations with preselection. Only the selected items become `vocab_items`, linked to the conversation.
8. No audio files exist on disk after a session (with default settings).

### P5 — Packaging & polish

- A signed and notarized `.app`/`.dmg` (needs an Apple Developer ID). Unsigned local builds are fine before this.
- Sidecars bundled as `externalBin`.
- A first-run model setup screen.
- Model license review.
- An accessibility pass (keyboard navigation, VoiceOver labels).
- An error-reporting UI (local only).

---

## 19. Testing strategy

- **A `Clock` trait** is injected into the scheduler, pick, notification, retention and SRS code. Tests use a fake clock. **No test sleeps.**
- **Fixtures** live in `src-tauri/tests/fixtures/`: RSS 2.0, Atom, a broken feed, HN JSON (top ids + items), and HTML pages (normal and paywalled).
- **An HTTP mock** (e.g. `wiremock`) for the feed and HN fetch tests.
- **A `MockProvider`** for LLM-dependent logic. A small **golden-prompt suite** (P3/P4) runs against the real local model on demand (`cargo test -- --ignored`). It checks JSON validity and the correction behaviour on about 30 learner sentences.
- **Frontend:** Vitest for pure logic (intent matching, quiz UI state, readability wrapper).
- **Manual QA checklist** per phase, taken from its acceptance list, plus the perf measurements.

---

## 20. Out of scope for v1 (later)

- Browser extensions (Safari, Chrome)
- Mobile companion app
- Anki export
- Reading-level detection per user
- A personalized difficulty model
- **Real pronunciation scoring** (phoneme-level)
- Semantic dedupe and embedding-based ranking
- Conversation search
- Weekly English / tech reports
- Cloud LLM providers (the interface exists; no implementations yet)
- Sync between devices
- Windows / Linux

---

## 21. Open questions (defaults assumed until answered)

| # | Question | Default used |
|---|---|---|
| Q1 | The app name and bundle id | "Tech English", `com.techenglish.app` |
| Q2 | The daily pick time | 08:00 local |
| Q3 | Show the Dock icon? | **Decided 2026-09-29:** hidden (menu-bar app); a setting can show it |
| Q4 | Are the default feeds (§7.2) OK? | **Decided 2026-09-29:** yes, plus the AI and data-engineering feeds |
| Q5 | How long to keep conversation transcripts | 90 days |
| Q6 | Will an Apple Developer ID be available for signing (P5)? | Unsigned local builds until then |
| Q7 | App license and distribution: personal use only, or shared or published? | **Decided 2026-09-29:** personal use only. P5 uses ad-hoc signing (§2.2), and the license review is informational. |
