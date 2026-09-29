import { useCallback, useEffect, useRef, useState } from "react";
import { api, type ArticleFilter, type ArticleListItem, type Feed, type Topic } from "../lib/api";
import { daysAgoIso } from "../lib/format";
import { useApp } from "../stores/app";
import { toastError } from "../stores/toast";
import ArticleRow from "../components/ArticleRow";
import styles from "./pages.module.css";

const RANGES = [
  { days: 1, label: "Last 24 hours" },
  { days: 3, label: "Last 3 days" },
  { days: 7, label: "Last 7 days" },
  { days: 30, label: "Last 30 days" },
];

export default function Explore() {
  const newsVersion = useApp((s) => s.newsVersion);
  const [topics, setTopics] = useState<Topic[]>([]);
  const [feeds, setFeeds] = useState<Feed[]>([]);
  const [topicId, setTopicId] = useState<number | "">("");
  const [feedId, setFeedId] = useState<number | "">("");
  const [days, setDays] = useState(3);
  const [unreadOnly, setUnreadOnly] = useState(false);
  const [savedOnly, setSavedOnly] = useState(false);
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<ArticleListItem[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const sentinel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.listTopics().then(setTopics).catch(toastError);
    api.listFeeds().then(setFeeds).catch(toastError);
  }, []);

  // One string key for all filter values, so the loader only changes when a filter changes.
  const spec = JSON.stringify({ topicId, feedId, unreadOnly, savedOnly, query: query.trim(), days });

  const loadPage = useCallback(
    (c: string | null) => {
      const f = JSON.parse(spec) as {
        topicId: number | "";
        feedId: number | "";
        unreadOnly: boolean;
        savedOnly: boolean;
        query: string;
        days: number;
      };
      const filter: ArticleFilter = {
        topicId: f.topicId === "" ? undefined : f.topicId,
        feedId: f.feedId === "" ? undefined : f.feedId,
        unreadOnly: f.unreadOnly,
        savedOnly: f.savedOnly,
        query: f.query || undefined,
        since: f.savedOnly ? undefined : daysAgoIso(f.days),
      };
      setLoading(true);
      return api
        .listArticles(filter, c, 40)
        .then((p) => {
          setItems((prev) => (c ? [...prev, ...p.items] : p.items));
          setCursor(p.nextCursor);
        })
        .catch(toastError)
        .finally(() => setLoading(false));
    },
    [spec],
  );

  useEffect(() => {
    const t = setTimeout(() => void loadPage(null), query ? 250 : 0);
    return () => clearTimeout(t);
  }, [loadPage, query, newsVersion]);

  useEffect(() => {
    const el = sentinel.current;
    if (!el) return;
    const io = new IntersectionObserver((entries) => {
      if (entries[0].isIntersecting && cursor && !loading) void loadPage(cursor);
    });
    io.observe(el);
    return () => io.disconnect();
  }, [cursor, loading, loadPage]);

  return (
    <div className={styles.page}>
      <div className={styles.pageHeader}>
        <h1>Explore</h1>
        <span className="muted">All stories, best first</span>
      </div>
      <div className={styles.filters}>
        <input type="search" placeholder="Search titles…" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search titles" />
        <select value={topicId} onChange={(e) => setTopicId(e.target.value ? Number(e.target.value) : "")} aria-label="Topic">
          <option value="">All topics</option>
          {topics.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
        <select value={feedId} onChange={(e) => setFeedId(e.target.value ? Number(e.target.value) : "")} aria-label="Source">
          <option value="">All sources</option>
          {feeds.map((f) => (
            <option key={f.id} value={f.id}>
              {f.name}
            </option>
          ))}
        </select>
        <select value={days} onChange={(e) => setDays(Number(e.target.value))} aria-label="Time range" disabled={savedOnly}>
          {RANGES.map((r) => (
            <option key={r.days} value={r.days}>
              {r.label}
            </option>
          ))}
        </select>
        <label className={styles.check}>
          <input type="checkbox" checked={unreadOnly} onChange={(e) => setUnreadOnly(e.target.checked)} /> Unread
        </label>
        <label className={styles.check}>
          <input type="checkbox" checked={savedOnly} onChange={(e) => setSavedOnly(e.target.checked)} /> Saved
        </label>
      </div>

      {items.map((a) => (
        <ArticleRow key={a.id} a={a} onChange={() => void loadPage(null)} />
      ))}
      {!loading && items.length === 0 && (
        <div className={styles.empty}>No stories here. Try a longer time range or fewer filters.</div>
      )}
      {loading && <div className={styles.empty}>Loading…</div>}
      <div ref={sentinel} className={styles.sentinel} />
    </div>
  );
}
