import { useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { api, type ExamplePage, type Feed, type FeedCandidate, type FeedTestResult } from "../../lib/api";
import { timeAgo } from "../../lib/format";
import { useApp } from "../../stores/app";
import { toast, toastError } from "../../stores/toast";
import styles from "../pages.module.css";

function FeedStatus({ f }: { f: Feed }) {
  if (f.lastError) {
    return (
      <span className={`${styles.feedStatus} error-text`} title={f.lastError}>
        ⚠ {f.lastError}
        {f.consecutiveFailures > 1 ? ` (${f.consecutiveFailures}×)` : ""}
      </span>
    );
  }
  return (
    <span className={`${styles.feedStatus} faint`}>
      {f.lastFetchedAt ? `Updated ${timeAgo(f.lastFetchedAt)}` : "Not fetched yet"}
    </span>
  );
}

function AddFeed({ onAdded }: { onAdded: () => void }) {
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [result, setResult] = useState<FeedTestResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const test = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const r = await api.testFeed(url);
      setResult(r);
      if (!name && r.title) setName(r.title);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const add = async () => {
    setBusy(true);
    try {
      await api.upsertFeed({ kind: "rss", name: name.trim() || url, url: url.trim() });
      toast("Source added. Fetching now…");
      setUrl("");
      setName("");
      setResult(null);
      onAdded();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={styles.addFeed}>
      <div className="row">
        <input
          type="url"
          placeholder="https://example.com/feed.xml"
          value={url}
          onChange={(e) => {
            setUrl(e.target.value);
            setResult(null);
            setError(null);
          }}
          aria-label="Feed URL"
        />
        <button onClick={test} disabled={busy || !url.trim()}>
          {busy && !result ? "Testing…" : "Test"}
        </button>
      </div>
      {error && <div className={`${styles.testResult} error-text`}>{error}</div>}
      {result && (
        <>
          <div className={styles.testResult}>
            ✓ {result.title ?? "Untitled feed"} · {result.itemCount} articles
            {result.newestPublishedAt ? ` · newest ${timeAgo(result.newestPublishedAt)}` : ""}
          </div>
          <div className="row">
            <input type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" aria-label="Feed name" />
            <button className="primary" onClick={add} disabled={busy}>
              Add source
            </button>
          </div>
        </>
      )}
    </div>
  );
}

/** Paste an article you like → find that site's feed (SPEC §7.12). */
function AddFromExample({ onAdded }: { onAdded: () => void }) {
  const navigate = useNavigate();
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState<"find" | "add" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [found, setFound] = useState<{ page: ExamplePage; candidates: FeedCandidate[] } | null>(null);
  const [choice, setChoice] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [learning, setLearning] = useState(false);
  const [saveArticle, setSaveArticle] = useState(true);
  const [done, setDone] = useState<{ message: string; articleId: number | null } | null>(null);

  const reset = () => {
    setFound(null);
    setError(null);
    setDone(null);
    setChoice(null);
  };

  const find = async () => {
    reset();
    setBusy("find");
    try {
      const r = await api.discoverFeeds(url.trim());
      setFound(r);
      const first = r.candidates.find((c) => !c.alreadyAdded) ?? null;
      setChoice(first?.url ?? null);
      setName(first?.title?.trim() || r.page.siteName);
      setSaveArticle(r.page.canSave);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const add = async (feedUrl: string | null, save: boolean) => {
    setBusy("add");
    try {
      const r = await api.addFeedFromExample({ url: url.trim(), feedUrl, name: name.trim() || found?.page.siteName || url, learning, saveArticle: save });
      const parts = [r.feed && `Added "${r.feed.name}". Fetching its articles now…`, r.article && "The article is saved."];
      setDone({ message: parts.filter(Boolean).join(" "), articleId: r.article?.id ?? null });
      setFound(null);
      setUrl("");
      onAdded();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };

  const allAdded = !!found && found.candidates.length > 0 && found.candidates.every((c) => c.alreadyAdded);
  const canSave = !!found?.page.canSave;

  return (
    <div className={styles.addFeed}>
      <div className="row">
        <input
          type="url"
          placeholder="Paste an article or blog you like, e.g. https://blog.example.com/my-favorite-post"
          value={url}
          onChange={(e) => {
            setUrl(e.target.value);
            reset();
          }}
          onKeyDown={(e) => e.key === "Enter" && url.trim() && !busy && void find()}
          aria-label="Example article URL"
        />
        <button onClick={find} disabled={!!busy || !url.trim()}>
          {busy === "find" ? "Looking…" : "Find feed"}
        </button>
      </div>
      {busy === "find" && <div className="faint" style={{ fontSize: 12 }}>Checking the site. This can take up to 30 seconds.</div>}
      {error && <div className={`${styles.testResult} error-text`}>{error}</div>}
      {done && (
        <div className={styles.testResult} role="status">
          ✓ {done.message}{" "}
          {done.articleId != null && (
            <button className="ghost" onClick={() => navigate(`/reader/${done.articleId}`)}>
              Read it now →
            </button>
          )}
        </div>
      )}

      {found && (
        <>
          <div className={styles.examplePage}>
            <div className={styles.feedName}>{found.page.title}</div>
            <div className="faint" style={{ fontSize: 12 }}>
              {found.page.siteName}
              {found.page.publishedAt ? ` · ${timeAgo(found.page.publishedAt)}` : ""}
            </div>
          </div>

          {found.candidates.length === 0 ? (
            <div className={styles.testResult}>We couldn't find a feed for this site. Some sites don't have one.</div>
          ) : (
            <div className={styles.candidates} role="radiogroup" aria-label="Feeds found">
              {found.candidates.map((c) => (
                <label key={c.url} className={`${styles.candidate} ${c.alreadyAdded ? styles.candidateOff : ""}`}>
                  <input
                    type="radio"
                    name="candidate"
                    checked={choice === c.url}
                    disabled={c.alreadyAdded}
                    onChange={() => {
                      setChoice(c.url);
                      setName(c.title?.trim() || found.page.siteName);
                    }}
                  />
                  <span style={{ minWidth: 0 }}>
                    <span className={styles.feedName}>{c.title ?? "Untitled feed"}</span>
                    <span className={styles.feedUrl}>{c.url}</span>
                    <span className="faint" style={{ fontSize: 12 }}>
                      {c.itemCount} articles
                      {c.newestPublishedAt ? ` · newest ${timeAgo(c.newestPublishedAt)}` : ""}
                      {c.alreadyAdded ? " · already added" : ""}
                    </span>
                  </span>
                </label>
              ))}
            </div>
          )}
          {allAdded && <div className={styles.testResult}>This source is already in your list.</div>}

          {choice && (
            <>
              <input type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" aria-label="Source name" />
              <label className={styles.switchRow}>
                <input type="checkbox" checked={learning} onChange={(e) => setLearning(e.target.checked)} />
                <span>
                  <strong>Learning source</strong>
                  <span className={styles.fieldHelp}>This site mostly publishes tutorials, guides or explainers.</span>
                </span>
              </label>
            </>
          )}
          {choice && canSave && (
            <label className={styles.switchRow}>
              <input type="checkbox" checked={saveArticle} onChange={(e) => setSaveArticle(e.target.checked)} />
              <span>
                <strong>Also save this article</strong>
                <span className={styles.fieldHelp}>You can read it right away in the app.</span>
              </span>
            </label>
          )}
          <div className="row">
            <span className="spacer" />
            {choice ? (
              <button className="primary" onClick={() => void add(choice, saveArticle && canSave)} disabled={!!busy}>
                {busy === "add" ? "Adding…" : "Add"}
              </button>
            ) : (
              canSave && (
                <button className="primary" onClick={() => void add(null, true)} disabled={!!busy}>
                  {busy === "add" ? "Saving…" : "Save article only"}
                </button>
              )
            )}
          </div>
        </>
      )}
    </div>
  );
}

export default function Feeds() {
  const newsVersion = useApp((s) => s.newsVersion);
  const [feeds, setFeeds] = useState<Feed[]>([]);
  const load = () => api.listFeeds().then(setFeeds).catch(toastError);
  useEffect(() => {
    void load();
  }, [newsVersion]);

  const update = (f: Feed, change: Partial<Feed>) =>
    api
      .upsertFeed({
        id: f.id,
        kind: f.kind,
        name: f.name,
        url: f.url,
        sourceWeight: f.sourceWeight,
        enabled: f.enabled,
        learning: f.learning,
        ...change,
      })
      .then(load)
      .catch(toastError);

  const remove = (f: Feed) => {
    if (!window.confirm(`Remove "${f.name}"? Stories you saved stay.`)) return;
    api.deleteFeed(f.id).then(load).catch(toastError);
  };

  const errors = feeds.filter((f) => f.lastError).length;

  return (
    <div>
      <div className={styles.groupTitle}>Add from an example</div>
      <div className={styles.group}>
        <AddFromExample onAdded={load} />
      </div>

      <div className={styles.groupTitle}>Add a feed URL</div>
      <div className={styles.group}>
        <AddFeed onAdded={load} />
      </div>

      <div className={styles.groupTitle}>
        Your sources ({feeds.length}){errors ? ` · ${errors} with errors` : ""}
      </div>
      <div className={styles.group}>
        {feeds.map((f) => (
          <div key={f.id} className={styles.feedRow}>
            <input
              type="checkbox"
              checked={f.enabled}
              onChange={(e) => void update(f, { enabled: e.target.checked })}
              aria-label={`Enable ${f.name}`}
            />
            <div style={{ minWidth: 0 }}>
              <div className={styles.feedName}>{f.name}</div>
              <div className={styles.feedUrl}>{f.url}</div>
              <FeedStatus f={f} />
            </div>
            {f.kind === "rss" ? (
              <label
                className={styles.learningToggle}
                title="This source mostly publishes learning material (tutorials, guides, explainers)"
              >
                <input
                  type="checkbox"
                  checked={f.learning}
                  onChange={(e) => void update(f, { learning: e.target.checked })}
                  aria-label={`${f.name} is a learning source`}
                />
                📘 Learning
              </label>
            ) : (
              <span />
            )}
            <label className={styles.weight}>
              Preference {Math.round(f.sourceWeight * 100)}%
              <input
                type="range"
                min={0}
                max={1}
                step={0.1}
                defaultValue={f.sourceWeight}
                onMouseUp={(e) => void update(f, { sourceWeight: Number((e.target as HTMLInputElement).value) })}
                onKeyUp={(e) => void update(f, { sourceWeight: Number((e.target as HTMLInputElement).value) })}
                aria-label={`Preference for ${f.name}`}
              />
            </label>
            <button className="ghost" onClick={() => remove(f)} aria-label={`Remove ${f.name}`} title="Remove">
              ✕
            </button>
          </div>
        ))}
        {!feeds.length && <div className={styles.empty}>No sources yet.</div>}
      </div>
    </div>
  );
}
