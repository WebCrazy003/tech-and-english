import { useEffect, useState } from "react";
import { api, type AboutInfo, type CrashNotice } from "../../lib/api";
import { toast, toastError } from "../../stores/toast";
import styles from "../pages.module.css";

export const SHORTCUTS: [string, string][] = [
  ["⌘1 – ⌘6", "Today, Explore, Word Book, Practice, Talk, Settings"],
  ["Space", "Practice: show the answer · Talk: hold to speak"],
  ["1 / 2 / 3", "Practice: Forgot / Not sure / Remember"],
  ["Esc", "Close a word popup"],
  ["Tab / Shift-Tab", "Move between buttons and fields"],
];

const LISTS: { kind: "notices" | "rust" | "js"; label: string }[] = [
  { kind: "notices", label: "Engines, word list and models" },
  { kind: "rust", label: "Rust libraries" },
  { kind: "js", label: "JavaScript libraries" },
];

/** Copies the diagnostics text (version, system, engine states, last log lines — no user content). */
export async function copyDiagnostics(): Promise<void> {
  try {
    await api.copyText(await api.getDiagnostics());
    toast("Diagnostics copied. Nothing was sent anywhere.");
  } catch (e) {
    toastError(e);
  }
}

/** Settings › About: version, crash notice, diagnostics, shortcuts and licenses (P6 §4, §6). */
export default function About() {
  const [info, setInfo] = useState<AboutInfo | null>(null);
  const [crash, setCrash] = useState<CrashNotice | null>(null);
  const [list, setList] = useState<{ kind: string; text: string } | null>(null);

  useEffect(() => {
    api.aboutInfo().then(setInfo).catch(toastError);
    api.takeCrashNotice().then(setCrash).catch(() => {});
  }, []);

  const show = (kind: "notices" | "rust" | "js") => {
    if (list?.kind === kind) return setList(null);
    api
      .thirdPartyLicenses(kind)
      .then((text) => setList({ kind, text }))
      .catch(toastError);
  };

  return (
    <div>
      {crash && (
        <div className={styles.crash} role="alertdialog" aria-label="Tech English closed unexpectedly">
          <strong>Tech English closed unexpectedly.</strong>
          <p className="muted">
            A short report was saved on this Mac ({crash.file}). It contains no stories, words or conversations, and it
            is not sent anywhere.
          </p>
          <div className="row">
            <button
              className="primary"
              onClick={() => void api.copyText(crash.text).then(() => toast("Crash report copied")).catch(toastError)}
            >
              Copy diagnostics
            </button>
            <button onClick={() => setCrash(null)}>OK</button>
          </div>
        </div>
      )}

      <div className={styles.groupTitle}>About</div>
      <div className={styles.group}>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>Tech English {info?.version ?? ""}</div>
            <div className={styles.fieldHelp}>
              {info ? `Build ${info.commit} · ${info.buildDate} · ${info.system}` : "Loading…"}
            </div>
          </div>
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>Diagnostics</div>
            <div className={styles.fieldHelp}>
              Version, macOS, mode, engine status and the last log lines. No stories, words or conversations. Nothing is
              sent over the network.
            </div>
          </div>
          <button onClick={() => void copyDiagnostics()}>Copy diagnostics</button>
        </div>
        {import.meta.env.DEV && (
          <div className={styles.field}>
            <div className={styles.fieldText}>
              <div className={styles.fieldLabel}>Debug: crash now</div>
              <div className={styles.fieldHelp}>Development builds only. Tests the crash notice on the next start.</div>
            </div>
            <button className="danger" onClick={() => void api.debugPanic().catch(toastError)}>
              Crash
            </button>
          </div>
        )}
      </div>

      <div className={styles.groupTitle}>Keyboard shortcuts</div>
      <div className={styles.group}>
        {SHORTCUTS.map(([keys, what]) => (
          <div key={keys} className={styles.field}>
            <div className={styles.fieldText}>
              <div className={styles.fieldLabel}>
                <kbd>{keys}</kbd>
              </div>
            </div>
            <span className="muted">{what}</span>
          </div>
        ))}
      </div>

      <div className={styles.groupTitle}>Licenses</div>
      <div className={styles.group}>
        {(info?.models ?? []).map((m) => (
          <div key={m.name} className={styles.field}>
            <div className={styles.fieldText}>
              <div className={styles.fieldLabel}>{m.name}</div>
              <div className={styles.fieldHelp}>Downloaded model · license {m.license}</div>
            </div>
            <button className="ghost" onClick={() => void api.openExternal(m.licenseUrl).catch(toastError)}>
              License ↗
            </button>
          </div>
        ))}
        {LISTS.map((l) => (
          <div key={l.kind} className={styles.field}>
            <div className={styles.fieldText}>
              <div className={styles.fieldLabel}>{l.label}</div>
            </div>
            <button onClick={() => show(l.kind)} aria-expanded={list?.kind === l.kind}>
              {list?.kind === l.kind ? "Hide" : "Show"}
            </button>
          </div>
        ))}
        {list && (
          <pre className={styles.licenseText} tabIndex={0} aria-label="License text">
            {list.text}
          </pre>
        )}
      </div>

      <div className={styles.groupTitle}>Remove the app</div>
      <div className={styles.group}>
        <div className={styles.field}>
          <div className={styles.fieldHelp}>
            Quit Tech English, move it to the Trash, then delete the folders{" "}
            <code>~/Library/Application Support/com.techenglish.app</code> (your data and the AI models) and{" "}
            <code>~/Library/Logs/com.techenglish.app</code>.
          </div>
        </div>
      </div>
    </div>
  );
}
