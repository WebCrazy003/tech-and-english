import { useToasts } from "../stores/toast";
import styles from "./components.module.css";

export default function Toasts({ compact = false }: { compact?: boolean }) {
  const { toasts, dismiss } = useToasts();
  if (!toasts.length) return null;
  return (
    <div className={compact ? styles.toastsCompact : styles.toasts} role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`${styles.toast} ${t.kind === "error" ? styles.toastError : ""}`}>
          <span>{t.text}</span>
          <button className="ghost" onClick={() => dismiss(t.id)} aria-label="Dismiss">
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
