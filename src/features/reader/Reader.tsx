import { useEffect, useRef, useState, type MouseEvent } from "react";
import { useNavigate, useParams } from "react-router";
import { api, type DerivKind, type ReaderArticle } from "../../lib/api";
import { ensureBody } from "../../lib/extract";
import { timeAgo } from "../../lib/format";
import { useApp } from "../../stores/app";
import { useReader } from "../../stores/reader";
import { toast, toastError } from "../../stores/toast";
import { useSettingsPatch } from "../../pages/settings/useSettingsPatch";
import AiPanel from "./AiPanel";
import DerivativeView from "./DerivativeView";
import styles from "./reader.module.css";

type Tab = "original" | DerivKind;

const TABS: { id: Tab; label: string }[] = [
  { id: "original", label: "Original" },
  { id: "summary_b1", label: "B1 Summary" },
  { id: "easy_english", label: "Easy English" },
];

const DIFFICULTY: Record<string, string> = { easy: "Easy English", medium: "Medium English", hard: "Hard English" };

/** Records "read" once: after scrolling 60 % of the text, or after half the reading time. */
function useReadTracking(article: ReaderArticle | null, scrollRef: React.RefObject<HTMLDivElement | null>) {
  const done = useRef(false);
  useEffect(() => {
    done.current = false;
  }, [article?.id]);
  useEffect(() => {
    if (!article || article.bodyStatus !== "ok") return;
    const mark = () => {
      if (done.current) return;
      done.current = true;
      void api.recordInteraction(article.id, "read").catch(() => {});
    };
    const el = scrollRef.current;
    const onScroll = () => {
      if (!el) return;
      const ratio = (el.scrollTop + el.clientHeight) / Math.max(1, el.scrollHeight);
      if (ratio >= 0.6) mark();
    };
    el?.addEventListener("scroll", onScroll);
    // Visible time only (the timer pauses while the window is hidden).
    let visibleMs = 0;
    const need = Math.max(1, article.readingMinutes ?? 1) * 30_000;
    const t = setInterval(() => {
      if (document.visibilityState === "visible") visibleMs += 5000;
      if (visibleMs >= need) mark();
    }, 5000);
    return () => {
      el?.removeEventListener("scroll", onScroll);
      clearInterval(t);
    };
  }, [article, scrollRef]);
}

