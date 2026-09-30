import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { api, type ArticleListItem } from "../lib/api";
import { daysAgoIso, scoreLabel, timeAgo } from "../lib/format";
import { useApp } from "../stores/app";
import { toast, toastError } from "../stores/toast";
import ArticleRow from "../components/ArticleRow";
import { Breakdown } from "../components/ScoreChip";
import styles from "./pages.module.css";

function PickCard({ article, why, label }: { article: ArticleListItem; why?: string; label: string }) {
  const refreshPick = useApp((s) => s.refreshPick);
  const navigate = useNavigate();
  const after = () => void refreshPick();
  return (
    <div className={styles.twoCol}>
      <section className={styles.card} aria-label={label}>
        <div className={styles.eyebrow}>{label}</div>
        <h2 className={styles.pickTitle}>{article.title}</h2>
        <div className={styles.meta}>
          {article.primaryTopic && <strong>{article.primaryTopic}</strong>}
          <span>{article.sourceName}</span>
          <span>·</span>
          <span>{timeAgo(article.publishedAt ?? article.discoveredAt)}</span>
          {article.hnPoints != null && <span>· ▲ {article.hnPoints} on Hacker News</span>}
          {article.difficulty && (
            <span>
              · {article.difficulty[0].toUpperCase() + article.difficulty.slice(1)} English
              {article.readingMinutes ? ` · ${article.readingMinutes} min read` : ""}
            </span>
          )}
        </div>
        {article.description && <p className={styles.desc}>{article.description}</p>}
        {why && <div className={styles.why}>💡 {why}</div>}
        <div className={styles.actions}>
          <button className="primary" onClick={() => navigate(`/reader/${article.id}`)}>
            Read
          </button>
          <button className="ghost" onClick={() => api.openArticle(article.id).then(after).catch(toastError)}>
            Open in browser ↗
          </button>
          <button onClick={() => api.setSaved(article.id, !article.saved).then(after).catch(toastError)}>
            {article.saved ? "★ Saved" : "☆ Save"}
          </button>
          <button
            className="ghost"
            onClick={() =>
              api
                .recordInteraction(article.id, "not_interested")
                .then(() => {
                  toast("Hidden. You will see fewer stories like this.");
                  after();
                })
                .catch(toastError)
            }
          >
            Not interested
          </button>
        </div>
        <p className="faint" style={{ fontSize: 12, marginTop: 14, marginBottom: 0 }}>
          In the reader: B1 summary, Easy English, and an AI chat about the story.
        </p>
      </section>
      {article.breakdown && (
        <aside className={styles.card}>
          <div className={styles.eyebrow}>Score {scoreLabel(article.score)} / 100</div>
          <Breakdown b={article.breakdown} />
        </aside>
      )}
    </div>
  );
}

export default function Today() {
  const { settings, pick, preview, newsVersion } = useApp();
  const navigate = useNavigate();
  const [more, setMore] = useState<ArticleListItem[]>([]);

  const loadMore = useCallback(() => {
    api
      .listArticles({ since: daysAgoIso(2) }, null, 6)
      .then((p) => setMore(p.items))
      .catch(toastError);
  }, []);
  useEffect(loadMore, [loadMore, newsVersion]);

  const pickId = pick?.article.id ?? preview?.id;
  const others = more.filter((a) => a.id !== pickId).slice(0, 5);
  const beforePickTime = settings ? new Date().toTimeString().slice(0, 5) < settings.pickTime : false;

  return (
    <div className={styles.page}>
      <div className={styles.pageHeader}>
        <h1>Today</h1>
        <span className="muted">{new Date().toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" })}</span>
      </div>

      {pick ? (
        <PickCard label="Today's pick" article={pick.article} why={pick.why} />
      ) : preview ? (
        <PickCard
          label={beforePickTime ? `Preview · today's pick is chosen at ${settings?.pickTime}` : "Best story right now"}
          article={preview}
        />
      ) : (
        <div className={`${styles.card} ${styles.empty}`}>
          <p>No story matches your topics yet.</p>
          <p className="faint">Try Refresh news, or add more keywords in Settings › Topics.</p>
          <button onClick={() => navigate("/settings/topics")}>Edit topics</button>
        </div>
      )}

      {others.length > 0 && (
        <section className={styles.section}>
          <h2>Also interesting</h2>
          {others.map((a) => (
            <ArticleRow key={a.id} a={a} onChange={loadMore} />
          ))}
        </section>
      )}
    </div>
  );
}
