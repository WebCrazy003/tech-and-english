import { useEffect, useState } from "react";
import { Link, useLocation, useNavigate, useParams } from "react-router";
import { api, type SessionReview, type Suggestion } from "../../lib/api";
import { formatDateTime } from "../../lib/format";
import { useApp } from "../../stores/app";
import { toast, toastError } from "../../stores/toast";
import styles from "./talk.module.css";

const GROUPS: { title: string; kinds: Suggestion["kind"][] }[] = [
  { title: "Words & phrases", kinds: ["word", "phrase", "term"] },
  { title: "Corrections", kinds: ["correction"] },
  { title: "Useful sentences", kinds: ["sentence"] },
];
const KIND_LABEL: Partial<Record<Suggestion["kind"], string>> = { phrase: "phrase", term: "technical term" };

/** End-of-session review (SPEC §12.7): pick what goes into the Word Book. */
export default function TalkReview() {
  const { id } = useParams();
  const conversationId = Number(id);
  const navigate = useNavigate();
  const mode = useApp((s) => s.mode);
  const passed = useLocation().state as SessionReview | null;
  const [review, setReview] = useState<SessionReview | null>(passed?.conversationId === conversationId ? passed : null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (review) return;
    api.getSessionReview(conversationId).then(setReview).catch(toastError);
  }, [conversationId, review]);
  useEffect(() => {
    if (review) setPicked(new Set(review.suggestions.filter((s) => s.preselected).map((s) => s.id)));
  }, [review]);

  if (!review)
    return (
      <div className={styles.page}>
        <div className={styles.card}>
          <strong>Preparing your review…</strong>
          <p className="muted">The AI is choosing words and corrections from your conversation.</p>
        </div>
      </div>
    );

  const toggle = (s: Suggestion) => {
    const next = new Set(picked);
    if (next.has(s.id)) next.delete(s.id);
    else next.add(s.id);
    setPicked(next);
  };
  const finish = async (selected: Suggestion[]) => {
    setBusy(true);
    try {
      const n = await api.applySessionReview(conversationId, selected);
      toast(n ? `Added ${n} item${n === 1 ? "" : "s"} to your Word Book` : "Review skipped. Nothing was added.");
      void navigate("/talk/history");
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  const done = review.reviewStatus !== "pending";
  const s = review.stats;

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <h1>Conversation review</h1>
        <span className="muted">
          {review.articleTitle ?? "Free talk"} · {formatDateTime(review.startedAt)}
        </span>
      </div>
      <div className={styles.stats}>
        <div className={styles.stat}>
          <strong>{s.speakingMinutes}</strong>
          <span>minutes speaking</span>
        </div>
        <div className={styles.stat}>
          <strong>{s.newWords}</strong>
          <span>new words</span>
        </div>
        <div className={styles.stat}>
          <strong>{s.corrections}</strong>
          <span>corrections</span>
        </div>
        <div className={styles.stat}>
          <strong>{s.drills}</strong>
          <span>pronunciation drills</span>
        </div>
      </div>

      <div className={styles.card}>
        {review.suggestions.length === 0 ? (
          <p className="muted">Nothing new to save from this conversation. Well done!</p>
        ) : (
          GROUPS.map((g) => {
            const items = review.suggestions.filter((x) => g.kinds.includes(x.kind));
            if (!items.length) return null;
            return (
              <div key={g.title} className={styles.group}>
                <h2>{g.title}</h2>
                {items.map((x) => (
                  <label key={x.id} className={styles.suggestion}>
                    <input type="checkbox" checked={picked.has(x.id)} disabled={done} onChange={() => toggle(x)} />
                    <span>
                      <strong>{x.text}</strong>
                      {KIND_LABEL[x.kind] && <span className={styles.kind}>{KIND_LABEL[x.kind]}</span>}
                      {(x.meaningSimple || x.note) && <small>{x.meaningSimple ?? x.note}</small>}
                    </span>
                  </label>
                ))}
              </div>
            );
          })
        )}
        {review.source === "fallback" && review.suggestions.length > 0 && (
          <p className="faint" style={{ fontSize: 12 }}>
            Made without the AI (it was not available). Meanings are added later.
          </p>
        )}
        <div className={styles.startRow}>
          {done ? (
            <span className="muted">
              This review is finished ({review.reviewStatus}). <Link to="/wordbook">Open the Word Book</Link>
            </span>
          ) : (
            <>
              <button
                className="primary"
                disabled={busy || picked.size === 0 || mode === "hibernate"}
                title={mode === "hibernate" ? "Switch to Standard to add words" : undefined}
                onClick={() => void finish(review.suggestions.filter((x) => picked.has(x.id)))}
              >
                Add selected to Word Book ({picked.size})
              </button>
              <button disabled={busy} onClick={() => void finish([])}>
                Skip
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
