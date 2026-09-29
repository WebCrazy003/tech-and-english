# P1 — App shell & News engine (dev spec)

- **Goal:** a menu-bar app with a floating widget. It fetches RSS and Hacker News, ranks stories against the user's topics, picks one story a day and sends notifications. **No AI, no reader, no vocabulary.**
- **SPEC sections:** §5, §6 (non-AI parts), §7, §13 (P1 tables), §14 (P1), §16, §18 P1.
- **Branch:** `p1/shell-and-news` → tag `v0.1.0`

## 0. Scope

**In scope:**
- Tauri app with two windows (widget + main), tray menu, single instance, autostart, hide-on-close, no Dock icon.
- SQLite with migrations; typed settings.
- Topics & feeds CRUD; first-run onboarding with seed data.
- RSS/Atom + Hacker News fetching, normalization, dedupe/merge, topic matching, ranking, affinities.
- Scheduler (60 s tick), Standard/Hibernate intervals, sleep/wake catch-up.
- Daily pick (template "why"), daily-pick + high-interest notifications with all limits.
- Retention job; "skipped" marking.
- Pages: Today, Explore, Settings (General, Topics, Feeds, Notifications), Onboarding.

**Out of scope (later phases):**
- Body extraction, paywall detection, difficulty, reading time → P2.
- Reader → P2. **In P1, "Read" opens the article in the default browser.**
- Listen / Easy Summary / Discuss buttons → hidden until P2–P4.

> **Adjustment to SPEC §7.8 for P1:** selection steps 1–3 (extraction, paywall retry, difficulty) are skipped. `PickService` still has a `post_select` hook that P2 fills in.
>
> **Adjustment to SPEC §18 P1 item 8:** difficulty and reading time are **not** shown in P1. They are P2 acceptance item 2.

---

## 1. Dependencies

### `src-tauri/Cargo.toml`

```toml
[package]
name = "tech-english"
version = "0.1.0"
edition = "2024"

[lib]
name = "tech_english_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2.7", features = [] }

[dependencies]
tauri = { version = "2.12", features = ["tray-icon", "macos-private-api"] }
tauri-plugin-notification = "2.5"
tauri-plugin-autostart = "2.6"
tauri-plugin-single-instance = "2.5"
tauri-plugin-opener = "2.6"
tokio = { version = "1.53", features = ["rt-multi-thread", "macros", "time", "sync"] }
reqwest = { version = "0.13", default-features = false, features = ["native-tls", "http2", "gzip", "json"] }
feed-rs = "3.0"
rusqlite = { version = "0.40", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
url = "2.5"
regex = "1.13"
futures = "0.3"
async-trait = "0.1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"

[dev-dependencies]
wiremock = "0.6"
tempfile = "3.27"
tokio = { version = "1.53", features = ["test-util"] }
```

### `package.json` (scripts + deps)

```json
{
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "tauri": "tauri",
    "typecheck": "tsc -b --noEmit",
    "lint": "eslint src",
    "test": "vitest"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.12.0",
    "@tauri-apps/plugin-notification": "^2.5.0",
    "@tauri-apps/plugin-autostart": "^2.6.0",
    "@tauri-apps/plugin-opener": "^2.6.0",
    "react": "^19.3.0",
    "react-dom": "^19.3.0",
    "react-router": "^8.4.0",
    "zustand": "^5.0.15"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.12.0",
    "typescript": "^7.0.2",
    "vite": "^8.3.1",
    "@vitejs/plugin-react": "latest",
    "vitest": "^5.0.2",
    "eslint": "^10.11.0"
  }
}
```

---

## 2. File plan

