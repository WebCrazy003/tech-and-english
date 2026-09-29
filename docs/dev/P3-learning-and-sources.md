# P3 — Learning materials & example sources (dev spec)

- **Goal:**
  - Each day, pick one **lesson** (a tutorial, explainer or deep dive) next to the news story, for topics the user wants to learn.
  - Let the user add a source by pasting an **example** article or blog URL.
- **SPEC sections:** §7.1 (`learn`), §7.11, §7.12, §13 (P3), §14 (P3), §18 P3.
- **Branch:** `p3/learning-sources` → tag `v0.3.0`
- **Why:** the user's keyword topic "learn data engineering" matched 0 of 363 stories. Learning needs a *content-type* signal (is this a tutorial?) on top of the *subject* match (is this data engineering?).

## 0. Scope

**In scope:**
- Topic `learn` flag; feed `learning` flag
- `learning_score` (rules, no LLM); lesson selection and `lesson_score`
- `daily_picks.kind` (story / lesson); a combined daily notification
- New learning feeds and flags (migration, only when missing)
- Feed discovery from an example URL; saving the example article
- UI:
  - widget Story | Lesson switch
  - Today: two cards
  - Explore: **Learning only** filter
  - Settings: Learn / Learning-source switches, **Add from an example**

**Out of scope:**
- Suggesting other similar blogs (SPEC §20)
- LLM-based classification (a possible later improvement: when the model is loaded, it could confirm the lesson candidate)

---

## 1. Database — `migrations/0003_learning.sql`

```sql
ALTER TABLE topics ADD COLUMN learn INTEGER NOT NULL DEFAULT 0;
ALTER TABLE feeds ADD COLUMN learning INTEGER NOT NULL DEFAULT 0;
ALTER TABLE articles ADD COLUMN learning_score REAL;
ALTER TABLE articles ADD COLUMN lesson_score REAL;
CREATE INDEX idx_articles_lesson ON articles(lesson_score DESC);

-- daily_picks gets a kind; the primary key becomes (date, kind)
CREATE TABLE daily_picks_new (
  date TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'story' CHECK (kind IN ('story','lesson')),
  article_id INTEGER NOT NULL REFERENCES articles(id),
  why TEXT NOT NULL,
  why_source TEXT NOT NULL DEFAULT 'template' CHECK (why_source IN ('template','llm')),
  created_at TEXT NOT NULL,
  PRIMARY KEY (date, kind)
);
INSERT INTO daily_picks_new(date, kind, article_id, why, why_source, created_at)
  SELECT date, 'story', article_id, why, why_source, created_at FROM daily_picks;
DROP TABLE daily_picks;
ALTER TABLE daily_picks_new RENAME TO daily_picks;

-- new learning sources, only if the URL is not already there
INSERT OR IGNORE INTO feeds(kind, name, url, source_weight, learning, created_at) VALUES
  ('rss','Practical Data Modeling (Joe Reis)','https://practicaldatamodeling.substack.com/feed',0.6,1,'2026-09-29T00:00:00Z'),
  ('rss','Data Engineering Central','https://dataengineeringcentral.substack.com/feed',0.6,1,'2026-09-29T00:00:00Z'),
  ('rss','Dagster Blog','https://dagster.io/blog/rss.xml',0.6,1,'2026-09-29T00:00:00Z'),
  ('rss','MotherDuck Blog','https://motherduck.com/rss.xml',0.6,1,'2026-09-29T00:00:00Z'),
  ('rss','Estuary Blog','https://estuary.dev/blog/rss.xml',0.5,1,'2026-09-29T00:00:00Z'),
  ('rss','Confessions of a Data Guy','https://www.confessionsofadataguy.com/feed/',0.5,1,'2026-09-29T00:00:00Z'),
  ('rss','Real Python','https://realpython.com/atom.xml',0.6,1,'2026-09-29T00:00:00Z'),
  ('rss','freeCodeCamp News','https://www.freecodecamp.org/news/rss/',0.5,1,'2026-09-29T00:00:00Z'),
  ('rss','Chip Huyen','https://huyenchip.com/feed.xml',0.6,1,'2026-09-29T00:00:00Z');
UPDATE feeds SET learning = 1 WHERE url IN (
  'https://seattledataguy.substack.com/feed','https://www.getdbt.com/blog/rss.xml','https://duckdb.org/feed.xml',
  'https://magazine.sebastianraschka.com/feed','https://towardsdatascience.com/feed');
```

