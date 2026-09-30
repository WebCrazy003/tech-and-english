import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import {
  ApiError,
  api,
  type ArticleListItem,
  type Conversation,
  type CorrectionPolicy,
  type SessionSettings,
  type VoiceSetup,
} from "../../lib/api";
import { formatDateTime } from "../../lib/format";
import { listEnglishVoices, pickDefaultVoice, voiceGroups } from "../../lib/tts";
import { AiSetupCard, DownloadButton } from "../../features/ai/AiSetup";
import { useAi } from "../../stores/ai";
import { useApp } from "../../stores/app";
import { startTalk } from "../../stores/talk";
import { toastError } from "../../stores/toast";
import styles from "./talk.module.css";

export const LEVELS: { level: 1 | 2 | 3; label: string; rate: number }[] = [
  { level: 1, label: "Level 1 · Very easy", rate: 0.7 },
  { level: 2, label: "Level 2 · B1", rate: 0.85 },
  { level: 3, label: "Level 3 · Natural", rate: 1.0 },
];
export const POLICIES: { value: CorrectionPolicy; label: string; help: string }[] = [
  { value: "low", label: "Low", help: "Only mistakes that change the meaning." },
  { value: "medium", label: "Medium", help: "Important grammar mistakes, and mistakes you repeat." },
  { value: "high", label: "High", help: "Any clear grammar or word-choice mistake." },
];

type TopicChoice = "story" | "lesson" | "article" | "free";

/** Mic permission help (SPEC §12.1): how to allow it in System Settings. */
export function MicHelp() {
  return (
    <div className={`${styles.card} ${styles.problem} ${styles.help}`}>
      <strong>The microphone is not allowed</strong>
      <ol>
        <li>Open System Settings › Privacy &amp; Security › Microphone.</li>
        <li>Turn on Tech English (in development: the Terminal or app that runs it).</li>
        <li>Come back here and try again.</li>
      </ol>
      <button
        onClick={() =>
          void api.openExternal("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone").catch(toastError)
        }
      >
        Open Microphone settings
      </button>
    </div>
  );
}