```
index.html                     # main window entry
widget.html                    # widget window entry
vite.config.ts                 # multi-page: { main: index.html, widget: widget.html }
src/
  main.tsx                     # main window root (router)
  widget.tsx                   # widget root
  lib/api.ts                   # ALL invoke/listen + DTO types
  lib/format.ts                # relative time, score formatting
  stores/{mode,pick,articles,topics,feeds,settings}.ts
  styles/theme.css
  windows/widget/Widget.tsx, PickCard.tsx, Pill.tsx, WidgetHeader.tsx
  pages/Today.tsx, Explore.tsx, Onboarding.tsx
  pages/settings/{General,Topics,Feeds,Notifications}.tsx
  components/{ScoreChip,TopicChip,ArticleRow,Toast,ConfirmDialog,KeywordInput}.tsx
src-tauri/
  tauri.conf.json
  capabilities/default.json
  migrations/0001_init.sql
  resources/default_topics.json
  resources/default_feeds.json
  icons/tray.png, tray@2x.png  # monochrome template icons
  src/
    main.rs                    # calls tech_english_lib::run()
    lib.rs                     # builder, plugins, setup, tray, window events, state
    error.rs                   # AppError
    clock.rs                   # Clock, SystemClock, FakeClock
    events.rs                  # EventSink trait + TauriEventSink + RecordingEventSink (tests)
    http.rs                    # HttpClient trait + ReqwestClient
    logging.rs
    settings.rs                # Settings struct, defaults, patch/validate
    state.rs                   # AppState
    db/mod.rs                  # Db, open, migrations runner
    db/repo/{topics,feeds,articles,interactions,affinities,picks,notifications,app_state}.rs
    news/mod.rs                # NewsService (orchestrates below)
    news/model.rs              # RawItem, Article DTOs, ScoreBreakdown …
    news/source.rs             # FeedSource trait, FetchCtx, FetchResult
    news/rss.rs
    news/hackernews.rs
    news/normalize.rs          # normalize_url, title_key, jaccard
    news/dedupe.rs
    news/ingest.rs             # the per-item transaction pipeline
    news/topics.rs             # CompiledTopic, match_topics
    news/ranking.rs            # pure scoring
    news/affinity.rs
    news/pick.rs               # PickService
    news/why.rs                # template "why" text
    news/retention.rs
    notify.rs                  # policy (pure) + Notifier trait + TauriNotifier
    mode.rs                    # ModeManager
    scheduler.rs               # tick loop
    seed.rs                    # onboarding seed
    commands/{mode,settings,topics,feeds,articles,pick,widget,onboarding}.rs
  tests/fixtures/
    rss2_basic.xml, atom_basic.xml, rss_broken.xml, rss_old_items.xml,
    hn_topstories.json, hn_item_story.json, hn_item_ask.json, hn_item_dead.json
scripts/check.sh
scripts/measure-idle.sh
docs/perf.md
```

---

## 3. Core types (Rust)

```rust
// settings.rs
#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub onboarding_done: bool,                 // false
    pub pick_time: String,                     // "08:00"
    pub fetch_interval_standard_min: u32,      // 20   (15..=30)
    pub fetch_interval_hibernate_min: u32,     // 45   (30..=60)
    pub ingest_max_age_days: u32,              // 7    (1..=30)
    pub hn_include_new: bool,                  // false
    pub ranking_weights: RankingWeights,       // 0.35/0.20/0.15/0.10/0.10/0.10 (must sum to 1 ± 0.001)
    pub notify_daily_pick: bool,               // true
    pub notify_high_interest: bool,            // true
    pub notify_threshold: u8,                  // 85
    pub notify_max_per_day: u8,                // 3    (0..=5)
    pub notify_min_gap_min: u32,               // 90
    pub quiet_hours: Option<(String, String)>, // Some(("22:00","08:00"))
    pub widget: WidgetSettings,                // { style: "card"|"pill"|"hidden", alwaysOnTop: true, position: Option<(i32,i32)> }
    pub show_dock_icon: bool,                  // false
}

// news/model.rs
pub struct RawItem {             // what a FeedSource returns, before normalization
    pub url: String,
    pub title: String,
    pub description: Option<String>,   // HTML stripped, ≤ 1000 chars
    pub author: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub hn: Option<HnStats>,           // { id, points, comments }
}
pub struct FetchResult { pub items: Vec<RawItem>, pub etag: Option<String>, pub last_modified: Option<String>, pub not_modified: bool }

#[async_trait]
pub trait FeedSource: Send + Sync {
    async fn fetch(&self, ctx: &FetchCtx) -> Result<FetchResult, AppError>;
}
pub struct FetchCtx<'a> { pub http: &'a dyn HttpClient, pub etag: Option<String>, pub last_modified: Option<String>, pub now: DateTime<Utc> }

#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct ScoreBreakdown {
    pub topic_relevance: f64, pub freshness: f64, pub popularity: f64,
    pub source_preference: f64, pub novelty: f64, pub user_history: f64,
    pub total: f64,                         // 0..100
}
```

---

## 4. Algorithms (implementation notes)

### 4.1 `normalize.rs`

- `normalize_url(&str) -> Result<String>`: SPEC §7.4, steps 1–6. Only `http` and `https` are accepted.
- `title_key(title, source_name) -> String`:
  1. Lowercase.
  2. Remove a trailing ` - <anything>` or ` | <anything>` **only if** the suffix is ≤ 40 chars and fuzzy-equals `source_name` (or is on a small list of known publisher names).
  3. Replace non-alphanumerics with spaces and collapse the spaces.
- `word_set(title_key) -> HashSet<&str>`: drop stop-words (`a an the of to in for on and or with is are`).
- `jaccard(a, b) -> f64`: returns 0 if either set is empty.