- The seeds in `resources/default_feeds.json` get the same entries, with a `learning` field and the group **"Learning"**, so new installs match.
- **Retention:** articles that were a lesson pick are protected, like story picks (the existing `daily_picks` check covers both kinds).
- **Migration test:** apply 0001–0003 on a DB that has P1 data (picks, feeds) → picks keep `kind='story'`; the new feeds are added once; running it on a DB that already has one of those URLs adds no duplicate.

---

## 2. Learning score — `news/learning.rs` (pure)

```rust
pub struct LearningInput<'a> { pub title: &'a str, pub description: Option<&'a str>,
                               pub feed_learning: bool, pub word_count: Option<u32> }
pub fn learning_score(i: &LearningInput) -> f64    // 0..1, SPEC §7.11 table
pub fn is_course_spam(text: &str) -> bool
```

**Patterns:**
- Case-insensitive, on word boundaries (reuse the `topics.rs` regex builder), applied to the title and the first 300 chars of the description.
- **Learning:** `how to` · `how .{1,40} works?` · `guide` · `tutorial` · `explained` · `explainer` · `introduction to` · `intro to` · `beginners?` · `101` · `deep dive` · `what is` · `what are` · `understanding` · `step[- ]by[- ]step` · `from scratch` · `best practices` · `patterns` · `primer` · `cheat ?sheet` · `hands[- ]on` · `walkthrough` · `fundamentals` · `lessons learned` · ` vs\.? ` (comparison)
- **News:** `announces?` · `launch(es|ed)?` · `raises` · `acquires?` · `funding` · `now available` · `introducing` · `release notes` · `weekly roundup` · `this week in` · `roundup`
- **Course spam:** `training in` · `course in` · `classes in` · `institute` · `certification training` · `bootcamp in` · `job guarantee` · `placement` · `who'?s hiring`

**Score:**
- +0.25 per distinct learning pattern (max 0.75)
- +0.35 if `feed_learning`
- +0.10 if `word_count ≥ 1200`
- −0.30 if any news pattern
- clamp to 0..1
- `is_course_spam` → 0

**When it is computed:**
1. At ingest, for new and merged articles (P1 `ingest.rs` calls it).
2. When a body is saved (P2 `save_article_body`), because `word_count` is now known.
3. When a feed's `learning` flag changes: recompute for that feed's articles from the last 60 days.

**Tests** use **real titles from the live data** (2026-09-29), stored in `tests/fixtures/learning_titles.json` as `{title, feed_learning, expect: "lesson" | "not" | "spam"}`. At least 25 cases, for example:

| Title | Expect |
|---|---|
| How to Evaluate RAG Pipeline Quality: Metrics and Test Harness | lesson |
| How to Detect Outliers in Python Using the Interquartile Range (IQR) | lesson |
| AWS Control Tower Landing Zone: A Complete Setup Guide for Multi-Account Migrations | lesson |
| Learning Data Engineering in Noida: What to Study, Who's Hiring, and How to Pick a Course | spam |
| Python Full Stack Training in Bangalore \| Learnmore Technologies 🚀 | spam |
| AWS Weekly Roundup: GPT-6 Sol and Luna, Claude Opus 5.5 on Amazon Bedrock | not |
| Announcing Claude Sonnet 5.5 on Snowflake Cortex AI | not |
| Nvidia wants to put a watchdog chip next to every AI agent | not |

"lesson" means score ≥ 0.5; "not" means < 0.5.

---

## 3. Lesson selection — `news/pick.rs`

`PickService` handles both kinds. `DailyPick` gains `kind: "story" | "lesson"`.

```
ensure_today(now):                       (same trigger as P1: local time ≥ pick_time)
  story_new = story missing → select story (P1 + P2 rules) → insert (kind='story')
  lesson_new = lesson missing and any topic has learn=1 → select_lesson(now, today, exclude = story id)
               → P2 post_select (paywall retry, body, difficulty) → insert (kind='lesson')
  if story_new: notify once:
       "Today's story and lesson are ready." if a lesson exists, else "Today's tech story is ready."
  (a lesson found later that day is inserted quietly; the UI updates via pick://changed)
```

**`select_lesson` SQL** (SPEC §7.11):
- `learning_score ≥ 0.5`
- `EXISTS article_topics at JOIN topics t … t.learn = 1 AND at.relevance ≥ 0.3`
- `COALESCE(published_at, discovered_at) ≥ now − lessonMaxAgeDays`
- `hidden = 0`, `body_status != 'paywalled'`
- `id != story_id`, and `id NOT IN (picks of either kind in the last 60 days)`
- Order: `lesson_score DESC, discovered_at DESC`.

