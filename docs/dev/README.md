# Development specs — index & shared conventions

These documents turn [../SPEC.md](../SPEC.md) (the **what**) into ordered, testable work (the **how**).

- **SPEC.md is the source of truth for behaviour.** If a dev spec disagrees with it, stop and fix one of the two documents before writing code.
- Every task in a phase spec lists its **Depends on**, **Deliverables** and **Done when** items.
- **A task is not done until its tests exist and pass.**

| Phase | Document | Result | Status |
|---|---|---|---|
| P1 | [P1-shell-and-news.md](P1-shell-and-news.md) | Menu-bar app + widget that fetches, ranks and picks one story a day. No AI. | ✅ v0.1.0 |
| P2 | [P2-reader-and-ai-chat.md](P2-reader-and-ai-chat.md) | Read stories in the app, with a local AI chat panel (summarize, ask questions) | ✅ v0.2.0 |
| P3 | [P3-learning-and-sources.md](P3-learning-and-sources.md) | "Today's lesson" (learning materials) + add a source from an example URL | ✅ v0.3.0 |
| P4 | [P4-words.md](P4-words.md) | Word popup (macOS dictionary + AI), Word Book, 10-item FSRS quiz | ✅ v0.4.0 |
| P5 | [P5-voice-tutor.md](P5-voice-tutor.md) | Push-to-talk English tutor with corrections and session review | |
| P6 | [P6-packaging.md](P6-packaging.md) | Self-contained `.app` with bundled sidecars (ad-hoc signed) | |

Each phase depends on the ones before it:

```
P1 ──► P2 ──► P3 ──► P4 ──► P5 ──► P6
```

> **The order changed in v0.3 of the spec** (SPEC C13–C17), after P1, at the user's request: AI chat first, then learning materials, then words. P3 doesn't need the AI, so it could be built in parallel with P2 if wanted.

**Migration numbers:** 0001 (P1) · 0002 reader + AI (P2) · 0003 learning (P3) · 0004 vocab (P4) · 0005 voice (P5).

---

## 1. Pinned versions (checked 2026-09-29)

At the start of each phase, re-check versions and **read the current docs** for any Tauri plugin API you use. Use the `find-docs` skill or the official docs; plugin APIs change between minor versions.

### Toolchain

| Tool | Version |
|---|---|
| Rust | 1.97 (edition 2024) |
| Node | 26 |
| npm | 11 |
| Tauri CLI | 2.12 |

### npm

| Package | Version | Phase |
|---|---|---|
| `@tauri-apps/api`, `@tauri-apps/cli` | 2.12.x | P1 |
| `@tauri-apps/plugin-notification` | 2.5.x | P1 |
| `@tauri-apps/plugin-autostart` | 2.6.x | P1 |
| `@tauri-apps/plugin-opener` | 2.6.x | P1 |
| `react`, `react-dom` | 19.3.x | P1 |
| `react-router` | 8.4.x | P1 |
| `zustand` | 5.0.x | P1 |
| `vite` | 8.3.x | P1 |
| `typescript` | 6.0.x (the Tauri template pins 6; 7 is not used yet) | P1 |
| `vitest` | 5.0.x | P1 |
| `eslint` | 10.x | P1 |
| `@mozilla/readability` | 0.6.x | P2 |
| `dompurify` | 3.4.x (ships its own types) | P2 |

### Rust crates

| Crate | Version | Phase |
|---|---|---|
| `tauri` | 2.12 (features: `tray-icon`, `macos-private-api`) | P1 |
| `tauri-build` | 2.7 | P1 |
| `tauri-plugin-notification` | 2.5 | P1 |
| `tauri-plugin-autostart` | 2.6 | P1 |
| `tauri-plugin-single-instance` | 2.5 | P1 |
| `tauri-plugin-opener` | 2.6 | P1 |
| `tokio` | 1.53 | P1 |
| `reqwest` | 0.13 (native-tls, http2, gzip, json) | P1 |
| `feed-rs` | 3.0 | P1 |
| `rusqlite` | 0.40 (feature `bundled`) | P1 |
| `serde`, `serde_json` | 1.0 | P1 |
| `chrono` | 0.4 (feature `serde`) | P1 |
| `url` | 2.5 | P1 |
| `regex` | 1.13 | P1 |
| `futures` | 0.3 | P1 |
| `async-trait` | 0.1 | P1 |
| `thiserror` | 2.0 | P1 |
| `tracing`, `tracing-subscriber`, `tracing-appender` | 0.1 / 0.3 / 0.2 | P1 |
| `wiremock` (dev) | 0.6 | P1 |
| `tempfile` (dev) | 3.27 | P1 |
| `fsrs` | 6.6 | P4 |
| `sysinfo` | 0.39 | P2 |
| `sha2` | 0.11 | P2 |
| `cpal` | 0.18 | P5 |
| `rubato` | 5.0 | P5 |
| `hound` | 3.5 | P5 |

### Homebrew (development only)

| Formula | Version | Phase |
|---|---|---|
| `llama.cpp` | 0.5.0 | P2 |
| `whisper.cpp` | 1.9.4 | P5 |