### 4.2 `ingest.rs` — per fetched item, in **one transaction per feed batch**

```
for raw in items:
  if raw.published_at < now - ingest_max_age_days: skip
  n = normalize_url(raw.url)            (invalid → skip, log at debug)
  key = title_key(raw.title, feed.name)
  existing = find by normalized_url
          ?? find by hn_id (if raw.hn)
          ?? find by title_key (discovered in last 72 h)
          ?? find by jaccard ≥ 0.8 among articles discovered in last 72 h
             (in-memory set loaded once per batch)
  if existing:
      merge: fill empty description/author/published_at; update hn stats if present (hn_checked_at = now);
             insert article_sources(article_id, feed_id) OR IGNORE
  else:
      insert article (discovered_at = now) + article_sources
      match topics → article_topics, primary_topic_id
  collect ids of new/updated articles
after all feeds in the cycle: rescore(articles discovered in last 72 h)
```

### 4.3 `topics.rs`

- `CompiledTopic` holds one regex per keyword: `(?i)(?:^|[^\p{L}\p{N}])` + `regex::escape(kw)` + `(?:$|[^\p{L}\p{N}])`.
  - Keywords containing symbols (e.g. `C++`, `.NET`) still work because they are escaped.
- `match_topics(topics, title, desc_plus_body500) -> Vec<(TopicId, f64)>` implements SPEC §7.5 exactly. Exclusions are checked against title + description.
- Topics are recompiled when they change. `NewsService` caches `Arc<Vec<CompiledTopic>>` behind an `RwLock`.
- **When a topic changes**, re-run matching for articles from the last 72 h, then rescore.

### 4.4 `ranking.rs` (pure)

```rust
pub struct RankInputs {
    pub relevance: f64, pub age_hours: f64,
    pub hn_points: Option<i64>, pub hn_comments: Option<i64>,
    pub source_weight: f64, pub max_recent_similarity: f64,
    pub topic_affinity: f64, pub source_affinity: f64,
}
pub fn score(i: &RankInputs, w: &RankingWeights) -> ScoreBreakdown
```

- The formulas are exactly those in SPEC §7.6.
- `age_hours` is clamped to ≥ 0 (feeds sometimes have future dates).
- `max_recent_similarity` = the maximum Jaccard against the titles of articles that were **picked or opened** in the last 7 days. Load them once per rescore.
- For RSS items with several sources, `source_weight` is the **max** over `article_sources`.

### 4.5 `affinity.rs`

- `apply(kind: InteractionKind, article) -> Vec<(AffinityKey, delta)>` uses the table in SPEC §7.7.
  - The topic delta is multiplied by the article's relevance for the primary topic.
  - The source delta goes to **every** feed in `article_sources`.
- Results are clamped to [−1, 1].
- `record_interaction` writes the interaction, applies the affinities, and for `not_interested` sets `hidden = 1`. All three happen in one transaction. It then rescores the last 72 h and emits `news://updated`.

### 4.6 `hackernews.rs`

- Base URL: `https://hacker-news.firebaseio.com/v0`. It is configurable for tests.
- Lists: `/topstories.json` (first 60), `/beststories.json` (first 30), `/showstories.json` (first 20), and optionally `/newstories.json` (first 30). The union of IDs is deduplicated.
- Items: `/item/{id}.json`, with up to 8 requests at a time. Skip if `deleted`, `dead`, `type != "story"`, or `title` is missing.
  - If there is no `url` → `https://news.ycombinator.com/item?id={id}`.
  - `published_at = time` (unix seconds).
  - `points = score`, `comments = descendants`.
- **Cache.** If the article with this `hn_id` exists and `hn_checked_at` is less than 1 h old, skip the item request. The ID still counts as "seen".
- An `hn` feed row has `url = "hn:top,best,show"`. The list set is parsed from the URL, and `hn_include_new` adds `new`.

### 4.7 `rss.rs`

- Uses conditional GET (`If-None-Match`, `If-Modified-Since`). `304` → `not_modified: true`.
- The body is parsed with `feed_rs::parser::parse`.
- For each entry:
  - **URL:** the first `link` with `rel` = `alternate` or no rel; otherwise `entry.id` if it is a URL.
  - **Title:** `entry.title.content`.
  - **Description:** `summary`, or else `content`. HTML is stripped with a simple tag-stripping function plus entity decoding; the result is truncated to 1000 chars.
  - **published_at:** `published` or `updated`.
- **Size limit:** stop reading after 5 MB → `AppError::Invalid("feed too large")`.
- **`test_feed(url)`:** fetches and parses without saving. Returns `{ title, itemCount, newestPublishedAt }`, or a readable error ("Not a feed (HTML page)", "HTTP 404", "Timed out").