**`lesson_score`** is computed in `rescore()` next to `score`, but only for articles with `learning_score ≥ 0.5`:
- The learn-topic relevance is the max relevance over topics with `learn = 1`.
- Popularity and user history use the P1 formulas.
- The rescore window for lessons is 60 days, not 7. Only the few learning articles are rescored, so this stays cheap.

**Why text** (template): `Tutorial · Data Engineering · from a learning source`.
- The first part is `Tutorial`, `Guide`, `Deep dive` or `Explainer`, from the strongest learning pattern.
- The second part is the learn topic.
- The third part is added when the feed has `learning = 1`.

**Ingest age:** in `ingest.rs`, `max_age_days = if feed.learning { settings.lesson_max_age_days } else { settings.ingest_max_age_days }`. The new setting is `lessonMaxAgeDays`, default 60, range 14–180.

**Skipped:** yesterday's lesson not opened → a `skipped` interaction, the same as for the story.

**Tests** (FakeClock + fixtures):
- A lesson is chosen and is never the story.
- News-only articles are never lessons.
- The 60-day repeat exclusion works.
- No learn topics → no lesson.
- The notification text changes when a lesson exists, and exactly one daily notification is sent.
- A lesson inserted later in the day triggers no notification.

---

## 4. Add a source from an example — `news/discover.rs`

```rust
pub struct FeedCandidate { pub url: String, pub title: Option<String>, pub item_count: usize,
                           pub newest_published_at: Option<String>, pub already_added: bool }
pub struct ExamplePage { pub url: String, pub title: String, pub description: Option<String>,
                         pub published_at: Option<String>, pub site_name: String }
pub async fn discover(http, url) -> AppResult<(ExamplePage, Vec<FeedCandidate>)>
```

**Algorithm** (SPEC §7.12):
1. GET the page (15 s, 3 MB, HTML only), following redirects; `final_url` is the base URL.
2. Parse `<head>` with regexes (no full HTML parser needed):
   - `<link rel="alternate" type="application/(rss|atom)+xml|application/feed+json" href=…>` → candidates, resolved against the base URL.
   - `og:title` / `<title>` → title; `og:description` / `meta description` → description; `article:published_time` → date; `og:site_name` or the host → site name.
   - `meta name="generator"` → platform hint (WordPress, Ghost, Hugo, Jekyll).
3. If there are no candidates yet, add pattern candidates, **max 8 probes**:
   - `*.substack.com` or a page containing `substackcdn` → `/feed`
   - `medium.com/@user/...` → `https://medium.com/feed/@user`; `medium.com/<pub>/...` → `https://medium.com/feed/<pub>`
   - WordPress hint or `/wp-content/` in HTML → `/feed/`; Ghost → `/rss/`; Hugo → `/index.xml`; Jekyll → `/feed.xml`
   - generic: `/feed`, `/rss.xml`, `/atom.xml`, `/index.xml`, then the same on the article's parent path (`/blog/feed`, …)
4. Validate each candidate with `rss::test_feed`, 4 at a time, 8 s each. Keep only valid feeds, deduplicated by URL. Mark `already_added` when the URL matches an existing feed after normalization.
5. Return the candidates sorted by `item_count` (desc).

**Commands:**

| Command | Args | Returns |
|---|---|---|
| `discover_feeds` | `{ url }` | `{ page: ExamplePage, candidates: FeedCandidate[] }` |
| `add_feed_from_example` | `{ url, feedUrl?, name, learning, saveArticle }` | `{ feed?: Feed, article?: ArticleListItem }` |

`add_feed_from_example` does the following:
- If `feedUrl` is given, it inserts the feed (P1 `upsert_feed` validation), with `learning` and `source_weight` 0.6.
- If `saveArticle` is true, it inserts the example page as an article: `source_name = site_name`, no `article_sources` row (scoring treats a missing source as weight 0.5), `saved = 1`. Then it runs topic matching and the learning score. The article can then be opened in the Reader (P2).
- It triggers a fetch of the new feed.
- It works **even when no feed is found** (`feedUrl` absent). Then it only saves the article.

**Tests** (wiremock, one fixture page per case):
- `<link rel=alternate>` with a relative href
- Substack subdomain
- WordPress generator
- Ghost generator
- Hugo generator
- Medium user URL (the rewrite is tested with a pure function; no live Medium call)
- a page with no feed → empty candidates
- a candidate that returns HTML is rejected
- `already_added` is detected
- the probe limit is respected

