-- "Show another" on Today: stories/lessons passed over on a day are not suggested again
-- (same windows as picks: 14 days for stories, 60 for lessons).
CREATE TABLE pick_skips (
  date TEXT NOT NULL,                -- local YYYY-MM-DD
  kind TEXT NOT NULL CHECK (kind IN ('story','lesson')),
  article_id INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  PRIMARY KEY (date, kind, article_id)
);
CREATE INDEX idx_pick_skips_article ON pick_skips(article_id);