### 4.8 `scheduler.rs`

```
loop every 60 s (tokio::time::interval, MissedTickBehavior::Skip):
  now = clock.now()
  1. due = feeds where enabled and (last_fetched_at is null
            or now - last_fetched_at >= interval(mode) * 2^min(failures, 5)  capped at 6 h)
     fetch due feeds (buffer_unordered(4)), 15 s timeout each; update feed status
     → ingest → rescore → emit news://updated { newCount }
     network error on ALL feeds in this tick → set retry_at = now + backoff(30,60,120 s)
  2. pick.ensure_today(now)               (§4.9)
  3. notify.maybe_high_interest(now)      (§4.10)
  4. once per local date: retention.run(now); pick.mark_yesterday_skipped(now)
```

- `refresh_now` triggers the same fetch step right away, ignoring intervals. It is guarded by a `tokio::sync::Mutex` so two fetch cycles never overlap.
- **Wake handling needs no special code.** On the first tick after wake, the elapsed time is greater than the interval, so the feeds are fetched.
- The tick is written as `async fn tick(&self, now)` so tests can call it directly with a `FakeClock`.

### 4.9 `pick.rs`

- `ensure_today(now)`:
  - local_date = `now` in the local offset.
  - If a `daily_picks` row already exists for that date, or local time < `pick_time`, do nothing.
  - Otherwise `select(now)`.
- `select(now)`:
  - Filter per SPEC §7.8 (the paywall condition is ignored in P1).
  - If nothing is found at a 48 h age limit, relax it to 72 h.
  - Take the highest score. Break ties by the newest `discovered_at`.
  - Call `post_select` (a no-op in P1).
  - Insert the `daily_picks` row with `why = why::template(...)`.
  - Emit `pick://changed`, then `notify.daily_pick(article)`.
- `mark_yesterday_skipped(now)`: if yesterday's pick has no `opened` interaction, record `skipped` (once).
- `why::template` joins up to 3 parts with ` · `:
  - `Matches your topics: A, B` (topics with relevance ≥ 0.3, highest first, max 2)
  - `{points} points on Hacker News` (if points ≥ 50)
  - `From a source you read often` (if source affinity ≥ 0.3)

### 4.10 `notify.rs`

The policy is a pure function:

```rust
pub fn high_interest_decision(ctx: &HiCtx) -> Decision  // Send | Skip(reason)
// HiCtx { now_local, settings, article_score, article_topics: Vec<(notify: bool, threshold: Option<u8>)>,
//         sent_today: u8, last_sent_at: Option<DateTime>, already_notified: bool, from_first_fetch: bool }
```

- All the SPEC §7.9 rules apply. The threshold used is the **lowest** threshold among the matching topics that have `notify = true`, falling back to the global threshold.
- Quiet hours may wrap past midnight.
- Each tick considers at most one candidate: the highest-scoring article not yet notified and discovered in the last 24 h.
- `from_first_fetch` = the article's `discovered_at` ≤ `app_state.first_fetch_completed_at`.
- `Notifier` trait → `TauriNotifier` (uses `tauri_plugin_notification`). Every sent notification is written to `notifications_log`.
- **Click behaviour (best effort).** Desktop notification click callbacks are limited in Tauri. When the app is activated right after a notification (within 2 min), show the main window on Today. Record this limitation in the QA notes.

### 4.11 `mode.rs`

- `ModeManager { mode: RwLock<Mode> }`. `set(mode)` persists to `app_state`, emits `mode://changed` and updates the tray checkmarks.
- P1 has no sidecars. The hook `on_enter_hibernate` is an empty `Vec<Box<dyn Fn>>` that P3/P4 register into.

### 4.12 `retention.rs`

This implements SPEC §7.10 in full. `0001_init.sql` creates the complete `articles` table from SPEC §13, including the `body_*`, `word_count` and `difficulty` columns. These stay empty until P2, but the body-clearing rule is implemented now so P2 doesn't need a migration for it.

The "linked to vocabulary or a conversation" condition checks tables that are created in P2 and P4. In P1, an article is protected only if `saved = 1`. P2 and P4 each extend the retention query with a `NOT EXISTS` check against their own tables.

---

## 5. Shell (Tauri)

### 5.1 `tauri.conf.json` (key parts)

```json
{
  "productName": "Tech English",
  "identifier": "com.techenglish.app",
  "app": {
    "macOSPrivateApi": true,
    "windows": [
      { "label": "widget", "url": "widget.html", "width": 340, "height": 260,
        "decorations": false, "transparent": true, "alwaysOnTop": true, "resizable": false,
        "skipTaskbar": true, "visible": false, "shadow": true }
    ]
  },
  "bundle": { "active": true, "targets": ["app", "dmg"], "icon": ["icons/icon.icns"] }
}
```

