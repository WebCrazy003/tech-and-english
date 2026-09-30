# Performance measurements

Machine: Mac mini M1, 16 GB, macOS 14.7. Budgets from SPEC §16.

## P1 — v0.1.0 (2026-09-29)

These are **release builds**, measured with `scripts/measure-idle.sh`. Each run uses a fresh data folder with all 13 topics and 33 feeds. After a 2-minute warm-up (which covers the first fetch), the script sums the app process and its WebKit processes every 10 s for 10 minutes. CPU is the average over the window, computed from the change in CPU time.

| Run | Window | Avg memory | Max memory | Avg CPU | Articles |
|---|---|---|---|---|---|
| Standard (release) | 10 min | 130.7 MB | 164.3 MB | 0.37 % | 320 |
| Hibernate (release) | 10 min | 130.5 MB | 142.7 MB | 0.03 % | 315 |

| Budget | Target | Result |
|---|---|---|
| Idle memory (app + WebKit) | ≤ 250 MB | ✅ about 131 MB |
| Idle CPU (10 min) | ≤ 0.5 % | ✅ 0.37 % Standard, 0.03 % Hibernate |
| One fetch cycle (33 feeds) | ≤ 12 s | ❌ about 28–30 s wall time (`full_pipeline` live test); CPU time is under 1 s |
| App bundle size | – | 9.7 MB |

Notes:
- **Main window on demand.** An earlier run, with the main window created at startup and kept hidden, used 215 MB on average (max 299 MB). The main window is now created when first opened and destroyed when closed, which saves one WebKit process.
- **Fetch wall time.** The time is spent waiting on the network, mostly for a few slow feeds and the Hacker News item requests (up to 110 small requests). It runs in the background every 20 minutes, so users don't notice it. The ≤ 12 s budget was a guess made before measuring. Options if it matters later: more concurrency (4 → 8 feeds), or fewer HN item fetches.

## P2 — model benchmark (2026-09-29)

Engine: llama.cpp b11256 (official macOS arm64 build), `-ngl 99`. All models Q4_K_M from unsloth, Apache-2.0.

**Speed** (`llama-bench`):

| Model | File | pp512 | pp2048 | tg128 |
|---|---|---|---|---|
| Qwen3.5 4B | 2.74 GB | 209 t/s | 207 t/s | 17.4 t/s |
| Qwen3 4B Instruct 2507 | 2.50 GB | 236 t/s | 222 t/s | 21.4 t/s |
| Gemma 4 E4B | 4.98 GB | 211 t/s | 205 t/s | 17.1 t/s |

**App-level** (`cargo test --release --test live_ai`: 5-paragraph DuckDB article, B1 summary + 3 chat turns, warm file cache):

| Model | Start | Summary: first text / done | Chat: first answer / follow-ups (first text) | Quality notes |
|---|---|---|---|---|
| **Qwen3.5 4B** (default) | 4.3 s | 2.1 s / 17.6 s | 3.5 s / 0.7 s | Plain B1 prose, accurate, good key words. Marks the general answer "(not from the article)" correctly. |
| Qwen3 4B Instruct 2507 | 3.8 s | 1.8 s / 13.7 s | 2.7 s / 0.2 s | Fastest. Adds headings ("Main points:"), writes "(from the article)" / "(not from the article)" in the wrong places, and adds facts that are not in the article (Iceberg "versioning"). |
| Gemma 4 E4B | 7.1 s | 2.4 s / 17.6 s | 3.5 s / 0.4 s | Very learner-friendly ("data questions (analytical queries)"), but marks an article-based answer "(not from the article)". Double the memory. |

**Decision:** Qwen3.5 4B stays the default (`models.json` `default: true`). The other two remain in the catalog as options in Settings › AI.

**Budgets (SPEC §16, P2 rows):** B1 summary first time: first text 2.1 s (≤ 12 s ✅), complete 17.6 s (≤ 30 s ✅). Chat: first answer 3.5 s (≤ 6 s ✅), follow-up 0.7 s (≤ 2 s ✅).

## P3 — learning & sources (2026-09-30)

Debug-build tests on the Mac mini M1 (network time dominates the live numbers).

| What | Result |
|---|---|
| Migration 0003 on a copy of the real database (1,065 articles) | 10 ms |
| One-time learning-score backfill of those 1,065 articles (+ rematch + rescore) | 2.5 s, in the background at start |
| One rescore, stories + lessons (1,065 articles, 60-day lesson window) | 68 ms |
| First fetch cycle, 42 feeds (learning feeds keep up to 60 days) | 16 s wall time, ~460 articles |
| "Find feed" per example URL (live, 13 URLs) | 1.1–10.4 s (median about 2 s) |

Idle memory and CPU are unchanged in kind: P3 adds no background process, only one small query per minute (lesson check) and a few SQL rows per article.

## P4 — words (2026-09-30)

| What | Result | Budget |
|---|---|---|
| macOS dictionary lookup, first (cold) | 238 ms | ≤ 300 ms ✅ |
| macOS dictionary lookup, warm (8 words) | 2 ms each | |
| "Explain simply" on Qwen3.5 4B (model already loaded) | 11–15 s per term, valid JSON 8/8 | |
| Release binary (all of P4, incl. the fsrs crate) | 10.9 → 11.3 MB (+0.4 MB) | fsrs ≤ 5 MB ✅ |
| Build of fsrs and its new dependencies | 40 s | ≤ 60 s ✅ |
| Background meaning fill | ≤ 10 items per minute; dictionary only unless the model is loaded | never loads the model ✅ |
