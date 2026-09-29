import { useEffect, useState } from "react";
import { api, type Feed, type FeedTestResult } from "../../lib/api";
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
      <div className={styles.groupTitle}>Add a source</div>
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
