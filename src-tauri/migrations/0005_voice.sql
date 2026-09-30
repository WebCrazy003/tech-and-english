-- P5: voice tutor conversations, their turns and the learning observations (SPEC §13).
-- vocab_contexts.conversation_id still has no foreign key (see 0004): the app deletes those
-- context rows itself when a conversation is deleted.

CREATE TABLE conversations (
  id INTEGER PRIMARY KEY,
  article_id INTEGER REFERENCES articles(id) ON DELETE SET NULL,
  settings TEXT NOT NULL,            -- level, rate, correction frequency, voice
  started_at TEXT NOT NULL, ended_at TEXT,
  user_speaking_seconds INTEGER NOT NULL DEFAULT 0,
  review_status TEXT NOT NULL DEFAULT 'pending' CHECK (review_status IN ('pending','done','skipped'))
);
CREATE INDEX idx_conversations_started ON conversations(started_at);

CREATE TABLE conversation_turns (
  id INTEGER PRIMARY KEY,
  conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('user','tutor')),
  text TEXT NOT NULL,
  meta TEXT,                         -- tutor_turn JSON (correction, unknown_terms…)
  created_at TEXT NOT NULL,
  UNIQUE (conversation_id, seq)
);
CREATE INDEX idx_turns_conv ON conversation_turns(conversation_id, seq);

CREATE TABLE learning_observations (
  id INTEGER PRIMARY KEY,
  conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('unknown_word','unknown_phrase','pronunciation','grammar','useful_sentence')),
  text TEXT NOT NULL,
  detail TEXT,                       -- JSON: explanation / original+corrected / heard+target
  saved_item_id INTEGER REFERENCES vocab_items(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_observations_conv ON learning_observations(conversation_id);
