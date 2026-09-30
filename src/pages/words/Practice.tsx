import { useCallback, useEffect, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router";
import { api, type Grade, type QuizCard, type QuizResult } from "../../lib/api";
import { useApp } from "../../stores/app";
import { toastError } from "../../stores/toast";
import { useSpeech, useWords } from "../../stores/words";
import styles from "./words.module.css";

const SIZES = [5, 10, 15, 20, 30];
const pct = (x: number) => `${Math.round(x * 100)}%`;

interface QuizState {
  sessionId: number;
  cards: QuizCard[];
}

/** /practice: counts, size, recent scores, Start. */
export function PracticeStart() {
  const navigate = useNavigate();
  const settings = useApp((s) => s.settings);
  const mode = useApp((s) => s.mode);
  const due = useWords((s) => s.due);
  const [size, setSize] = useState<number | null>(null);
  const [history, setHistory] = useState<QuizResult[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void useWords.getState().refresh();
    api.listQuizHistory(5).then(setHistory).catch(toastError);
  }, []);

  const n = size ?? settings?.learning.quizSize ?? 10;
  const reason =
    mode === "hibernate"
      ? "Switch to Standard to practise."
      : due && due.ready < 3
        ? "Add a few more words first (at least 3 with a meaning)."
        : null;

  const start = async () => {
    setBusy(true);
    try {
      const q = await api.startQuiz(n);
      navigate("/practice/session", { state: q });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <h1>Practice</h1>
      </div>
      <div className={styles.card}>
        <div className={styles.bigNumbers}>
          <div>
            <strong>{due?.due ?? "–"}</strong>
            <span>due</span>
          </div>
          <div>
            <strong>{due?.new ?? "–"}</strong>
            <span>new</span>
          </div>
          <div>
            <strong>{due?.total ?? "–"}</strong>
            <span>in your Word Book</span>
          </div>
        </div>
        <div className="row" style={{ gap: 12, marginTop: 16 }}>
          <label className={styles.check}>
            Words per quiz
            <select value={n} onChange={(e) => setSize(Number(e.target.value))}>
              {SIZES.map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </select>
          </label>
          <span className="spacer" />
          {reason && <span className="faint">{reason}</span>}
          <button className="primary" onClick={() => void start()} disabled={busy || !!reason}>
            Start
          </button>
        </div>
        <p className="faint" style={{ fontSize: 12, marginBottom: 0 }}>
          Words you forgot recently come first, then unsure and due words, then a few new ones. Keys: Space shows the
          answer, 1 / 2 / 3 grade it.
        </p>
      </div>

      {history.length > 0 && (
        <>
          <h2 className={styles.h2}>Last quizzes</h2>
          <div className={styles.historyBars}>
            {history
              .slice()
              .reverse()
              .map((h) => (
                <div key={h.sessionId} title={`${pct(h.score)} · ${h.total} words · ${h.finishedAt ? new Date(h.finishedAt).toLocaleDateString() : ""}`}>
                  <div className={styles.bar} style={{ height: `${Math.max(4, h.score * 80)}px` }} />
                  <span>{pct(h.score)}</span>
                </div>
              ))}
          </div>
        </>
      )}
    </div>
  );
}

/** /practice/session: the card flow (SPEC §10.3). */
export function PracticeSession() {
  const navigate = useNavigate();
  const quiz = useLocation().state as QuizState | null;
  const settings = useApp((s) => s.settings);
  const speech = useSpeech();
  const [i, setI] = useState(0);
  const [shown, setShown] = useState(false);
  const [busy, setBusy] = useState(false);
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!quiz) navigate("/practice", { replace: true });
  }, [quiz, navigate]);

  const card = quiz?.cards[i];

  const show = useCallback(() => {
    if (!card || shown) return;
    setShown(true);
    if (settings?.learning.autoPronounce) speech.word(card.text);
  }, [card, shown, settings, speech]);

  const grade = useCallback(
    async (g: Grade) => {
      if (!quiz || !card || !shown || busy) return;
      setBusy(true);
      try {
        await api.gradeQuizItem(quiz.sessionId, card.itemId, g);
        if (i + 1 < quiz.cards.length) {
          setI(i + 1);
          setShown(false);
        } else {
          const result = await api.finishQuiz(quiz.sessionId);
          navigate("/practice/result", { state: result, replace: true });
        }
      } catch (e) {
        toastError(e);
      } finally {
        setBusy(false);
      }
    },
    [quiz, card, shown, busy, i, navigate],
  );

  const quit = useCallback(() => {
    if (window.confirm("Stop this quiz? The words you already graded are saved.")) navigate("/practice");
  }, [navigate]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof Element && e.target.closest("input,textarea,select")) return;
      if (e.key === " ") {
        e.preventDefault();
        show();
      } else if (e.key === "1") void grade("forgot");
      else if (e.key === "2") void grade("unsure");
      else if (e.key === "3") void grade("remember");
      else if (e.key === "Escape") quit();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [show, grade, quit]);

  useEffect(() => cardRef.current?.focus(), [i]);

  if (!quiz || !card) return null;
  const fixes = card.kind === "sentence" || card.kind === "correction";

  return (
    <div className={styles.page}>
      <div className={styles.quizTop}>
        <span className="muted">
          Question {i + 1} / {quiz.cards.length}
        </span>
        <div className={styles.progress} aria-hidden>
          <div style={{ width: `${(i / quiz.cards.length) * 100}%` }} />
        </div>
        <button className="ghost" onClick={quit}>
          Quit (Esc)
        </button>
      </div>

      <div className={`${styles.card} ${styles.quizCard}`} ref={cardRef} tabIndex={-1} aria-live="polite">
        {fixes ? (
          <>
            <p className="muted">{card.prompt}</p>
            <p className={styles.original}>{card.original}</p>
          </>
        ) : (
          <div className={styles.prompt}>
            {card.prompt}
            <button className="ghost" onClick={() => speech.word(card.text)} disabled={speech.disabled} aria-label="Listen">
              🔊
            </button>
          </div>
        )}

        {!shown ? (
          <>
            <p className="faint">Think about the answer.</p>
            <button className="primary" onClick={show}>
              Show answer <kbd>Space</kbd>
            </button>
          </>
        ) : (
          <>
            <hr />
            <div className={styles.answer}>
              <p className={styles.meaning}>
                {card.answer.meaning}
                {card.answer.partOfSpeech && <em className="faint"> · {card.answer.partOfSpeech}</em>}
              </p>
              {card.answer.ipa && <p className="faint">/{card.answer.ipa}/</p>}
              {card.answer.example && <p>Example: {card.answer.example}</p>}
              {card.answer.context && <p className="faint">“{card.answer.context}”</p>}
            </div>
            <div className={styles.grades}>
              <button className={styles.forgot} onClick={() => void grade("forgot")} disabled={busy}>
                Forgot <kbd>1</kbd>
              </button>
              <button className={styles.unsure} onClick={() => void grade("unsure")} disabled={busy}>
                Not sure <kbd>2</kbd>
              </button>
              <button className={styles.remember} onClick={() => void grade("remember")} disabled={busy}>
                Remember <kbd>3</kbd>
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** /practice/result */
export function PracticeResult() {
  const navigate = useNavigate();
  const r = useLocation().state as QuizResult | null;
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!r) navigate("/practice", { replace: true });
  }, [r, navigate]);
  if (!r) return null;

  const again = async () => {
    setBusy(true);
    try {
      const q = await api.startQuiz(null, r.missed.map((m) => m.id));
      navigate("/practice/session", { state: q });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={styles.page}>
      <div className={styles.card} style={{ textAlign: "center" }}>
        <div className={styles.score}>{pct(r.score)}</div>
        <p className="muted">
          ✓ {r.remember} remembered · ~ {r.unsure} not sure · ✗ {r.forgot} forgot
        </p>
      </div>
      {r.missed.length > 0 && (
        <>
          <h2 className={styles.h2}>Practise these again</h2>
          <div className={styles.list}>
            {r.missed.map((m) => (
              <button key={m.id} className={`${styles.item} ${styles.itemSimple}`} onClick={() => navigate(`/wordbook/${m.id}`)}>
                <span className={styles.itemMain}>
                  <span className={styles.itemText}>{m.text}</span>
                  <span className={styles.itemMeaning}>{m.meaningSimple ?? m.meaningB1}</span>
                </span>
              </button>
            ))}
          </div>
        </>
      )}
      <div className="row" style={{ marginTop: 16 }}>
        <span className="spacer" />
        {r.missed.length > 0 && (
          <button onClick={() => void again()} disabled={busy}>
            Practise missed again
          </button>
        )}
        <button className="primary" onClick={() => navigate("/practice")}>
          Done
        </button>
      </div>
    </div>
  );
}
