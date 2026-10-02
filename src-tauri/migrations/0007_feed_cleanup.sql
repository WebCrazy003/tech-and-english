-- Feed cleanup (tested 2026-10-02 with the app's own request).
-- Removed: sites that block the app or give no article text (Towards Data Science: bot check,
-- Netflix TechBlog: 403, Confessions of a Data Guy: no text in the page).
UPDATE articles SET hidden = 1, body_text = NULL, body_html = NULL
WHERE body_status <> 'ok' AND id IN (
  SELECT s.article_id FROM article_sources s JOIN feeds f ON f.id = s.feed_id
  WHERE f.url IN ('https://towardsdatascience.com/feed', 'https://netflixtechblog.com/feed',
                  'https://www.confessionsofadataguy.com/feed/'));
DELETE FROM feeds WHERE url IN ('https://towardsdatascience.com/feed', 'https://netflixtechblog.com/feed',
                                'https://www.confessionsofadataguy.com/feed/');

-- New sources whose full articles the app can read (existing installs; a fresh install picks them in onboarding).
INSERT OR IGNORE INTO feeds(kind, name, url, source_weight, learning, created_at)
SELECT 'rss', column1, column2, column3, column4, '2026-10-02T00:00:00Z' FROM (VALUES
  ('The Pragmatic Engineer','https://newsletter.pragmaticengineer.com/feed',0.6,0),
  ('InfoQ','https://feed.infoq.com/',0.5,0),
  ('The New Stack','https://thenewstack.io/feed/',0.5,0),
  ('Spotify Engineering','https://engineering.atspotify.com/feed/',0.6,0),
  ('Stripe Blog','https://stripe.com/blog/feed.rss',0.5,0),
  ('Slack Engineering','https://slack.engineering/feed/',0.6,0),
  ('Meta Engineering','https://engineering.fb.com/feed/',0.6,0),
  ('Dropbox Tech','https://dropbox.tech/feed',0.6,0),
  ('Lilian Weng','https://lilianweng.github.io/index.xml',0.6,1),
  ('Hamel Husain','https://hamel.dev/index.xml',0.6,1),
  ('Jay Alammar','https://newsletter.languagemodels.co/feed',0.6,1),
  ('Martin Fowler','https://martinfowler.com/feed.atom',0.6,1),
  ('Julia Evans','https://jvns.ca/atom.xml',0.6,1),
  ('ByteByteGo','https://blog.bytebytego.com/feed',0.6,1))
WHERE EXISTS (SELECT 1 FROM feeds);
