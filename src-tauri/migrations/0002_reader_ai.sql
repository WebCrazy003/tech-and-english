-- P2: cached AI outputs per article, and the per-article AI chat.

CREATE TABLE article_derivatives (
  article_id INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('summary_b1','easy_english','why_interesting')),
  model_id TEXT NOT NULL,
  prompt_version TEXT NOT NULL,
  text TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (article_id, kind)
);

CREATE TABLE article_chats (
  id INTEGER PRIMARY KEY,
  article_id INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('user','assistant')),
  content TEXT NOT NULL,
  model_id TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_article_chats_article ON article_chats(article_id, id);
