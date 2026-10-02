-- Articles whose full text could not be loaded are hidden and their partial text dropped
-- (new rule: never show or keep stories the app can't read).
UPDATE articles SET hidden = 1, body_text = NULL, body_html = NULL
WHERE body_status IN ('failed', 'paywalled');
