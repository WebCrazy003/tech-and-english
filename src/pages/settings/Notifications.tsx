import { useApp } from "../../stores/app";
import styles from "../pages.module.css";
import { useSettingsPatch } from "./useSettingsPatch";

export default function Notifications() {
  const settings = useApp((s) => s.settings);
  const patch = useSettingsPatch();
  if (!settings) return null;
  const quiet = settings.quietHours;

  return (
    <div>
      <div className={styles.groupTitle}>Daily pick</div>
      <div className={styles.group}>
        <label className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>Notify me when today's story is ready</div>
          </div>
          <input type="checkbox" checked={settings.notifyDailyPick} onChange={(e) => void patch({ notifyDailyPick: e.target.checked })} />
        </label>
      </div>

      <div className={styles.groupTitle}>Very interesting stories</div>
      <div className={styles.group}>
        <label className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>Notify me about very interesting stories</div>
            <div className={styles.fieldHelp}>Only for topics with 🔔 turned on (Settings › Topics).</div>
          </div>
          <input
            type="checkbox"
            checked={settings.notifyHighInterest}
            onChange={(e) => void patch({ notifyHighInterest: e.target.checked })}
          />
        </label>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="thr">Minimum score</label>
            <div className={styles.fieldHelp}>Higher = fewer notifications. A topic can set its own value.</div>
          </div>
          <input
            id="thr"
            type="range"
            min={50}
            max={100}
            step={1}
            value={settings.notifyThreshold}
            onChange={(e) => void patch({ notifyThreshold: Number(e.target.value) })}
          />
          <span style={{ width: 32, textAlign: "right" }}>{settings.notifyThreshold}</span>
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="max">At most per day</label>
          </div>
          <select id="max" value={settings.notifyMaxPerDay} onChange={(e) => void patch({ notifyMaxPerDay: Number(e.target.value) })}>
            {[0, 1, 2, 3, 4, 5].map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="gap">Wait between notifications</label>
          </div>
          <select id="gap" value={settings.notifyMinGapMin} onChange={(e) => void patch({ notifyMinGapMin: Number(e.target.value) })}>
            {[30, 60, 90, 120, 180].map((n) => (
              <option key={n} value={n}>
                {n} minutes
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className={styles.groupTitle}>Quiet hours</div>
      <div className={styles.group}>
        <label className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>No "interesting story" notifications at night</div>
          </div>
          <input
            type="checkbox"
            checked={!!quiet}
            onChange={(e) => void patch({ quietHours: e.target.checked ? ["22:00", "08:00"] : null })}
          />
        </label>
        {quiet && (
          <div className={styles.field}>
            <div className={styles.fieldText}>From / to</div>
            <input
              type="time"
              value={quiet[0]}
              aria-label="Quiet hours start"
              onChange={(e) => e.target.value && void patch({ quietHours: [e.target.value, quiet[1]] })}
            />
            <input
              type="time"
              value={quiet[1]}
              aria-label="Quiet hours end"
              onChange={(e) => e.target.value && void patch({ quietHours: [quiet[0], e.target.value] })}
            />
          </div>
        )}
      </div>
    </div>
  );
}
