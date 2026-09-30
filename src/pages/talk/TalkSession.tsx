import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { api, EVENTS, onEvent, type Correction, type CorrectionPolicy } from "../../lib/api";
import { clock } from "../../lib/format";
import { wordDiff } from "../../features/talk/diff";
import WordPopup from "../../features/words/WordPopup";
import { sentenceAt, type WordSelection } from "../../features/words/selection";
import { useApp } from "../../stores/app";
import {
  attachTalk,
  changeCorrection,
  changeRate,
  endTalk,
  micDown,
  micUp,
  onTalkEnded,
  practise,
  replayLast,
  sendTyped,
  setLevel,
  speakText,
  useTalk,
  type TalkMessage,
} from "../../stores/talk";
import { toastError } from "../../stores/toast";
import { MicHelp, POLICIES } from "./TalkSetup";
import styles from "./talk.module.css";

const LEVEL_NAMES = { 1: "Level 1", 2: "Level 2 · B1", 3: "Level 3" } as const;
const LOADING: Record<string, string> = {
  llm: "Starting the AI…",
  stt: "Starting speech recognition…",
  summary: "Reading the story…",
};

/** Tutor text with clickable words (→ word popup: Explain / Add / Practise saying). */
function TutorText({ text, onWord }: { text: string; onWord: (sel: WordSelection) => void }) {
  const parts = text.split(/(\s+)/);
  let offset = 0;
  return (
    <>
      {parts.map((p, i) => {
        const start = offset;
        offset += p.length;
        const term = p.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "");
        if (!term || /^\s+$/.test(p)) return p;
        return (
          <span
            key={i}
            className={styles.word}
            onClick={(e) => {
              const r = (e.target as HTMLElement).getBoundingClientRect();
              onWord({
                term,
                sentence: sentenceAt(text, start),
                rect: { left: r.left, top: r.top, bottom: r.bottom, width: r.width },
              });
            }}
          >
            {p}
          </span>
        );
      })}
    </>
  );
}

function CorrectionCard({ c }: { c: Correction }) {
  const d = wordDiff(c.original, c.corrected);
  return (
    <div className={styles.correction} aria-label="Correction">
      <span className={styles.tag}>You said</span>
      <span>
        {d.original.map((w, i) => (
          <span key={i} className={w.changed ? styles.removed : undefined}>
            {w.text}{" "}
          </span>
        ))}
      </span>
      <span />
      <span className={styles.tag}>Better</span>
      <span>
        {d.corrected.map((w, i) => (
          <span key={i} className={w.changed ? styles.added : undefined}>
            {w.text}{" "}
          </span>
        ))}
      </span>
      <button className="ghost" aria-label="Listen to the corrected sentence" onClick={() => speakText(c.corrected)}>
        🔊
      </button>
      {c.explanation && <span className={styles.explanation}>{c.explanation}</span>}
    </div>
  );
}

function Bubble({ m, onWord }: { m: TalkMessage; onWord: (sel: WordSelection) => void }) {
  if (m.role === "notice") return <div className={styles.notice}>{m.text}</div>;
  if (m.role === "user") return <div className={`${styles.bubble} ${styles.user}`}>{m.text}</div>;
  return (
    <>
      <div className={`${styles.bubble} ${styles.tutor} ${m.local ? styles.local : ""}`}>
        <span className={m.streaming ? styles.cursor : undefined}>
          <TutorText text={m.text} onWord={onWord} />
        </span>
      </div>
      {m.correction && <CorrectionCard c={m.correction} />}
    </>
  );
}