The main window (1000 × 700, min 760 × 520) is **not** declared here. It is created on demand in code (see §5.2).

### 5.2 `lib.rs` setup order

1. Init logging.
2. Register plugins:
   - `single_instance`. Its callback shows and focuses the widget.
   - `notification`
   - `autostart` (`MacosLauncher::LaunchAgent`)
   - `opener`
3. `setup`:
   1. Open the DB at `app.path().app_data_dir()/app.db` (create the directory), then run migrations.
   2. Load settings and build `AppState`, then `app.manage(state)`.
   3. If `show_dock_icon` is false: `app.set_activation_policy(ActivationPolicy::Accessory)`.
   4. Build the tray: template icon plus the SPEC §5.3 menu. The Mode items are check items. The "Today's pick" item is disabled until a pick exists; update its text on `pick://changed`.
   5. Position the widget: use the saved position if it is still on a connected monitor; otherwise `Position::TopRight` with an 12 px inset. Then apply the style and show the widget, unless its style is `hidden`.
   6. If `!onboarding_done`: show the main window at `/onboarding`.
   7. Spawn the scheduler (`tauri::async_runtime::spawn`).
4. `on_window_event`:
   - Widget `CloseRequested` → `api.prevent_close()`; hide the window. The main window is allowed to close: it is built on demand by `shell::show_main` (`WebviewWindowBuilder`) and destroyed on close, so it costs no memory while idle.
   - Widget `Moved` → save the position, debounced by 500 ms.
5. `RunEvent::ExitRequested` is only allowed from the tray Quit item (the `quitting` flag is set there). Otherwise call `api.prevent_exit()`.

### 5.3 Widget styles (`set_widget_style`)

| Style | Window |
|---|---|
| `card` | 340 × 260 |
| `pill` | 180 × 36 |
| `hidden` | the window is hidden |

- The style is persisted.
- `alwaysOnTop` is toggled with `set_always_on_top`.
- The widget can be dragged by its header (`data-tauri-drag-region`).

### 5.4 `capabilities/default.json`

- Windows: `widget`, `main`.
- Permissions:
  - `core:default`
  - `core:window:allow-start-dragging`, `core:window:allow-set-size`, `core:window:allow-hide`, `core:window:allow-show`, `core:window:allow-set-focus`
  - `notification:default`
  - `autostart:allow-enable`, `autostart:allow-disable`, `autostart:allow-is-enabled`
  - `opener:allow-open-url`
- Check the exact permission identifiers against the plugin docs at implementation time.

---

## 6. IPC contract (P1)

All commands are `async`, return `Result<T, AppError>` and use camelCase DTOs.

```ts
// src/lib/api.ts (types)
export type Mode = "standard" | "hibernate";
export type Priority = 1 | 2 | 3;
export interface Topic { id: number; name: string; keywords: string[]; excludedKeywords: string[];
  priority: Priority; enabled: boolean; notify: boolean; notifyThreshold: number | null; }
export interface Feed { id: number; kind: "rss" | "hn"; name: string; url: string; sourceWeight: number;
  enabled: boolean; lastFetchedAt: string | null; lastError: string | null; consecutiveFailures: number; }
export interface ScoreBreakdown { topicRelevance: number; freshness: number; popularity: number;
  sourcePreference: number; novelty: number; userHistory: number; total: number; }
export interface ArticleListItem { id: number; url: string; title: string; sourceName: string;
  publishedAt: string | null; discoveredAt: string; primaryTopic: string | null; topics: string[];
  score: number | null; breakdown: ScoreBreakdown | null; hnPoints: number | null; hnComments: number | null;
  readStatus: "unread" | "opened" | "read"; saved: boolean; }
export interface DailyPick { date: string; article: ArticleListItem; why: string; }
export type InteractionKind = "opened" | "read" | "saved" | "liked" | "not_interested";
export interface ArticleFilter { topicId?: number; feedId?: number; unreadOnly?: boolean;
  savedOnly?: boolean; minScore?: number; query?: string; }
export interface Page<T> { items: T[]; nextCursor: string | null; }
export interface FeedTestResult { title: string | null; itemCount: number; newestPublishedAt: string | null; }
```