export default function TalkSetup() {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const { mode, pick, lesson, setMode } = useApp();
  const overview = useAi((s) => s.overview);
  const [setup, setSetup] = useState<VoiceSetup | null>(null);
  const [form, setForm] = useState<SessionSettings | null>(null);
  const [topic, setTopic] = useState<TopicChoice>("story");
  const [articleId, setArticleId] = useState<number | null>(null);
  const [articles, setArticles] = useState<ArticleListItem[]>([]);
  const [voices, setVoices] = useState<SpeechSynthesisVoice[]>([]);
  const [pending, setPending] = useState<Conversation[]>([]);
  const [busy, setBusy] = useState(false);
  const [lowMemory, setLowMemory] = useState<string | null>(null);

  useEffect(() => {
    api
      .voiceSetup()
      .then((s) => {
        if (s.active) {
          void navigate("/talk/session", { replace: true });
          return;
        }
        setSetup(s);
        setForm(s.settings);
      })
      .catch(toastError);
    void listEnglishVoices().then(setVoices);
    api
      .listArticles({}, null, 40)
      .then((p) => setArticles(p.items))
      .catch(() => {});
    api
      .listConversations()
      .then((l) => setPending(l.filter((c) => c.reviewStatus === "pending" && c.endedAt)))
      .catch(() => {});
    void useAi.getState().refresh().catch(() => {});
  }, [navigate]);

  // "Talk about this" from the Reader: /talk?article=12
  useEffect(() => {
    const a = Number(params.get("article"));
    if (a) {
      setTopic("article");
      setArticleId(a);
    } else if (!pick && lesson) setTopic("lesson");
    else if (!pick) setTopic("free");
  }, [params, pick, lesson]);

  const groups = useMemo(() => voiceGroups(voices), [voices]);

  const sttModel = overview?.models.find((m) => m.role === "stt" && m.default);
  const chatReady = overview?.models.some((m) => m.role === "chat" && m.downloaded) ?? true;
  const sttReady = setup?.sttModel || sttModel?.downloaded;

  const chosenArticle =
    topic === "story" ? (pick?.article.id ?? null) : topic === "lesson" ? (lesson?.article.id ?? null) : topic === "article" ? articleId : null;

  const blocked =
    mode === "hibernate"
      ? "Switch to Standard to talk."
      : !chatReady
        ? "Download an AI model first."
        : !sttReady
          ? "Download the speech model first."
          : !setup?.sttEngine
            ? "Install the speech engine first."
            : topic === "article" && !articleId
              ? "Choose an article."
              : null;

  const start = async (force = false) => {
    if (!form) return;
    setBusy(true);
    setLowMemory(null);
    try {
      void navigate("/talk/session");
      await startTalk(chosenArticle, form, force);
    } catch (e) {
      if (e instanceof ApiError && e.code === "low_memory") {
        void navigate("/talk");
        setLowMemory(e.message);
      } else {
        void navigate("/talk");
        toastError(e);
      }
    } finally {
      setBusy(false);
    }
  };

  if (!form || !setup) return <div className={styles.page}>Loading…</div>;
  const set = (patch: Partial<SessionSettings>) => setForm({ ...form, ...patch });
  const fallbackVoice = pickDefaultVoice(voices);

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <h1>Talk</h1>
        <span className="muted">Speak English with your tutor about a story.</span>
        <span className="spacer" />
        <Link to="/talk/history">History</Link>
      </div>

      {mode === "hibernate" && (
        <div className={`${styles.card} ${styles.problem}`}>
          <strong>Talk is off in Hibernate</strong>
          <p>The tutor needs the AI and the speech engine.</p>
          <button className="primary" onClick={() => void setMode("standard").catch(toastError)}>
            Switch to Standard
          </button>
        </div>
      )}
      {mode === "standard" && !chatReady && (
        <div className={styles.card}>
          <AiSetupCard />
        </div>
      )}
      {mode === "standard" && !sttReady && sttModel && (
        <div className={`${styles.card} ${styles.problem}`}>
          <strong>Download the speech model</strong>
          <p>
            It turns your voice into text, on this Mac. {sttModel.displayName}, license {sttModel.license}.
          </p>
          <DownloadButton m={sttModel} />
        </div>
      )}
      {mode === "standard" && !setup.sttEngine && (
        <div className={`${styles.card} ${styles.problem}`}>
          <strong>The speech engine (whisper-server) was not found</strong>
          <p>
            Install it once in Terminal: <code>brew install whisper.cpp</code> — then open this page again.
          </p>
        </div>
      )}
      {setup.mic === "denied" && <MicHelp />}
      {lowMemory && (
        <div className={`${styles.card} ${styles.problem}`}>
          <strong>Low memory</strong>
          <p>{lowMemory} Close some apps, or start anyway (your Mac may become slow).</p>
          <button onClick={() => void start(true)}>Start anyway</button>
        </div>
      )}

      <div className={styles.card}>
        <div className={styles.form}>
          <span className={styles.formLabel}>Topic</span>
          <div className={styles.topics} role="radiogroup" aria-label="Topic">
            <label>
              <input type="radio" checked={topic === "story"} disabled={!pick} onChange={() => setTopic("story")} />
              Today’s story{pick ? `: ${pick.article.title}` : " (not ready yet)"}
            </label>
            {lesson && (
              <label>
                <input type="radio" checked={topic === "lesson"} onChange={() => setTopic("lesson")} />
                Today’s lesson: {lesson.article.title}
              </label>
            )}
            <label>
              <input type="radio" checked={topic === "article"} onChange={() => setTopic("article")} />
              Another story
            </label>
            {topic === "article" && (
              <select
                aria-label="Story"
                value={articleId ?? ""}
                onChange={(e) => setArticleId(Number(e.target.value) || null)}
              >
                <option value="">Choose a story…</option>
                {articleId && !articles.some((a) => a.id === articleId) && <option value={articleId}>This story</option>}
                {articles.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.title}
                  </option>
                ))}
              </select>
            )}
            <label>
              <input type="radio" checked={topic === "free"} onChange={() => setTopic("free")} />
              Free talk about your topics
            </label>
          </div>

          <label htmlFor="level">English level</label>
          <select
            id="level"
            value={form.level}
            onChange={(e) => {
              const l = LEVELS.find((x) => x.level === Number(e.target.value))!;
              set({ level: l.level, rate: l.rate });
            }}
          >
            {LEVELS.map((l) => (
              <option key={l.level} value={l.level}>
                {l.label}
              </option>
            ))}
          </select>

          <label htmlFor="rate">Speaking speed</label>
          <div className={styles.inline}>
            <input
              id="rate"
              type="range"
              min={0.5}
              max={1.2}
              step={0.05}
              value={form.rate}
              onChange={(e) => set({ rate: Number(e.target.value) })}
            />
            <span className="muted">{form.rate.toFixed(2)}×</span>
          </div>

          <label htmlFor="pause">Pause between sentences</label>
          <div className={styles.inline}>
            <input
              id="pause"
              type="range"
              min={0}
              max={1500}
              step={100}
              value={form.pauseMs}
              onChange={(e) => set({ pauseMs: Number(e.target.value) })}
            />
            <span className="muted">{(form.pauseMs / 1000).toFixed(1)} s</span>
          </div>

          <span className={styles.formLabel}>Corrections</span>
          <div className={styles.inline}>
            <div className={styles.segmented} role="radiogroup" aria-label="Corrections">
              {POLICIES.map((p) => (
                <button
                  key={p.value}
                  role="radio"
                  aria-checked={form.correction === p.value}
                  onClick={() => set({ correction: p.value })}
                >
                  {p.label}
                </button>
              ))}
            </div>
            <span className="faint">{POLICIES.find((p) => p.value === form.correction)?.help}</span>
          </div>

          <label htmlFor="voice">Voice</label>
          <select id="voice" value={form.voiceUri ?? ""} onChange={(e) => set({ voiceUri: e.target.value || null })}>
            <option value="">Automatic{fallbackVoice ? ` (${fallbackVoice.name})` : ""}</option>
            {groups.map(([accent, list]) => (
              <optgroup key={accent} label={accent}>
                {list.map((v) => (
                  <option key={v.voiceURI} value={v.voiceURI}>
                    {v.name}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        </div>
        <div className={styles.startRow}>
          <button className="primary" disabled={!!blocked || busy} onClick={() => void start()} title={blocked ?? undefined}>
            🎙 Start conversation
          </button>
          <span className="faint">{blocked ?? "Hold Space (or click the mic) while you speak."}</span>
        </div>
      </div>

      {pending.length > 0 && (
        <div className={styles.card}>
          <strong>Reviews to finish</strong>
          <div className={styles.list}>
            {pending.slice(0, 5).map((c) => (
              <div key={c.id} className={styles.listRow}>
                <div className={styles.listMain}>
                  <strong>{c.articleTitle ?? "Free talk"}</strong>
                  <span className="faint">{formatDateTime(c.startedAt)}</span>
                </div>
                <button onClick={() => void navigate(`/talk/review/${c.id}`)}>Finish review</button>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
