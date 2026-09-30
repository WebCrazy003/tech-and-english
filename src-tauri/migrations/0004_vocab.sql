-- P4: Word Book, contexts, quiz sessions and reviews (SPEC §13).
-- vocab_contexts.conversation_id has no foreign key: the conversations table arrives in P5,
-- and the app deletes contexts itself when a conversation is deleted.

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
  conversation_id INTEGER,
  sentence TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_vocab_contexts_item ON vocab_contexts(item_id);

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
CREATE INDEX idx_vocab_reviews_item ON vocab_reviews(item_id, reviewed_at);