export default function Reader() {
  const { id } = useParams();
  const articleId = Number(id);
  const navigate = useNavigate();
  const settings = useApp((s) => s.settings);
  const patch = useSettingsPatch();
  const setQuote = useReader((s) => s.setQuote);
  const [article, setArticle] = useState<ReaderArticle | null>(null);
  const [loading, setLoading] = useState(true);
  const [tab, setTab] = useState<Tab>("original");
  const [chip, setChip] = useState<{ text: string; x: number; y: number } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const panelOpen = settings?.readerAiPanelOpen ?? true;

  useEffect(() => {
    if (!Number.isFinite(articleId)) return;
    setLoading(true);
    setTab("original");
    setChip(null);
    scrollRef.current?.scrollTo({ top: 0 });
    void api.recordInteraction(articleId, "opened").catch(() => {});
    ensureBody(articleId)
      .then(setArticle)
      .catch((e) => {
        toastError(e);
        void api.getReaderArticle(articleId).then(setArticle).catch(() => {});
      })
      .finally(() => setLoading(false));
  }, [articleId]);

  useReadTracking(article, scrollRef);

  // Links in the article open in the browser; selecting text offers "Ask AI about this".
  const onBodyClick = (e: MouseEvent) => {
    const a = (e.target as HTMLElement).closest("a");
    if (a?.href) {
      e.preventDefault();
      if (!window.getSelection()?.toString()) void api.openExternal(a.href);
    }
  };
  const onMouseUp = () => {
    const sel = window.getSelection();
    const text = sel?.toString().trim() ?? "";
    if (!sel || !text || text.length > 1000 || sel.rangeCount === 0) {
      setChip(null);
      return;
    }
    const rect = sel.getRangeAt(0).getBoundingClientRect();
    const host = scrollRef.current?.getBoundingClientRect();
    if (!host) return;
    setChip({ text, x: rect.left - host.left + rect.width / 2, y: rect.top - host.top + (scrollRef.current?.scrollTop ?? 0) - 36 });
  };
  const askAi = () => {
    if (!chip) return;
    setQuote(chip.text);
    setChip(null);
    window.getSelection()?.removeAllRanges();
    if (!panelOpen) void patch({ readerAiPanelOpen: true });
  };

  const a = article;
  const meta = a
    ? [a.sourceName, timeAgo(a.publishedAt ?? a.discoveredAt), a.difficulty && DIFFICULTY[a.difficulty], a.readingMinutes && `${a.readingMinutes} min read`]
        .filter(Boolean)
        .join(" · ")
    : "";

  return (
    <div className={styles.shell}>
      <div className={styles.main} ref={scrollRef} onMouseUp={onMouseUp}>
        <div className={styles.column}>
          <div className={styles.topBar}>
            <button className="ghost" onClick={() => navigate(-1)} aria-label="Back">
              ← Back
            </button>
            <span className="spacer" />
            {a && (
              <>
                <button className="ghost" onClick={() => api.openExternal(a.url)} title="Open in browser">
                  Open in browser ↗
                </button>
                <button
                  className="ghost"
                  onClick={() => api.setSaved(a.id, !a.saved).then(() => setArticle({ ...a, saved: !a.saved })).catch(toastError)}
                >
                  {a.saved ? "★ Saved" : "☆ Save"}
                </button>
                <button
                  className="ghost"
                  onClick={() =>
                    api
                      .recordInteraction(a.id, "not_interested")
                      .then(() => {
                        toast("Hidden. You will see fewer stories like this.");
                        navigate(-1);
                      })
                      .catch(toastError)
                  }
                >
                  Not interested
                </button>
              </>
            )}
            {!panelOpen && (
              <button onClick={() => patch({ readerAiPanelOpen: true })} title="Show AI panel">
                ✦ AI
              </button>
            )}
          </div>

          {loading && !a && <p className={styles.status}>Loading the story…</p>}
          {a && (
            <>
              <h1 className={styles.title}>{a.title}</h1>
              <div className={styles.meta}>
                {meta}
                {a.topics.slice(0, 3).map((t) => (
                  <span key={t} className={styles.topic}>
                    {t}
                  </span>
                ))}
              </div>

              <div className={styles.tabs} role="tablist" aria-label="Reading mode">
                {TABS.map((t) => (
                  <button
                    key={t.id}
                    role="tab"
                    aria-selected={tab === t.id}
                    className={tab === t.id ? styles.tabOn : ""}
                    onClick={() => setTab(t.id)}
                  >
                    {t.label}
                  </button>
                ))}
              </div>

              {tab === "original" &&
                (loading ? (
                  <p className={styles.status}>Getting the article text…</p>
                ) : a.bodyStatus === "ok" && a.bodyHtml ? (
                  <article
                    className={styles.body}
                    onClick={onBodyClick}
                    // Sanitized by DOMPurify during extraction (only safe tags/attributes kept).
                    dangerouslySetInnerHTML={{ __html: a.bodyHtml }}
                  />
                ) : (
                  <div className={styles.fallback}>
                    <p>
                      {a.bodyStatus === "paywalled"
                        ? "This story is behind a paywall, so the app can't show the full text."
                        : "We couldn't load the full article."}
                    </p>
                    {a.description && <p className="muted">{a.description}</p>}
                    <button className="primary" onClick={() => api.openExternal(a.url)}>
                      Open in browser ↗
                    </button>
                    <p className="faint">You can still ask the AI about it — it uses the title and description.</p>
                  </div>
                ))}
              {tab !== "original" && <DerivativeView key={`${a.id}-${tab}`} articleId={a.id} kind={tab} />}
            </>
          )}
          {chip && (
            <button className={styles.askChip} style={{ left: chip.x, top: chip.y }} onMouseDown={(e) => e.preventDefault()} onClick={askAi}>
              ✦ Ask AI about this
            </button>
          )}
        </div>
      </div>
      {panelOpen && a && <AiPanel articleId={a.id} onClose={() => patch({ readerAiPanelOpen: false })} />}
    </div>
  );
}