| Command | Args | Returns |
|---|---|---|
| `get_mode` | – | `Mode` |
| `set_mode` | `{ mode }` | `Mode` |
| `get_settings` | – | `Settings` |
| `update_settings` | `{ patch: Partial<Settings> }` | `Settings` |
| `list_topics` | – | `Topic[]` |
| `upsert_topic` | `{ topic: Omit<Topic,"id"> & {id?: number} }` | `Topic` |
| `delete_topic` | `{ id }` | `void` |
| `list_feeds` | – | `Feed[]` |
| `upsert_feed` | `{ feed }` (validates the URL; runs test_feed first for new RSS feeds) | `Feed` |
| `delete_feed` | `{ id }` | `void` |
| `test_feed` | `{ url }` | `FeedTestResult` |
| `refresh_now` | – | `{ newCount: number }` |
| `get_today_pick` | – | `DailyPick \| null` |
| `get_pick_preview` | – | `ArticleListItem \| null` (the best candidate before pick time) |
| `list_articles` | `{ filter, cursor?, limit? (default 50) }` | `Page<ArticleListItem>` (sorted by score desc, then id; cursor = `"score:id"`) |
| `get_article` | `{ id }` | `ArticleListItem` |
| `record_interaction` | `{ articleId, kind }` | `void` |
| `open_article` | `{ articleId }` (records `opened` + opens the URL in the browser) | `void` |
| `set_widget_style` | `{ style?, alwaysOnTop? }` | `WidgetSettings` |
| `show_main` | `{ route?: string }` | `void` |
| `get_onboarding_defaults` | – | `{ topics: TopicSeed[], feeds: FeedSeed[] }` |
| `complete_onboarding` | `{ topicNames: string[], feedUrls: string[], pickTime, notifyDailyPick, notifyHighInterest, launchAtLogin }` | `void` (seeds, then triggers the first fetch) |

Events:
- `news://updated {newCount}`
- `pick://changed DailyPick`
- `mode://changed Mode`
- `settings://changed Settings`
- `navigate {route}` — main window; used by the tray and by `show_main`

---

## 7. Seed data

### `resources/default_feeds.json`

Every feed from SPEC §7.2, including the AI and data-engineering tables.

- DEV Community is expanded to **one feed row per tag**: python, ai, aws, llm, machinelearning, dataengineering.
- `source_weight`: 0.6 for primary blogs, Ars Technica and HN; 0.5 for The Verge and TechCrunch; 0.35 for Snowflake and Towards Data Science; 0.3 for DEV Community tags (tuned on live data; see SPEC §7.6).

### `resources/default_topics.json`

Onboarding pre-selects nothing; the user chooses.

| Topic | Priority | Keywords | Excluded |
|---|---|---|---|
| AI | normal | artificial intelligence, AI, generative AI, GenAI, AI model, foundation model | |
| LLMs | high | LLM, LLMs, large language model, GPT, Claude, Gemini, Llama, Mistral, Qwen, DeepSeek, transformer, fine-tuning, RAG, context window, inference, open-weight | |
| AI Agents | high | AI agent, AI agents, agentic, MCP, Model Context Protocol, tool calling, function calling, computer use, multi-agent, coding agent | |
| Machine Learning | normal | machine learning, ML, deep learning, neural network, PyTorch, JAX, training run, dataset, benchmark | |
| Python | normal | Python, PyPI, uv, Django, FastAPI, Flask, pandas, Polars, NumPy, Jupyter | Monty Python |
| Data Engineering | high | data engineering, data pipeline, ETL, ELT, data warehouse, lakehouse, data lake, Spark, Kafka, Flink, Airflow, dbt, DuckDB, Iceberg, Delta Lake, Snowflake, Databricks, streaming, batch processing, data quality, orchestration | |
| Databases | normal | database, SQL, PostgreSQL, Postgres, MySQL, SQLite, vector database, query planner, indexing | |
| Backend | normal | backend, API, microservices, distributed systems, scalability, gRPC, REST API, Redis, Go, Rust | |
| Cloud / AWS | normal | AWS, Amazon Web Services, Azure, Google Cloud, GCP, Kubernetes, serverless, Lambda, EC2, S3, cloud infrastructure | |
| Cybersecurity | normal | security, vulnerability, CVE, exploit, ransomware, malware, phishing, data breach, zero-day, supply chain attack | |
| Apple | normal | Apple, iPhone, Mac, macOS, iOS, Apple Silicon, WWDC, Xcode, Swift, Vision Pro | apple cider |
| Robotics | low | robot, robotics, humanoid, autonomous vehicle, drone, Boston Dynamics | |
| Startups | low | startup, funding round, Series A, seed round, Y Combinator, YC, acquisition, IPO, valuation | |

---

## 8. UI (P1)

### 8.1 Widget (`widget.html`)

**Header:** the app name (drag region) and a mode badge. Clicking the badge opens a Standard/Hibernate menu.

**Body states:**