export default function TalkSession() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const settingsApp = useApp((s) => s.settings);
  const setMode = useApp((s) => s.setMode);
  const t = useTalk();
  const [sel, setSel] = useState<WordSelection | null>(null);
  const [typedOpen, setTypedOpen] = useState(false);
  const [typed, setTyped] = useState("");
  const [started] = useState(() => Date.now());
  const [now, setNow] = useState(Date.now());
  const endRef = useRef<HTMLDivElement>(null);

  // Attach to a running session after a reload; otherwise go back to the setup page.
  useEffect(() => {
    const s = useTalk.getState();
    if (s.conversationId === null && !s.starting) {
      attachTalk()
        .then((ok) => {
          if (!ok) void navigate("/talk", { replace: true });
        })
        .catch(toastError);
    }
  }, [navigate]);

  useEffect(
    () =>
      onTalkEnded((review, id) => {
        if (id !== null) void navigate(`/talk/review/${id}`, { replace: true, state: review });
      }),
    [navigate],
  );

  useEffect(() => {
    const unl = [
      onEvent<{ rms: number }>(EVENTS.voiceLevel, (p) => setLevel(p.rms)),
      onEvent<{ reason: string }>(EVENTS.voiceAutostop, () => void micUp()),
    ];
    const blur = () => void micUp();
    window.addEventListener("blur", blur);
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      unl.forEach((p) => void p.then((f) => f()));
      window.removeEventListener("blur", blur);
      clearInterval(tick);
    };
  }, []);

  // Hold Space to talk (not while typing in a text field).
  useEffect(() => {
    const typing = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable);
    };
    const down = (e: KeyboardEvent) => {
      if (e.code !== "Space" || typing(e)) return;
      e.preventDefault();
      if (!e.repeat) void micDown();
    };
    const up = (e: KeyboardEvent) => {
      if (e.code !== "Space" || typing(e)) return;
      e.preventDefault();
      void micUp();
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, []);

  const lastText = t.messages.at(-1)?.text;
  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [t.messages.length, lastText]);

  // Hibernate / Quit asked from the tray menu while talking (P5 §13).
  useEffect(() => {
    const confirmKind = params.get("confirm");
    if (!confirmKind) return;
    setParams({}, { replace: true });
    const question =
      confirmKind === "quit"
        ? "End the conversation and quit Tech English? You can finish the review later."
        : "End the conversation and switch to Hibernate? You can finish the review later.";
    if (!window.confirm(question)) return;
    void (async () => {
      if (confirmKind === "quit") {
        await endTalk("quit");
        await api.quitApp();
      } else {
        await endTalk("hibernate");
        await setMode("hibernate").catch(toastError);
        void navigate("/talk/history");
      }
    })();
  }, [params, setParams, setMode, navigate]);

  const status = t.starting
    ? t.loading.map((l) => LOADING[l]).filter(Boolean).at(-1) ?? "Loading AI…"
    : t.recording
      ? "Listening… release to send"
      : t.transcribing
        ? "Transcribing…"
        : t.speaking
          ? "Speaking…"
          : t.waiting
            ? "Thinking…"
            : "Your turn";
  const busy = t.starting || t.ending || t.conversationId === null;
  const meter = Math.min(100, Math.round((Math.sqrt(t.level) / 0.5) * 100));

  const submitTyped = () => {
    const text = typed;
    setTyped("");
    void sendTyped(text);
  };

  return (
    <div className={styles.session}>
      <div className={styles.sessionHead}>
        <span className={styles.sessionTitle} title={t.articleTitle ?? undefined}>
          🎙 {t.articleTitle ?? "Free talk"}
        </span>
        {t.settings && <span className={styles.chip}>{LEVEL_NAMES[t.settings.level]}</span>}
        <span className={styles.timer}>{clock((now - started) / 1000)}</span>
        {t.settings && (
          <span className={styles.speed}>
            Speed
            <button aria-label="Slower" onClick={() => void changeRate(-0.05)}>
              −
            </button>
            {t.settings.rate.toFixed(2)}×
            <button aria-label="Faster" onClick={() => void changeRate(0.05)}>
              +
            </button>
          </span>
        )}
        {t.settings && (
          <select
            aria-label="Corrections"
            value={t.settings.correction}
            onChange={(e) => void changeCorrection(e.target.value as CorrectionPolicy)}
          >
            {POLICIES.map((p) => (
              <option key={p.value} value={p.value}>
                Corrections: {p.label}
              </option>
            ))}
          </select>
        )}
        {settingsApp?.debug.keepAudio && <span className={styles.debugBadge}>● Recordings saved (debug)</span>}
        <button className="danger" disabled={busy} onClick={() => void endTalk("user")}>
          End
        </button>
      </div>

      {t.micDenied && <MicHelp />}

      <div className={styles.transcript} aria-live="polite">
        {t.messages.map((m) => (
          <Bubble key={m.key} m={m} onWord={setSel} />
        ))}
        <div ref={endRef} />
      </div>

      {t.phase === "awaitRepeat" && t.repeatTarget && (
        <div className={styles.banner}>
          <span>🔁</span>
          <strong>Please say: {t.repeatTarget}</strong>
          <button className="ghost" aria-label="Listen again" onClick={() => speakText(t.repeatTarget!, (t.settings?.rate ?? 0.85) - 0.15)}>
            🔊
          </button>
        </div>
      )}
      {t.phase === "drill" && t.drill && (
        <div className={`${styles.banner} ${styles.drill}`}>
          <span className={styles.drillWord}>{t.drill.word}</span>
          <button className="ghost" aria-label="Listen" onClick={() => speakText(t.drill!.word, 0.6)}>
            🔊
          </button>
          <span className={styles.hint}>{t.drill.hint ? `hint: ${t.drill.hint}` : ""}</span>
          <span className="spacer" />
          <span className={styles.dots} aria-label={`Attempt ${t.drill.attempt} of ${t.drill.maxAttempts}`}>
            {Array.from({ length: t.drill.maxAttempts }, (_, i) => (
              <span key={i} className={i < t.drill!.attempt - 1 ? styles.done : undefined} />
            ))}
          </span>
        </div>
      )}

      <div className={styles.bottom}>
        <button
          className={`${styles.mic} ${t.recording ? styles.micOn : ""}`}
          aria-label={t.recording ? "Stop recording" : "Start recording"}
          aria-pressed={t.recording}
          disabled={busy}
          onClick={() => void (t.recording ? micUp() : micDown())}
        >
          {t.recording ? "■" : "🎙"}
        </button>
        <div className={styles.meter} aria-hidden>
          <span style={{ width: `${t.recording ? meter : 0}%` }} />
        </div>
        <div className={styles.status} role="status">
          {status}
          {!busy && !t.recording && (
            <>
              {" · "}
              <button className="ghost" onClick={replayLast} disabled={!t.lastReply.length}>
                ↺ Say again
              </button>
              <button className="ghost" onClick={() => setTypedOpen(!typedOpen)}>
                ⌨ Type
              </button>
            </>
          )}
        </div>
        {typedOpen && (
          <div className={styles.typed}>
            <input
              type="text"
              value={typed}
              placeholder="Type what you want to say…"
              onChange={(e) => setTyped(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && submitTyped()}
              aria-label="Type your answer"
            />
            <button onClick={submitTyped} disabled={!typed.trim() || busy}>
              Send
            </button>
          </div>
        )}
      </div>

      {sel && (
        <WordPopup
          sel={sel}
          articleId={null}
          conversationId={t.conversationId}
          onClose={() => setSel(null)}
          onPractise={(w) => void practise(w)}
        />
      )}

      {(t.starting || t.ending) && (
        <div className={styles.overlay}>
          <div className={styles.card}>
            <strong>{t.ending ? "Preparing your review…" : status}</strong>
            <p className="muted">
              {t.ending
                ? "The AI is choosing words and corrections for your Word Book."
                : "The first start takes up to a minute. Next time it is faster."}
            </p>
          </div>
        </div>
      )}
    </div>
  );
}
