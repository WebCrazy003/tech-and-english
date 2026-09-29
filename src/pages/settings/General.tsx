import { useEffect, useState } from "react";
import { api, type Mode, type RankingWeights, type WidgetStyle } from "../../lib/api";
import { useApp } from "../../stores/app";
import { toast, toastError } from "../../stores/toast";
import styles from "../pages.module.css";
import { useSettingsPatch } from "./useSettingsPatch";

const WEIGHT_LABELS: [keyof RankingWeights, string][] = [
  ["topicRelevance", "Topic match"],
  ["freshness", "Freshness"],
  ["popularity", "Popularity"],
  ["sourcePreference", "Source preference"],
  ["novelty", "Novelty"],
  ["userHistory", "Your history"],
];

function Weights({ value }: { value: RankingWeights }) {
  const patch = useSettingsPatch();
  const [w, setW] = useState(value);
  useEffect(() => setW(value), [value]);
  const sum = Object.values(w).reduce((a, b) => a + b, 0);
  const ok = Math.abs(sum - 1) < 0.001;
  return (
    <div className={styles.group}>
      {WEIGHT_LABELS.map(([k, label]) => (
        <div key={k} className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor={`w-${k}`}>{label}</label>
          </div>
          <input
            id={`w-${k}`}
            type="number"
            min={0}
            max={1}
            step={0.05}
            value={w[k]}
            onChange={(e) => setW({ ...w, [k]: Number(e.target.value) })}
            style={{ width: 80 }}
          />
        </div>
      ))}
      <div className={styles.field}>
        <div className={styles.fieldText}>
          <span className={ok ? "muted" : "error-text"}>Total: {sum.toFixed(2)} (must be 1.00)</span>
        </div>
        <button onClick={() => setW(value)}>Undo</button>
        <button className="primary" disabled={!ok} onClick={() => void patch({ rankingWeights: w })}>
          Save weights
        </button>
      </div>
    </div>
  );
}

export default function General() {
  const { settings, mode, setMode } = useApp();
  const patch = useSettingsPatch();
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [advanced, setAdvanced] = useState(false);

  useEffect(() => {
    api
      .getAutostart()
      .then(setAutostart)
      .catch(() => setAutostart(null));
  }, []);

  if (!settings) return null;
  const widget = settings.widget;

  return (
    <div>
      <div className={styles.groupTitle}>Mode</div>
      <div className={styles.group}>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="mode">Operating mode</label>
            <div className={styles.fieldHelp}>
              Hibernate keeps only news fetching running. AI and voice (later versions) turn off.
            </div>
          </div>
          <select id="mode" value={mode} onChange={(e) => setMode(e.target.value as Mode).catch(toastError)}>
            <option value="standard">Standard</option>
            <option value="hibernate">Hibernate</option>
          </select>
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="intStd">Check for news every (Standard)</label>
          </div>
          <select
            id="intStd"
            value={settings.fetchIntervalStandardMin}
            onChange={(e) => void patch({ fetchIntervalStandardMin: Number(e.target.value) })}
          >
            {[15, 20, 25, 30].map((m) => (
              <option key={m} value={m}>
                {m} minutes
              </option>
            ))}
          </select>
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="intHib">Check for news every (Hibernate)</label>
          </div>
          <select
            id="intHib"
            value={settings.fetchIntervalHibernateMin}
            onChange={(e) => void patch({ fetchIntervalHibernateMin: Number(e.target.value) })}
          >
            {[30, 45, 60].map((m) => (
              <option key={m} value={m}>
                {m} minutes
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className={styles.groupTitle}>Daily pick</div>
      <div className={styles.group}>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="pick">Choose today's story at</label>
          </div>
          <input
            id="pick"
            type="time"
            value={settings.pickTime}
            onChange={(e) => e.target.value && void patch({ pickTime: e.target.value })}
          />
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="age">Ignore stories older than</label>
            <div className={styles.fieldHelp}>Some feeds include very old articles. They are skipped.</div>
          </div>
          <select
            id="age"
            value={settings.ingestMaxAgeDays}
            onChange={(e) => void patch({ ingestMaxAgeDays: Number(e.target.value) })}
          >
            {[1, 3, 7, 14, 30].map((d) => (
              <option key={d} value={d}>
                {d} {d === 1 ? "day" : "days"}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className={styles.groupTitle}>Widget and app</div>
      <div className={styles.group}>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="wstyle">Widget</label>
          </div>
          <select
            id="wstyle"
            value={widget.style}
            onChange={(e) => api.setWidgetStyle({ style: e.target.value as WidgetStyle }).catch(toastError)}
          >
            <option value="card">Card</option>
            <option value="pill">Small bar</option>
            <option value="hidden">Hidden (menu bar only)</option>
          </select>
        </div>
        <label className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>Keep the widget above other windows</div>
          </div>
          <input
            type="checkbox"
            checked={widget.alwaysOnTop}
            onChange={(e) => api.setWidgetStyle({ alwaysOnTop: e.target.checked }).catch(toastError)}
          />
        </label>
        <label className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>Open when I log in</div>
          </div>
          <input
            type="checkbox"
            checked={!!autostart}
            disabled={autostart === null}
            onChange={(e) => api.setAutostart(e.target.checked).then(setAutostart).catch(toastError)}
          />
        </label>
        <label className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>Show an icon in the Dock</div>
            <div className={styles.fieldHelp}>Works after you quit and open the app again.</div>
          </div>
          <input
            type="checkbox"
            checked={settings.showDockIcon}
            onChange={(e) => void patch({ showDockIcon: e.target.checked }).then(() => toast("Restart the app to apply."))}
          />
        </label>
      </div>

      <div className={styles.groupTitle}>
        <button className="ghost" onClick={() => setAdvanced(!advanced)} aria-expanded={advanced}>
          {advanced ? "▾" : "▸"} Advanced: ranking weights
        </button>
      </div>
      {advanced && <Weights value={settings.rankingWeights} />}

      <div className={styles.groupTitle}>Quit</div>
      <div className={styles.group}>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldHelp}>Closing a window keeps the app running in the menu bar.</div>
          </div>
          <button className="danger" onClick={() => api.quitApp().catch(toastError)}>
            Quit Tech English
          </button>
        </div>
      </div>
    </div>
  );
}
