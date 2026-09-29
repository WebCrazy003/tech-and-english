-- P1: settings, topics, feeds, articles, ranking and picks.
-- Times are RFC 3339 UTC strings ("2026-09-29T08:00:00Z"); JSON columns are TEXT.

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);

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
  etag TEXT,
  last_modified TEXT,
  last_fetched_at TEXT,
  last_error TEXT,
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
  hn_id INTEGER UNIQUE,
  hn_points INTEGER,
  hn_comments INTEGER,
  hn_checked_at TEXT,
  body_text TEXT,
  body_html TEXT,
  body_status TEXT NOT NULL DEFAULT 'none' CHECK (body_status IN ('none','ok','failed','paywalled')),
  word_count INTEGER,
  difficulty TEXT CHECK (difficulty IN ('easy','medium','hard')),
  primary_topic_id INTEGER REFERENCES topics(id) ON DELETE SET NULL,
  score REAL,
  score_breakdown TEXT,
  scored_at TEXT,
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
CREATE INDEX idx_article_sources_feed ON article_sources(feed_id);

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
CREATE INDEX idx_interactions_article ON article_interactions(article_id, kind);

CREATE TABLE affinities (
  kind TEXT NOT NULL CHECK (kind IN ('topic','source')),
  ref_id INTEGER NOT NULL,
  value REAL NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (kind, ref_id)
);

CREATE TABLE daily_picks (
  date TEXT PRIMARY KEY,
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
