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
