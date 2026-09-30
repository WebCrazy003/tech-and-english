import { useRouteError } from "react-router";
import { copyDiagnostics } from "../pages/settings/About";
import styles from "../pages/pages.module.css";

/** Shown in place of a page that threw an error (P6 §6). The rest of the app keeps working. */
export default function ErrorPage() {
  const error = useRouteError();
  const message = error instanceof Error ? error.message : String(error);
  return (
    <div className={styles.page} role="alert">
      <div className={styles.pageHeader}>
        <h1>Something went wrong on this page</h1>
      </div>
      <div className={styles.card}>
        <p className="muted">Your data is safe. Reload the page, or open another page from the sidebar.</p>
        <p className="faint" style={{ fontSize: 12 }}>
          {message}
        </p>
        <div className={styles.actions}>
          <button className="primary" onClick={() => window.location.reload()}>
            Reload page
          </button>
          <button onClick={() => void copyDiagnostics()}>Copy diagnostics</button>
        </div>
      </div>
    </div>
  );
}