> **Changes from SPEC §17:**
> - `tauri-plugin-positioner` is **not** used. The widget is placed with `Monitor::work_area()`.
> - `reqwest` uses **native-tls** (macOS Security framework), not rustls. With rustls, the TLS handshake to some Cloudflare-hosted feeds (e.g. Substack custom domains) hung until the connect timeout. Found in the P1 live feed check.
> - `tauri-plugin-shell` is **not** used. Sidecars are started with `tokio::process::Command` using a resolved binary path (P2 §8.3). This keeps one code path for dev and bundled builds.

---

## 2. Repository conventions

### 2.1 Layout & layering (Rust)

```
commands/*   thin #[tauri::command] fns: parse args → call service → map errors
   ↓
services     news::NewsService, learning::*, ai::AiManager, voice::VoiceEngine
   ↓
db::repo::*  plain functions taking &rusqlite::Connection (sync, easy to test)
```

- Services MUST NOT depend on `tauri::AppHandle` directly. They depend on these traits:
  - `EventSink` (emit events)
  - `Notifier` (send notifications)
  - `Clock` (time)
  - `HttpClient` (reqwest wrapper)

  The real implementations wrap Tauri or reqwest. Tests use fakes.
- `AppState` holds the services as `Arc<…>` and is registered with `app.manage(...)`.

### 2.2 Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found: {0}")]         NotFound(String),
    #[error("invalid input: {0}")]     Invalid(String),
    #[error("unavailable in hibernate mode")] Hibernating,
    #[error("network: {0}")]           Network(String),
    #[error("database: {0}")]          Db(#[from] rusqlite::Error),
    #[error("ai: {0}")]                Ai(String),
    #[error("internal: {0}")]          Internal(String),
}
// Serialize as { "code": "not_found" | "invalid" | "hibernating" | ..., "message": "..." }
```

- The frontend receives `ApiError { code, message }` and shows `message` in a toast.
- `code === "hibernating"` shows a "Switch to Standard" action.

### 2.3 Time

- `trait Clock: Send + Sync { fn now(&self) -> DateTime<Utc>; fn local_offset(&self) -> FixedOffset; }`
- `SystemClock` is the real one; `FakeClock` (settable/advanceable) is for tests.
- **Never call `Utc::now()` outside `SystemClock`.** CI enforces this with a grep in `scripts/check.sh`.
- Store times as RFC 3339 UTC strings (`to_rfc3339_opts(SecondsFormat::Secs, true)`).

### 2.4 Database

- `Db` wraps `Arc<Mutex<rusqlite::Connection>>`.
  - `db.call(|c| ...)` runs the closure on `tokio::task::spawn_blocking`.
  - `db.tx(|tx| ...)` does the same inside a transaction.
- The PRAGMAs `journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000` are set when the connection opens.
- Migrations live in `src-tauri/migrations/NNNN_name.sql` and are embedded with `include_str!`. They run in order inside a transaction and set `PRAGMA user_version = NNNN`. **Never edit a shipped migration; add a new one.**
- Tests use `Db::open_in_memory()`, which runs all migrations.

### 2.5 Settings

- Settings are one typed `Settings` struct with `#[serde(default)]`. Defaults live in code.
- They are stored as a single JSON row: `settings(key='app', value=<json>)`.
- `update_settings` takes a **partial** JSON patch, merges it, validates ranges and emits `settings://changed`.

### 2.6 Frontend

- TypeScript `strict`.
- `src/lib/api.ts` is the **only** file that calls `invoke()` or `listen()`. It holds the hand-written types that mirror the Rust `serde` structs (snake_case → camelCase via `#[serde(rename_all = "camelCase")]` on every DTO).
- State lives in Zustand stores in `src/stores/`. Components are function components; styles are CSS Modules plus CSS variables in `src/styles/theme.css` (light/dark via `prefers-color-scheme`).
- Two Vite entry points, **`index.html`** (main window) and **`widget.html`** (widget), share `src/`.

### 2.7 Logging

- `tracing` writes to stdout in dev, and to `~/Library/Logs/com.techenglish.app/app.log` with daily rotation, keeping 5 files.
- Logs MUST NOT contain transcripts, article bodies or LLM outputs. IDs, counts and timings only.

### 2.8 Git

- Run `git init` in P1-T0.
- Each phase uses a branch `p<N>/<short-name>`. Commits follow Conventional Commits.
- `main` must always build and pass `scripts/check.sh`.

### 2.9 Quality gate — `scripts/check.sh`

Run before each commit:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run typecheck && npm run lint && npm test -- --run
! grep -rn "Utc::now()" src-tauri/src --include=*.rs | grep -v "clock.rs"
```

---

## 3. Definition of Done (every task)

1. The code is written and follows the layering above.
2. Unit tests for the new logic exist and pass. Any time-based logic is tested with `FakeClock`.
3. `scripts/check.sh` is green.
4. The phase spec's "Done when" items for the task are checked.
5. If behaviour changed, SPEC.md or the phase spec is updated in the same commit.

## 4. Definition of Done (every phase)

1. Every task is done.
2. Every item in the phase's **Acceptance** list (SPEC §18) has been verified manually. The results are recorded in the phase's QA checklist.
3. The perf numbers for the phase are measured and appended to `docs/perf.md`.
4. The phase branch is merged into `main` and tagged `v0.<N>.0`.
