-- P3: learn topics, learning sources, learning/lesson scores, and a lesson kind for daily picks.

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

-- new learning sources for existing installs (a fresh install picks them in onboarding),
-- only if the URL is not already there
INSERT OR IGNORE INTO feeds(kind, name, url, source_weight, learning, created_at)
SELECT 'rss', column1, column2, column3, 1, '2026-09-29T00:00:00Z' FROM (VALUES
  ('Practical Data Modeling (Joe Reis)','https://practicaldatamodeling.substack.com/feed',0.6),
  ('Data Engineering Central','https://dataengineeringcentral.substack.com/feed',0.6),
  ('Dagster Blog','https://dagster.io/blog/rss.xml',0.6),
  ('MotherDuck Blog','https://motherduck.com/rss.xml',0.6),
  ('Estuary Blog','https://estuary.dev/blog/rss.xml',0.5),
  ('Confessions of a Data Guy','https://www.confessionsofadataguy.com/feed/',0.5),
  ('Real Python','https://realpython.com/atom.xml',0.6),
  ('freeCodeCamp News','https://www.freecodecamp.org/news/rss/',0.5),
  ('Chip Huyen','https://huyenchip.com/feed.xml',0.6))
WHERE EXISTS (SELECT 1 FROM feeds);
UPDATE feeds SET learning = 1 WHERE url IN (
  'https://seattledataguy.substack.com/feed','https://www.getdbt.com/blog/rss.xml','https://duckdb.org/feed.xml',
  'https://magazine.sebastianraschka.com/feed','https://towardsdatascience.com/feed');
