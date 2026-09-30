import { useNavigate } from "react-router";
import { api, type ArticleListItem } from "../lib/api";
import { timeAgo } from "../lib/format";
import { toast, toastError } from "../stores/toast";
import ScoreChip from "./ScoreChip";
import styles from "./components.module.css";

export default function ArticleRow({ a, onChange }: { a: ArticleListItem; onChange: () => void }) {
  const navigate = useNavigate();
  const open = () => navigate(`/reader/${a.id}`);
  const openBrowser = () => api.openArticle(a.id).then(onChange).catch(toastError);
  const toggleSave = () => api.setSaved(a.id, !a.saved).then(onChange).catch(toastError);
  const hide = () =>
    api
      .recordInteraction(a.id, "not_interested")
      .then(() => {
        toast("Hidden. You will see fewer stories like this.");
        onChange();
      })
      .catch(toastError);
  return (
    <article className={`${styles.row} ${a.readStatus !== "unread" ? styles.rowSeen : ""}`}>
      <ScoreChip score={a.score} breakdown={a.breakdown} />
      <div className={styles.rowMain}>
        <button className={styles.rowTitle} onClick={open} title="Read in the app">
          {a.title}
        </button>
        <div className={styles.rowMeta}>
          <span>{a.sourceName}</span>
          <span>·</span>
          <span>{timeAgo(a.publishedAt ?? a.discoveredAt)}</span>
          {a.readingMinutes != null && (
            <>
              <span>·</span>
              <span>{a.readingMinutes} min</span>
            </>
          )}
          {a.hnPoints != null && (
            <>
              <span>·</span>
              <span>
                ▲ {a.hnPoints} · 💬 {a.hnComments ?? 0}
              </span>
            </>
          )}
          {a.topics.slice(0, 3).map((t) => (
            <span key={t} className={styles.chip}>
              {t}
            </span>
          ))}
        </div>
      </div>
      <div className={styles.rowActions}>
        <button className="ghost" onClick={openBrowser} aria-label="Open in browser" title="Open in browser">
          ↗
        </button>
        <button className="ghost" onClick={toggleSave} aria-label={a.saved ? "Unsave" : "Save"} title={a.saved ? "Unsave" : "Save"}>
          {a.saved ? "★" : "☆"}
        </button>
        <button className="ghost" onClick={hide} aria-label="Not interested" title="Not interested">
          ✕
        </button>
      </div>
    </article>
  );
}