| State | Content |
|---|---|
| Pick ready | title (2 lines, ellipsis) · `Topic · Source · 3h` · why (2 lines) · **[Read]** (open in browser) · **[⋯]** menu (Save, Not interested, Open in app) |
| Before pick time | "Today's pick at 08:00" + a preview item labelled *Preview* (from `get_pick_preview`) |
| No eligible | "Nothing matched today." + [Open Explore] |
| First run | "Welcome! Set up your topics." + [Start] (→ onboarding) |
| Pill | `● Today's pick` or `💤 Hibernate`. Click → back to card style. |

The footer is empty in P1; P2 adds the due count.

### 8.2 Main window routes

| Route | Content |
|---|---|
| `/today` | The pick, shown large: why, score breakdown, actions; below it, the "Also interesting" top 5 |
| `/explore` | Filter bar (topic, source, unread, saved, search) + infinite list of `ArticleRow` (title, source, topic chips, age, HN points, `ScoreChip` with a breakdown tooltip, actions: Open, Save, Not interested) |
| `/settings/general` | Mode, pick time, fetch intervals, widget style & always-on-top, launch at login, show Dock icon (needs restart; say so), ingest max age, ranking weights (advanced, collapsed) |
| `/settings/topics` | List + editor (name, `KeywordInput` chips for keywords and exclusions, priority, enabled, notify, threshold). Live preview: "Would match N of the last 72 h articles". |
| `/settings/feeds` | List with status (last fetched, error, failure count), enable toggle, source weight slider, **Add feed** (URL → Test → Save), delete |
| `/settings/notifications` | Daily pick on/off, high-interest on/off, threshold, max per day, min gap, quiet hours |
| `/onboarding` | 4 steps per SPEC §5.4, then "Fetching news…" with a live count from `news://updated`, then go to `/today` |

Accessibility: every button has a label, keyboard focus is visible, and ⌘1–⌘4 switch between the main sections.

---

## 9. Tasks

| # | Task | Depends on | Deliverables | Done when |
|---|---|---|---|---|
| T0 | Scaffold | – | `git init`; Tauri 2 + React-TS app via `npm create tauri-app@latest` (or `vite` + `tauri init`) at repo root; two Vite entries; `scripts/check.sh`; `.gitignore` (target/, node_modules/, dist/) | `npm run tauri dev` opens both windows; `scripts/check.sh` passes |
| T1 | Foundations | T0 | `error.rs`, `clock.rs`, `events.rs`, `http.rs`, `logging.rs`, `settings.rs`, `db/` + `0001_init.sql` (P1 tables from SPEC §13 + `hn_checked_at` + `app_state`), `state.rs` | Tests: migrations run on in-memory DB; settings patch/validate (reject weights not summing to 1, out-of-range intervals) |
| T2 | Shell | T1 | Windows, tray menu, hide-on-close, quit flow, accessory policy, single instance, autostart, widget positioning/persistence, `set_widget_style`, `show_main` | Manual: SPEC §18 P1 items 1, 2, 12 |
| T3 | Normalize & dedupe | T1 | `normalize.rs`, `dedupe.rs` | Tests in §10 pass |
| T4 | RSS source | T1 | `rss.rs`, `test_feed` | wiremock tests: 200, 304 + ETag, 404, HTML page, too large, timeout; fixtures parse |
| T5 | HN source | T1 | `hackernews.rs` | wiremock tests: lists, dead/ask/no-url items, cache skip < 1 h |
| T6 | Topics & ranking | T1 | `topics.rs`, `ranking.rs`, `affinity.rs` | Tests in §10 pass |
| T7 | Ingest & NewsService | T3–T6 | `ingest.rs`, `news/mod.rs` (fetch cycle, rescore, record_interaction) | Integration test: HN + RSS fixture with the same story → 1 article, 2 sources, HN stats attached; old items skipped |
| T8 | Scheduler & modes | T7 | `scheduler.rs`, `mode.rs`, `refresh_now` | FakeClock tests: interval per mode, backoff, catch-up after 3 h gap, no overlapping cycles |
| T9 | Daily pick | T7 | `pick.rs`, `why.rs` | Tests: before/after pick time, 48 → 72 h relaxation, 14-day repeat exclusion, hidden excluded, ties, skipped marking |
| T10 | Notifications | T9 | `notify.rs` | Policy table tests (every rule, quiet hours wrapping midnight, first-fetch suppression); daily pick sends exactly once |
| T11 | Retention | T7 | `retention.rs` | Tests: keeps saved/linked, deletes old, runs once per day |
| T12 | Seed & onboarding backend | T1, T7 | `seed.rs`, resources JSON, onboarding commands | Test: `complete_onboarding` inserts the selected topics/feeds only; is idempotent |
| T13 | IPC layer | T7–T12 | `commands/*`, `src/lib/api.ts` | Every command in §6 is callable from a dev-only `/debug` page |
| T14 | Widget UI | T13 | `windows/widget/*` | All states in §8.1 render; live-updates on events |
| T15 | Main window UI | T13 | pages + settings + onboarding | Manual: SPEC §18 P1 items 3, 4, 7, 10, 11 |
| T16 | Verify feeds & QA | T14, T15 | Run all default feeds once; remove any dead ones from the seed + SPEC; QA checklist (§11); `scripts/measure-idle.sh`; `docs/perf.md` | Every SPEC §18 P1 item checked; perf recorded |

