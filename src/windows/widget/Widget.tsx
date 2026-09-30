import { useEffect, useRef, useState } from "react";
import { ApiError, api, EVENTS, onEvent, type ArticleListItem, type Mode } from "../../lib/api";
import { shortAge, timeAgo } from "../../lib/format";
import { useApp } from "../../stores/app";
import { AiDot } from "../../features/ai/AiSetup";
import { toastError } from "../../stores/toast";
import { useWords } from "../../stores/words";
import styles from "./Widget.module.css";

/** Close a popover when the user clicks outside it or presses Escape. */
function useDismiss(open: boolean, close: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, close]);
  return ref;
}

function ModeBadge({ mode }: { mode: Mode }) {
  const [open, setOpen] = useState(false);
  const ref = useDismiss(open, () => setOpen(false));
  const setMode = useApp((s) => s.setMode);
  const choose = (m: Mode) => {
    setOpen(false);
    setMode(m).catch((e) => {
      // A conversation is running: the main window asks first (P5 §13).
      if (e instanceof ApiError && e.code === "session_active") void api.showMain("/talk/session?confirm=hibernate");
      else toastError(e);
    });
  };
  return (
    <div className={styles.badgeWrap} ref={ref}>
      <button
        className={`${styles.badge} ${mode === "hibernate" ? styles.badgeHibernate : ""}`}
        onClick={() => setOpen(!open)}
        aria-haspopup="menu"
        aria-label={`Mode: ${mode}. Change mode`}
      >
        {mode === "hibernate" ? "💤 Hibernate" : "● Standard"}
      </button>
      {open && (
        <div className={styles.menu} role="menu">
          <button role="menuitem" onClick={() => choose("standard")}>
            <strong>{mode === "standard" ? "✓ " : ""}Standard</strong>
            <span className={styles.menuHint}>All features on</span>
          </button>
          <button role="menuitem" onClick={() => choose("hibernate")}>
            <strong>{mode === "hibernate" ? "✓ " : ""}Hibernate</strong>
            <span className={styles.menuHint}>News only, uses less power</span>
          </button>
        </div>
      )}
    </div>
  );
}

function MoreMenu({ article, onDone }: { article: ArticleListItem; onDone: () => void }) {
  const [open, setOpen] = useState(false);
  const ref = useDismiss(open, () => setOpen(false));
  const act = (f: () => Promise<void>) => {
    setOpen(false);
    f().then(onDone).catch(toastError);
  };
  return (
    <div className={styles.badgeWrap} ref={ref}>
      <button className={styles.iconBtn} onClick={() => setOpen(!open)} aria-label="More actions">
        ⋯
      </button>
      {open && (
        <div className={`${styles.menu} ${styles.menuUp}`} role="menu">
          <button role="menuitem" onClick={() => act(() => api.setSaved(article.id, !article.saved))}>
            {article.saved ? "Unsave" : "Save for later"}
          </button>
          <button role="menuitem" onClick={() => act(() => api.openArticle(article.id))}>
            Open in browser
          </button>
          <button role="menuitem" onClick={() => act(() => api.recordInteraction(article.id, "not_interested"))}>
            Not interested
          </button>
        </div>
      )}
    </div>
  );
}

function ArticleCard({
  label,
  article,
  why,
  lesson = false,
}: {
  label: string;
  article: ArticleListItem;
  why?: string;
  lesson?: boolean;
}) {
  const refreshPick = useApp((s) => s.refreshPick);
  const meta = [article.primaryTopic, article.sourceName, shortAge(article.publishedAt ?? article.discoveredAt)]
    .filter(Boolean)
    .join(" · ");
  const level = article.difficulty
    ? `${article.difficulty[0].toUpperCase()}${article.difficulty.slice(1)} English${article.readingMinutes ? ` · ${article.readingMinutes} min read` : ""}`
    : null;
  return (
    <div className={styles.pick}>
      <div className={`${styles.label} ${lesson ? styles.labelLesson : ""}`}>{lesson ? `📘 ${label}` : label}</div>
      <h2 className={styles.title} title={article.title}>
        {article.title}
      </h2>
      <div className={styles.meta}>{meta}</div>
      {level && <div className={styles.meta}>{level}</div>}
      {why && <p className={styles.why}>{why}</p>}
      <div className={styles.actions}>
        <button className="primary" onClick={() => api.showMain(`/reader/${article.id}`).catch(toastError)}>
          Read
        </button>
        {article.saved && <span className={styles.saved}>★ Saved</span>}
        <span className="spacer" />
        <MoreMenu article={article} onDone={() => void refreshPick()} />
      </div>
    </div>
  );
}

type View = "story" | "lesson";

/** Story | Lesson switch in the header (only when there is a lesson today). */
function ViewSwitch({ view, onChange }: { view: View; onChange: (v: View) => void }) {
  return (
    <div className={styles.segmented} role="tablist" aria-label="Today's story or lesson">
      {(["story", "lesson"] as View[]).map((v) => (
        <button
          key={v}
          role="tab"
          aria-selected={view === v}
          className={view === v ? styles.segOn : undefined}
          onClick={() => onChange(v)}
        >
          {v === "story" ? "Story" : "Lesson"}
        </button>
      ))}
    </div>
  );
}

