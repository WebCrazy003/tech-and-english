import { useEffect, useState } from "react";
import { NavLink, Outlet, useLocation, useNavigate } from "react-router";
import { ApiError, api, type Mode } from "../lib/api";
import { endTalk } from "../stores/talk";
import { timeAgo } from "../lib/format";
import { useApp } from "../stores/app";
import { toast, toastError } from "../stores/toast";
import Toasts from "../components/Toasts";
import styles from "./Layout.module.css";

const NAV = [
  { to: "/today", label: "Today", icon: "☀︎", key: "1" },
  { to: "/explore", label: "Explore", icon: "☰", key: "2" },
  { to: "/wordbook", label: "Word Book", icon: "📖", key: "3" },
  { to: "/practice", label: "Practice", icon: "✎", key: "4" },
  { to: "/talk", label: "Talk", icon: "🎙", key: "5" },
  { to: "/settings", label: "Settings", icon: "⚙︎", key: "6" },
];

function ModeSwitch() {
  const { mode, setMode } = useApp();
  const navigate = useNavigate();
  const pick = async (m: Mode) => {
    try {
      await setMode(m);
    } catch (e) {
      // A conversation is running: ask first, then end it (its review waits in History).
      if (e instanceof ApiError && e.code === "session_active") {
        if (!window.confirm("End the conversation and switch to Hibernate? You can finish the review later.")) return;
        await endTalk("hibernate");
        await setMode(m).catch(toastError);
        void navigate("/talk/history");
      } else toastError(e);
    }
  };
  return (
    <div className={styles.modeBox}>
      <div className={styles.modeLabel}>Mode</div>
      <div className={styles.segmented} role="radiogroup" aria-label="Mode">
        <button role="radio" aria-checked={mode === "standard"} className={mode === "standard" ? styles.on : ""} onClick={() => void pick("standard")}>
          Standard
        </button>
        <button
          role="radio"
          aria-checked={mode === "hibernate"}
          className={mode === "hibernate" ? styles.onHibernate : ""}
          onClick={() => void pick("hibernate")}
        >
          Hibernate
        </button>
      </div>
      <div className={styles.modeHint}>
        {mode === "standard" ? "All features on." : "Only news fetching runs. Uses less power."}
      </div>
    </div>
  );
}

function RefreshBox() {
  const newsVersion = useApp((s) => s.newsVersion);
  const [busy, setBusy] = useState(false);
  const [last, setLast] = useState<string | null>(null);
  useEffect(() => {
    api.newsStatus().then((s) => setLast(s.lastFetchedAt)).catch(() => {});
  }, [newsVersion]);
  const refresh = async () => {
    setBusy(true);
    try {
      const r = await api.refreshNow();
      toast(r.newCount ? `${r.newCount} new stories` : "No new stories");
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className={styles.refresh}>
      <button onClick={refresh} disabled={busy}>
        {busy ? "Refreshing…" : "↻ Refresh news"}
      </button>
      <div className="faint">{last ? `Updated ${timeAgo(last)}` : "Not updated yet"}</div>
    </div>
  );
}

export default function Layout() {
  const navigate = useNavigate();
  // The Reader needs the room: the sidebar becomes an icon rail and the page is not padded.
  const reader = useLocation().pathname.startsWith("/reader/");
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey) return;
      const item = NAV.find((n) => n.key === e.key);
      if (item) {
        e.preventDefault();
        void navigate(item.to);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [navigate]);

  return (
    <div className={styles.shell}>
      <nav className={`${styles.sidebar} ${reader ? styles.rail : ""}`} aria-label="Main">
        <div className={styles.brand}>{reader ? "TE" : "Tech English"}</div>
        {NAV.map((n) => (
          <NavLink
            key={n.to}
            to={n.to}
            title={n.label}
            className={({ isActive }) => `${styles.nav} ${isActive ? styles.active : ""}`}
          >
            <span className={styles.navIcon} aria-hidden>
              {n.icon}
            </span>
            {!reader && n.label}
            {!reader && <span className={styles.kbd}>⌘{n.key}</span>}
          </NavLink>
        ))}
        <div className="spacer" />
        {!reader && <RefreshBox />}
        {!reader && <ModeSwitch />}
      </nav>
      <main className={`${styles.content} ${reader ? styles.contentFlush : ""}`}>
        <Outlet />
      </main>
      <Toasts />
    </div>
  );
}
