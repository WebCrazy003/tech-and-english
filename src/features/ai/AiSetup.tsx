import { useNavigate } from "react-router";
import { api, type ModelInfo } from "../../lib/api";
import { useApp } from "../../stores/app";
import { gb, useAi } from "../../stores/ai";
import { toastError } from "../../stores/toast";
import styles from "./ai.module.css";

function DownloadButton({ m }: { m: ModelInfo }) {
  const progress = useAi((s) => s.progress[m.id]);
  if (m.downloaded) return <span className={styles.ok}>✓ Downloaded</span>;
  if (m.downloading || progress) {
    const p = progress ?? { bytes: m.partialBytes, total: m.sizeBytes };
    const pct = Math.floor((p.bytes / Math.max(1, p.total)) * 100);
    return (
      <div className={styles.progressRow}>
        <div className={styles.bar} role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
          <span style={{ width: `${pct}%` }} />
        </div>
        <span className="faint">
          {pct}% of {gb(m.sizeBytes)}
        </span>
        <button className="ghost" onClick={() => api.cancelDownload(m.id).catch(toastError)}>
          Pause
        </button>
      </div>
    );
  }
  return (
    <button
      className="primary"
      onClick={() =>
        api
          .downloadModel(m.id)
          .then(() => useAi.setState((s) => ({ progress: { ...s.progress, [m.id]: { bytes: m.partialBytes, total: m.sizeBytes } } })))
          .catch(toastError)
      }
    >
      {m.partialBytes > 0 ? "Resume download" : "Download"} ({gb(m.sizeBytes - m.partialBytes)})
    </button>
  );
}

/** First-use card: recommends the default model, shows size + license, downloads it. */
export function AiSetupCard() {
  const overview = useAi((s) => s.overview);
  if (!overview) return null;
  const rec = overview.models.find((m) => m.default) ?? overview.models[0];
  if (!rec) return null;
  return (
    <div className={styles.card}>
      <strong>Set up the AI</strong>
      <p className="muted">
        The AI runs on your Mac. It needs one model file: <b>{rec.displayName}</b> ({gb(rec.sizeBytes)}, license{" "}
        <a href="#" onClick={(e) => { e.preventDefault(); void api.openExternal(rec.licenseUrl); }}>
          {rec.license}
        </a>
        ).
      </p>
      {!overview.enginePath && (
        <p className="error-text">The AI engine (llama-server) was not found. See Settings › AI.</p>
      )}
      <DownloadButton m={rec} />
    </div>
  );
}

export { DownloadButton };

/** What to show when an AI request fails, by error code. `retry(force)` repeats the request. */
export function AiProblem({
  code,
  message,
  retry,
}: {
  code: string;
  message: string;
  retry: (force?: boolean) => void;
}) {
  const navigate = useNavigate();
  const setMode = useApp((s) => s.setMode);
  const overview = useAi((s) => s.overview);
  const hasModel = overview?.models.some((m) => m.downloaded);
  switch (code) {
    case "no_model":
      return (
        <div className={styles.problem}>
          <AiSetupCard />
          {hasModel && (
            <button className="primary" onClick={() => retry()}>
              Try again
            </button>
          )}
        </div>
      );
    case "hibernating":
      return (
        <div className={styles.problem}>
          <p>AI is off in Hibernate mode.</p>
          <button className="primary" onClick={() => setMode("standard").then(() => retry()).catch(toastError)}>
            Switch to Standard
          </button>
        </div>
      );
    case "low_memory":
      return (
        <div className={styles.problem}>
          <p>{message}</p>
          <p className="faint">Closing other apps frees memory. The AI can still start, but your Mac may get slow.</p>
          <button className="primary" onClick={() => retry(true)}>
            Start anyway
          </button>
        </div>
      );
    case "no_engine":
      return (
        <div className={styles.problem}>
          <p>{message}</p>
          <button onClick={() => navigate("/settings/ai")}>Open Settings › AI</button>
        </div>
      );
    default:
      return (
        <div className={styles.problem}>
          <p className="error-text">{message}</p>
          <button onClick={() => retry()}>Try again</button>
        </div>
      );
  }
}

/** Grey/amber/green/blue/red dot for the AI state. */
export function AiDot() {
  const status = useAi((s) => s.status);
  const mode = useApp((s) => s.mode);
  if (mode === "hibernate") return null;
  const label =
    status.state === "unloaded"
      ? "AI not loaded"
      : status.state === "loading"
        ? "AI is starting…"
        : status.state === "error"
          ? `AI error: ${status.message ?? ""}`
          : `AI ${status.state}${status.modelId ? ` (${status.modelId})` : ""}`;
  return <span className={`${styles.dot} ${styles[status.state]}`} title={label} aria-label={label} role="img" />;
}