Suggested order: T0 → T1 → (T2 ∥ T3 ∥ T4 ∥ T5 ∥ T6) → T7 → (T8 ∥ T9 ∥ T11 ∥ T12) → T10 → T13 → (T14 ∥ T15) → T16

---

## 10. Unit test list (minimum)

**normalize**
- `HTTPS://WWW.Example.com/a/?utm_source=x&b=2&a=1#top` → `https://example.com/a?a=1&b=2`
- A non-http scheme is rejected.
- Title suffix removal: ` - The Verge` is removed; ` - A Deep Dive` is kept.
- Jaccard: identical = 1, disjoint = 0, empty = 0.

**dedupe**
- Same normalized URL → merge.
- Same `hn_id` → merge.
- Same title key within 72 h → merge; after 72 h → new article.
- Jaccard 0.8 → merge; 0.79 → new.

**topics**
- The word boundary stops "AI" from matching "said" and "maintain".
- "C++" and ".NET" match.
- Phrase keywords match across punctuation ("tool-calling" vs "tool calling"). **Decision:** hyphens and spaces are equivalent. Normalize both the text and the keywords by replacing `-` with a space before matching.
- An exclusion zeroes only that topic.
- The priority factor is applied.

**ranking**
- Freshness at 0 h = 1, at 12 h = 0.5, at 24 h = 0.25; a future date gives 1.
- Popularity with no HN = 0.4; 500 points / 200 comments = 1.0.
- Weights applied; total is in 0..100.

**affinity**
- Deltas are applied and clamped at ±1.
- `not_interested` hides the article and affects all its sources.

**scheduler / pick / notify / retention:** as listed in §9.

---

## 11. Manual QA checklist (fill in `docs/qa/P1.md`)

- [ ] Fresh install (delete the app data dir) → onboarding appears; after finishing, Explore has items within 30 s
- [ ] Widget top-right, draggable, position remembered after restart; pill and hidden styles work; always-on-top toggle works
- [ ] No Dock icon; tray menu complete; the Mode checkmarks follow changes made from the widget/settings
- [ ] Close both windows → the app keeps running; tray Quit exits; launching again while running focuses the widget
- [ ] Add a feed with a bad URL → clear error; a good URL → shows title and count
- [ ] Topic editor live preview count changes while typing keywords
- [ ] Put the Mac to sleep for > 30 min → wake → the log shows a fetch within 2 min
- [ ] Set pick time to now + 2 min → the pick appears and the notification arrives (**on a `tauri build` bundle**)
- [ ] Set threshold 50, max/day 1 → only one high-interest notification; none during quiet hours
- [ ] Not interested → the article disappears; the breakdown of a related article shows a lower user_history
- [ ] Hibernate → the badge changes; the log shows the 45-min interval; restart keeps Hibernate
- [ ] Launch at login on → appears in System Settings › General › Login Items; off → removed

## 12. Perf procedure (`scripts/measure-idle.sh`)

1. Launch a **release build**. Wait 2 min.
2. Every 10 s for 10 min, sum the RSS and CPU% of the app process plus its `com.apple.WebKit.*` child processes, using `ps -o rss=,%cpu= -p <pids>`.
3. Output the average and max.
4. Run it once in Standard and once in Hibernate.
5. Also time one full `refresh_now` from the logs.
6. Append the results to `docs/perf.md`.

## 13. Risks

| Risk | Mitigation |
|---|---|
| Notifications don't show in `tauri dev` | Verify only on the bundled app (SPEC §7.9 note) |
| Notification click can't deep-link on desktop | Activation fallback (§4.10); document it |
| Transparent frameless window glitches on macOS | `macOSPrivateApi` is enabled; fallback: opaque rounded window with `decorations: false` |
| Feed format oddities | `feed-rs` is tolerant; per-feed errors never stop a cycle; failures are visible in Settings |
| Widget off-screen after a monitor change | Validate the saved position against the available monitors at startup (§5.2) |
