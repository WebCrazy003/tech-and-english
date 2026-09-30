import { useEffect, useState } from "react";
import { api, type LatencyReport, type Percentiles } from "../../lib/api";
import { useApp } from "../../stores/app";
import { gb, useAi } from "../../stores/ai";
import { toastError } from "../../stores/toast";
import { AiDot, DownloadButton } from "../../features/ai/AiSetup";
import styles from "../pages.module.css";
import { useSettingsPatch } from "./useSettingsPatch";

const STATE_LABEL: Record<string, string> = {
  unloaded: "Not loaded (starts when you use it)",
  loading: "Starting…",
  ready: "Ready",
  busy: "Working…",
  error: "Error",
};

export default function Ai() {
  const settings = useApp((s) => s.settings);
  const { overview, status, refresh } = useAi();
  const patch = useSettingsPatch();

  useEffect(() => {
    void refresh().catch(toastError);
  }, [refresh]);

  if (!settings || !overview) return null;
  const ai = settings.ai;

  return (
    <div>
      <div className={styles.groupTitle}>Status</div>
      <div className={styles.group}>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <div className={`${styles.fieldLabel} row`}>
              <AiDot /> {STATE_LABEL[status.state] ?? status.state}
              {status.modelId && <span className="faint">· {status.modelId}</span>}
            </div>
            {status.message && <div className="error-text">{status.message}</div>}
            <div className={styles.fieldHelp}>Free memory now: {overview.availableMemoryGb.toFixed(1)} GB</div>
          </div>
          {(status.state === "ready" || status.state === "busy") && (
            <button onClick={() => api.unloadAi().catch(toastError)}>Unload now</button>
          )}
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>AI engine (llama-server)</div>
            <div className={styles.fieldHelp}>{overview.enginePath ?? "Not found. The app looks in its data folder, in Homebrew, and in your PATH."}</div>
          </div>
        </div>
      </div>

      <div className={styles.groupTitle}>Models</div>
      <div className={styles.group}>
        {overview.models.filter((m) => m.role === "chat").map((m) => (
          <div key={m.id} className={styles.field}>
            <div className={styles.fieldText}>
              <div className={styles.fieldLabel}>
                {m.displayName} {m.default && <span className="faint">· recommended</span>}
              </div>
              <div className={styles.fieldHelp}>
                {gb(m.sizeBytes)} · needs about {m.recommendedRamGb} GB of memory · license{" "}
                <a href="#" onClick={(e) => { e.preventDefault(); void api.openExternal(m.licenseUrl); }}>
                  {m.license}
                </a>
              </div>
            </div>
            <DownloadButton m={m} />
            {m.downloaded &&
              (m.active ? (
                <span className="faint">In use</span>
              ) : (
                <button onClick={() => api.setActiveModel(m.id).then(refresh).catch(toastError)}>Use this</button>
              ))}
            {(m.downloaded || m.partialBytes > 0) && !m.downloading && (
              <button
                className="ghost"
                title="Delete the model file"
                onClick={() => {
                  if (window.confirm(`Delete ${m.displayName} (${gb(m.sizeBytes)})?`)) {
                    api.deleteModel(m.id).then(refresh).catch(toastError);
                  }
                }}
              >
                ✕
              </button>
            )}
          </div>
        ))}
      </div>

      <div className={styles.groupTitle}>Options</div>
      <div className={styles.group}>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="lvl">English level of AI answers</label>
          </div>
          <select id="lvl" value={ai.englishLevel} onChange={(e) => void patch({ ai: { englishLevel: Number(e.target.value) as 1 | 2 | 3 } })}>
            <option value={1}>Very easy</option>
            <option value={2}>B1 (recommended)</option>
            <option value={3}>Natural</option>
          </select>
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="idle">Unload the AI after it is unused for</label>
            <div className={styles.fieldHelp}>Unloading frees about 3 GB of memory. Starting again takes a few seconds.</div>
          </div>
          <select id="idle" value={ai.idleTimeoutMin} onChange={(e) => void patch({ ai: { idleTimeoutMin: Number(e.target.value) } })}>
            {[2, 5, 10, 20, 30, 60].map((m) => (
              <option key={m} value={m}>
                {m} minutes
              </option>
            ))}
          </select>
        </div>
        <label className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>AI-written reason for the daily pick</div>
            <div className={styles.fieldHelp}>Only when the AI is already loaded, so it never uses extra memory.</div>
          </div>
          <input type="checkbox" checked={ai.llmWhy} onChange={(e) => void patch({ ai: { llmWhy: e.target.checked } })} />
        </label>
      </div>

      <Diagnostics />
    </div>
  );
}

const ms = (x: number | null) => (x == null ? "–" : `${(x / 1000).toFixed(1)} s`);

/** Voice tutor speed (P5 §10): end of speech → first tutor audio, and each stage. */
function Diagnostics() {
  const [r, setR] = useState<LatencyReport | null>(null);
  useEffect(() => {
    api.voiceLatency().then(setR).catch(() => {});
  }, []);
  if (!r) return null;
  const rows: [string, Percentiles][] = [
    ["End of speech → first tutor audio", r.total],
    ["Speech to text", r.stt],
    ["AI: first word", r.firstToken],
    ["AI: first sentence", r.firstSentence],
    ["AI: whole answer", r.llmDone],
  ];
  return (
    <>
      <div className={styles.groupTitle}>Diagnostics · voice tutor speed</div>
      <div className={styles.group}>
        {r.total.count === 0 && r.stt.count === 0 ? (
          <div className={styles.field}>
            <div className={styles.fieldHelp}>No spoken turns yet in this app session. Talk to the tutor, then look here.</div>
          </div>
        ) : (
          rows.map(([label, p]) => (
            <div key={label} className={styles.field}>
              <div className={styles.fieldText}>
                <div className={styles.fieldLabel}>{label}</div>
                <div className={styles.fieldHelp}>{p.count} turns (last 50)</div>
              </div>
              <span className="muted">
                typical {ms(p.p50)} · slow {ms(p.p90)}
              </span>
            </div>
          ))
        )}
        <div className={styles.field}>
          <div className={styles.fieldHelp}>Goal: typical ≤ 3 s, slow ≤ 5 s from the end of your speech to the tutor’s voice.</div>
          <button className="ghost" onClick={() => api.voiceLatency().then(setR).catch(toastError)}>
            Refresh
          </button>
        </div>
      </div>
    </>
  );
}
