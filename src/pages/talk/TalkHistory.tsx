import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { api, type Conversation, type ConversationTurn } from "../../lib/api";
import { formatDateTime } from "../../lib/format";
import { toastError } from "../../stores/toast";
import styles from "./talk.module.css";

const STATUS: Record<Conversation["reviewStatus"], string> = {
  pending: "Review to finish",
  done: "Reviewed",
  skipped: "Skipped",
};

const minutes = (c: Conversation) => Math.max(0, Math.round(c.userSpeakingSeconds / 6) / 10);

/** /talk/history: past conversations (SPEC §12.7). */
export default function TalkHistory() {
  const navigate = useNavigate();
  const [list, setList] = useState<Conversation[] | null>(null);
  useEffect(() => {
    api.listConversations().then(setList).catch(toastError);
  }, []);
  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <h1>Talk history</h1>
        <span className="spacer" />
        <Link to="/talk">New conversation</Link>
      </div>
      <div className={styles.card}>
        {list === null ? (
          <p className="muted">Loading…</p>
        ) : list.length === 0 ? (
          <p className="muted">No conversations yet.</p>
        ) : (
          <div className={styles.list}>
            {list.map((c) => (
              <div key={c.id} className={styles.listRow}>
                <div className={styles.listMain}>
                  <strong>{c.articleTitle ?? "Free talk"}</strong>
                  <span className="faint">
                    {formatDateTime(c.startedAt)} · {minutes(c)} min speaking · {c.corrections} correction
                    {c.corrections === 1 ? "" : "s"}
                  </span>
                </div>
                <span className={`${styles.badge} ${c.reviewStatus === "pending" ? styles.badgePending : ""}`}>
                  {STATUS[c.reviewStatus]}
                </span>
                {c.reviewStatus === "pending" && c.endedAt && (
                  <button onClick={() => void navigate(`/talk/review/${c.id}`)}>Finish review</button>
                )}
                <button className="ghost" onClick={() => void navigate(`/talk/history/${c.id}`)}>
                  Open
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** /talk/history/:id — the read-only transcript. */
export function TalkTranscript() {
  const { id } = useParams();
  const navigate = useNavigate();
  const [data, setData] = useState<{ conversation: Conversation; turns: ConversationTurn[] } | null>(null);
  useEffect(() => {
    api.getConversation(Number(id)).then(setData).catch(toastError);
  }, [id]);
  if (!data) return <div className={styles.page}>Loading…</div>;
  const c = data.conversation;
  return (
    <div className={`${styles.page} ${styles.readonly}`}>
      <div className={styles.header}>
        <h1>{c.articleTitle ?? "Free talk"}</h1>
        <span className="muted">{formatDateTime(c.startedAt)}</span>
        <span className="spacer" />
        {c.reviewStatus === "pending" && c.endedAt && (
          <button className="primary" onClick={() => void navigate(`/talk/review/${c.id}`)}>
            Finish review
          </button>
        )}
        <Link to="/talk/history">Back</Link>
      </div>
      <div className={styles.transcript}>
        {data.turns.map((t) =>
          t.role === "user" ? (
            <div key={t.id} className={`${styles.bubble} ${styles.user}`}>
              {t.text}
            </div>
          ) : (
            <div key={t.id}>
              <div className={`${styles.bubble} ${styles.tutor} ${t.meta?.local ? styles.local : ""}`}>{t.text}</div>
              {t.meta?.correction && (
                <div className={styles.correction} style={{ marginTop: 6 }}>
                  <span className={styles.tag}>You said</span>
                  <span className={styles.removed}>{t.meta.correction.original}</span>
                  <span />
                  <span className={styles.tag}>Better</span>
                  <span className={styles.added}>{t.meta.correction.corrected}</span>
                  <span />
                </div>
              )}
            </div>
          ),
        )}
      </div>
    </div>
  );
}