function Body({ view }: { view: View }) {
  const { settings, pick, lesson, preview, loaded } = useApp();
  if (!loaded || !settings) return <div className={styles.center}>Loading…</div>;
  if (!settings.onboardingDone) {
    return (
      <div className={styles.center}>
        <p>Welcome! Choose your topics to get your first tech story.</p>
        <button className="primary" onClick={() => api.showMain("/onboarding")}>
          Start
        </button>
      </div>
    );
  }
  if (view === "lesson" && lesson) {
    return <ArticleCard label="Today's lesson" article={lesson.article} why={lesson.why} lesson />;
  }
  if (pick) return <ArticleCard label="Today's story" article={pick.article} why={pick.why} />;
  const beforePickTime = new Date().toTimeString().slice(0, 5) < settings.pickTime;
  if (preview) {
    const label = beforePickTime ? `Preview · today's pick at ${settings.pickTime}` : "Best story right now";
    return <ArticleCard label={label} article={preview} />;
  }
  if (beforePickTime) {
    return (
      <div className={styles.center}>
        <p>Today's pick comes at {settings.pickTime}.</p>
        <p className="faint">Fetching news in the background…</p>
      </div>
    );
  }
  return (
    <div className={styles.center}>
      <p>Nothing matched your topics today.</p>
      <button onClick={() => api.showMain("/explore")}>Open Explore</button>
    </div>
  );
}

function Footer() {
  const newsVersion = useApp((s) => s.newsVersion);
  const due = useWords((s) => s.due);
  const [status, setStatus] = useState<string>("");
  useEffect(() => {
    api
      .newsStatus()
      .then((s) => {
        const updated = s.lastFetchedAt ? `updated ${timeAgo(s.lastFetchedAt)}` : "not updated yet";
        const errors = s.feedsWithErrors ? ` · ${s.feedsWithErrors} with errors` : "";
        setStatus(`${s.feedCount} sources · ${updated}${errors}`);
      })
      .catch(() => setStatus(""));
  }, [newsVersion]);
  const words = due && due.total > 0 ? `📚 ${due.due} due · ${due.new} new` : null;
  // P5: a voice session is running.
  const [talking, setTalking] = useState(false);
  useEffect(() => {
    api
      .getActiveSession()
      .then((a) => setTalking(!!a))
      .catch(() => {});
    const un = onEvent<{ active: boolean }>(EVENTS.voiceActive, (p) => setTalking(p.active));
    return () => void un.then((f) => f());
  }, []);
  return (
    <footer className={styles.footer}>
      <span className={styles.footerText}>{status}</span>
      {talking ? (
        <button className={styles.practice} onClick={() => api.showMain("/talk/session").catch(toastError)}>
          🎙 Talking… · Open
        </button>
      ) : words && (
        <button
          className={styles.practice}
          onClick={() => api.showMain("/practice").catch(toastError)}
          title="Practise your Word Book"
        >
          {words} · Practice
        </button>
      )}
    </footer>
  );
}

export default function Widget() {
  const { settings, mode, pick, lesson } = useApp();
  const [view, setView] = useState<View>("story");
  const style = settings?.widget.style ?? "card";
  const collapse = () => api.setWidgetStyle({ style: "pill" }).catch(toastError);
  const expand = () => api.setWidgetStyle({ style: "card" }).catch(toastError);

  if (style === "pill") {
    return (
      <div className={styles.pill} data-tauri-drag-region>
        <span data-tauri-drag-region className={mode === "hibernate" ? styles.dotHibernate : styles.dot} />
        <span data-tauri-drag-region className={styles.pillText}>
          {mode === "hibernate"
            ? "Hibernate"
            : pick && lesson
              ? "Story + lesson ready"
              : pick
                ? "Today's story ready"
                : "Tech English"}
        </span>
        <button className={styles.iconBtn} onClick={expand} aria-label="Expand widget">
          ⌄
        </button>
      </div>
    );
  }

  return (
    <div className={styles.card}>
      <header className={styles.header} data-tauri-drag-region>
        {lesson ? (
          <ViewSwitch view={view} onChange={setView} />
        ) : (
          <span data-tauri-drag-region className={styles.appName}>
            Tech English
          </span>
        )}
        <span className="spacer" data-tauri-drag-region />
        <AiDot />
        <ModeBadge mode={mode} />
        <button className={styles.iconBtn} onClick={collapse} aria-label="Collapse to a small bar" title="Collapse">
          ⌃
        </button>
        <button className={styles.iconBtn} onClick={() => api.showMain("/today")} aria-label="Open app" title="Open app">
          ⤢
        </button>
      </header>
      <main className={styles.body}>
        <Body view={lesson ? view : "story"} />
      </main>
      <Footer />
    </div>
  );
}