**Live check** (ignored test): the two example URLs the user gives, plus `https://www.startdataengineering.com/` → it must report "no feed".

---

## 5. UI

| Place | Change |
|---|---|
| Widget | Header row: segmented **Story \| Lesson** (hidden if there is no lesson). The lesson card shows the label "Today's lesson" and the lesson "why". The pill text is "Story + lesson ready". |
| `/today` | Two cards side by side (stacked if the window is narrow): **Today's story** and **Today's lesson**. The lesson card shows the learn topic chip and "Why this lesson". If there is no lesson: "No lesson today. Turn on **Learn** for a topic in Settings › Topics." |
| `/explore` | **Learning only** checkbox (`learning_score ≥ 0.5`). Rows with a high learning score show a small "📘 Learn" badge. |
| Settings › Topics | **Learn** switch: "Also find learning materials (tutorials, explainers) for this topic". **Tip:** if a topic's name starts with "learn" and it matched 0 stories in 3 days, show: "Tip: learning topics work better as a switch. Turn on **Learn** for your subject topic (e.g. Data Engineering) and delete this one." |
| Settings › News sources | A new **Add from an example** card at the top: URL input → [Find feed] → results (radio list: title, item count, newest date, "already added") → name field, **Learning source** switch, **Also save this article** (checked) → [Add]. With no candidates: the message from SPEC §7.12, plus [Save article only]. Each feed row gets a **Learning** switch. |
| Settings › General | "Lessons: consider articles up to N days old" (14–180, default 60) |
| Mock backend | `installMock.ts` gains lesson and discovery fixtures, for browser previews |

---

## 6. Tasks

| # | Task | Depends on | Done when |
|---|---|---|---|
| T1 | Migration 0003 + repo changes (`learn`, `learning`, `kind`, scores) + seeds | – | §1 migration test; P1 tests still pass |
| T2 | `learning.rs` + fixture titles | – | ≥ 25 real-title cases pass |
| T3 | Ingest integration (score at ingest/body/flag change; learning ingest age) | T1, T2 | Tests: a learning feed keeps 50-day-old items; a normal feed keeps 7 days only |
| T4 | Lesson scoring + selection + notification text (§3) | T1–T3 | §3 tests |
| T5 | `discover.rs` + commands (§4) | T1 | §4 wiremock tests |
| T6 | Widget + Today + Explore UI | T4 | Manual: SPEC §18 P3 items 1, 4, 7 |
| T7 | Settings UI (Learn, Learning source, Add from example, lesson age, tip) | T5 | Manual: SPEC §18 P3 items 5, 6 |
| T8 | Live evaluation | T4 | `full_pipeline` prints the top 10 lesson candidates; ≥ 8/10 judged learning material; recorded in `docs/qa/P3.md`. Tune the patterns if needed (with tests). |
| T9 | QA | all | `docs/qa/P3.md` complete |

Suggested order: (T1 ∥ T2) → (T3 ∥ T5) → T4 → (T6 ∥ T7) → T8 → T9

---

## 7. Manual QA checklist (`docs/qa/P3.md`)

- [ ] Turn on Learn for Data Engineering → after the next pick time (or Refresh), Today shows a lesson that is clearly a tutorial or explainer
- [ ] The widget switch shows Story and Lesson; the lesson opens in the Reader with the AI panel
- [ ] Explore › Learning only lists tutorials and guides, not news
- [ ] Paste a Substack article URL → its feed is found and added; its articles appear after the fetch
- [ ] Paste `https://www.startdataengineering.com/` → "We couldn't find a feed…" + Save article only works
- [ ] Paste an article from a source already in the list → "already added"
- [ ] The daily notification says "Today's story and lesson are ready." (bundled app)
- [ ] The tip appears for the "learn data engineering" topic

---

## 8. Risks

| Risk | Mitigation |
|---|---|
| Pattern rules pick weak "how to" spam posts from DEV | Source weight (DEV 0.3) in `lesson_score`; spam rules; live evaluation (T8); the user can switch off Learning for a feed |
| Not enough lesson candidates on some days | 60-day window; learning sources; if none is found, "No lesson today" is fine |
| Feed discovery misses some platforms | Probe list + a clear message; the user can still paste a feed URL in "Add a source" |
| Medium feeds are sometimes blocked | Test only via the pure rewrite function; a failure shows the normal error |
