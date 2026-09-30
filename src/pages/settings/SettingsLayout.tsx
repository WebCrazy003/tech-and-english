import { NavLink, Outlet } from "react-router";
import styles from "../pages.module.css";

const LINKS = [
  { to: "/settings/general", label: "General" },
  { to: "/settings/topics", label: "Topics" },
  { to: "/settings/feeds", label: "News sources" },
  { to: "/settings/notifications", label: "Notifications" },
  { to: "/settings/ai", label: "AI" },
  { to: "/settings/voice", label: "Voice" },
  { to: "/settings/learning", label: "Practice" },
];

export default function SettingsLayout() {
  return (
    <div>
      <div className={styles.pageHeader}>
        <h1>Settings</h1>
      </div>
      <div className={styles.settingsShell}>
        <nav className={styles.settingsNav} aria-label="Settings sections">
          {LINKS.map((l) => (
            <NavLink key={l.to} to={l.to} className={({ isActive }) => (isActive ? styles.activeLink : "")}>
              {l.label}
            </NavLink>
          ))}
        </nav>
        <div className={styles.settingsBody}>
          <Outlet />
        </div>
      </div>
    </div>
  );
}
