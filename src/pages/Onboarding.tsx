import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router";
import { api, ensureNotificationPermission, type FeedSeed, type TopicSeed } from "../lib/api";
import { useApp } from "../stores/app";
import { toastError } from "../stores/toast";
import Toasts from "../components/Toasts";
import styles from "./pages.module.css";

const STEPS = 4;

export default function Onboarding() {
  const navigate = useNavigate();
  const { newsVersion } = useApp();
  const [step, setStep] = useState(0);
  const [topics, setTopics] = useState<TopicSeed[]>([]);
  const [feeds, setFeeds] = useState<FeedSeed[]>([]);
  const [chosenTopics, setChosenTopics] = useState<Set<string>>(new Set());
  const [chosenFeeds, setChosenFeeds] = useState<Set<string>>(new Set());
  const [pickTime, setPickTime] = useState("08:00");
  const [notifyDaily, setNotifyDaily] = useState(true);
  const [notifyHigh, setNotifyHigh] = useState(true);
  const [launchAtLogin, setLaunchAtLogin] = useState(true);
  const [saving, setSaving] = useState(false);
  const [done, setDone] = useState(false);
  const [startVersion, setStartVersion] = useState(0);

  useEffect(() => {
    api
      .getOnboardingDefaults()
      .then((d) => {
        setTopics(d.topics);
        setFeeds(d.feeds);
        setChosenFeeds(new Set(d.feeds.map((f) => f.url)));
      })
      .catch(toastError);
  }, []);

  const groups = useMemo(() => {
    const m = new Map<string, FeedSeed[]>();
    for (const f of feeds) m.set(f.group || "Other", [...(m.get(f.group || "Other") ?? []), f]);
    return [...m.entries()];
  }, [feeds]);

  const toggle = (set: Set<string>, key: string, update: (s: Set<string>) => void) => {
    const next = new Set(set);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    update(next);
  };

  const finish = async () => {
    setSaving(true);
    try {
      if (notifyDaily || notifyHigh) await ensureNotificationPermission();
      setStartVersion(useApp.getState().newsVersion);
      await api.completeOnboarding({
        topicNames: [...chosenTopics],
        feedUrls: [...chosenFeeds],
        pickTime,
        notifyDailyPick: notifyDaily,
        notifyHighInterest: notifyHigh,
        launchAtLogin,
      });
      setDone(true);
    } catch (e) {
      toastError(e);
    } finally {
      setSaving(false);
    }
  };

  const fetched = done && newsVersion > startVersion;

  if (done) {
    return (
      <div className={styles.onboard}>
        <h1>{fetched ? "Your news is ready" : "Getting your first news…"}</h1>
        <p className={styles.lead}>
          {fetched
            ? "The app keeps running in the menu bar. Your daily pick will appear in the small widget."
            : `Reading ${chosenFeeds.size} sources. This takes about 10–30 seconds.`}
        </p>
        <button className="primary" disabled={!fetched} onClick={() => navigate("/today")}>
          Go to Today
        </button>
        <Toasts />
      </div>
    );
  }

  return (
    <div className={styles.onboard}>
      <div className={styles.steps} aria-label={`Step ${step + 1} of ${STEPS}`}>
        {Array.from({ length: STEPS }, (_, i) => (
          <div key={i} className={`${styles.stepDot} ${i <= step ? styles.stepDotOn : ""}`} />
        ))}
      </div>

      {step === 0 && (
        <>
          <h1>What do you like to read about?</h1>
          <p className={styles.lead}>Choose your topics. You can change them later in Settings.</p>
          <div className={styles.chips}>
            {topics.map((t) => (
              <button
                key={t.name}
                className={`${styles.toggleChip} ${chosenTopics.has(t.name) ? styles.toggleChipOn : ""}`}
                aria-pressed={chosenTopics.has(t.name)}
                onClick={() => toggle(chosenTopics, t.name, setChosenTopics)}
                title={t.keywords.join(", ")}
              >
                {t.name}
              </button>
            ))}
          </div>
        </>
      )}

      {step === 1 && (
        <>
          <h1>Where should news come from?</h1>
          <p className={styles.lead}>These free sources are ready to use. Uncheck any you don't want.</p>
          {groups.map(([group, list]) => {
            const allOn = list.every((f) => chosenFeeds.has(f.url));
            return (
              <div key={group} className={styles.feedGroup}>
                <h3>
                  {group}
                  <button
                    className="ghost"
                    onClick={() => {
                      const next = new Set(chosenFeeds);
                      for (const f of list) {
                        if (allOn) next.delete(f.url);
                        else next.add(f.url);
                      }
                      setChosenFeeds(next);
                    }}
                  >
                    {allOn ? "None" : "All"}
                  </button>
                </h3>
                <div className={styles.feedGrid}>
                  {list.map((f) => (
                    <label key={f.url} className={styles.check}>
                      <input
                        type="checkbox"
                        checked={chosenFeeds.has(f.url)}
                        onChange={() => toggle(chosenFeeds, f.url, setChosenFeeds)}
                      />
                      {f.name}
                    </label>
                  ))}
                </div>
              </div>
            );
          })}
        </>
      )}

      {step === 2 && (
        <>
          <h1>Your daily story</h1>
          <p className={styles.lead}>Each day the app chooses one story for you.</p>
          <div className={styles.group}>
            <div className={styles.field}>
              <div className={styles.fieldText}>
                <label htmlFor="pickTime">Choose the story at</label>
                <div className={styles.fieldHelp}>If your Mac is asleep then, it chooses when it wakes up.</div>
              </div>
              <input id="pickTime" type="time" value={pickTime} onChange={(e) => setPickTime(e.target.value)} />
            </div>
            <label className={styles.field}>
              <div className={styles.fieldText}>
                <div className={styles.fieldLabel}>Notify me when the daily story is ready</div>
              </div>
              <input type="checkbox" checked={notifyDaily} onChange={(e) => setNotifyDaily(e.target.checked)} />
            </label>
            <label className={styles.field}>
              <div className={styles.fieldText}>
                <div className={styles.fieldLabel}>Notify me about very interesting stories</div>
                <div className={styles.fieldHelp}>At most 3 per day, never at night (22:00–08:00).</div>
              </div>
              <input type="checkbox" checked={notifyHigh} onChange={(e) => setNotifyHigh(e.target.checked)} />
            </label>
          </div>
        </>
      )}

      {step === 3 && (
        <>
          <h1>Start with your Mac?</h1>
          <p className={styles.lead}>The app lives in the menu bar and uses very little power in the background.</p>
          <div className={styles.group}>
            <label className={styles.field}>
              <div className={styles.fieldText}>
                <div className={styles.fieldLabel}>Open Tech English when I log in</div>
              </div>
              <input type="checkbox" checked={launchAtLogin} onChange={(e) => setLaunchAtLogin(e.target.checked)} />
            </label>
          </div>
        </>
      )}

      <div className={styles.navRow}>
        {step > 0 && <button onClick={() => setStep(step - 1)}>Back</button>}
        <span className="spacer" />
        {step === 0 && chosenTopics.size === 0 && <span className="faint">Choose at least one topic</span>}
        {step === 1 && chosenFeeds.size === 0 && <span className="faint">Choose at least one source</span>}
        {step < STEPS - 1 ? (
          <button
            className="primary"
            disabled={(step === 0 && chosenTopics.size === 0) || (step === 1 && chosenFeeds.size === 0)}
            onClick={() => setStep(step + 1)}
          >
            Next
          </button>
        ) : (
          <button className="primary" disabled={saving} onClick={finish}>
            {saving ? "Saving…" : "Finish"}
          </button>
        )}
      </div>
      <Toasts />
    </div>
  );
}
