import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router";
import { api, ensureNotificationPermission, type Engines, type FeedSeed, type ModelInfo, type TopicSeed } from "../lib/api";
import { gb, useAi } from "../stores/ai";
import { useApp } from "../stores/app";
import { toastError } from "../stores/toast";
import Toasts from "../components/Toasts";
import styles from "./pages.module.css";

const STEPS = 5;

/** Step 5 (P6 §3): optional AI downloads. They continue in the background after Finish. */
function AiStep({
  engines,
  llm,
  stt,
  want,
  setWant,
}: {
  engines: Engines | null;
  llm?: ModelInfo;
  stt?: ModelInfo;
  want: { llm: boolean; stt: boolean };
  setWant: (w: { llm: boolean; stt: boolean }) => void;
}) {
  const row = (
    key: "llm" | "stt",
    title: string,
    m: ModelInfo | undefined,
    engine: { ok: boolean } | undefined,
    use: string,
  ) => {
    const missing = engines !== null && !engine?.ok;
    return (
      <label className={styles.field}>
        <div className={styles.fieldText}>
          <div className={styles.fieldLabel}>
            {title} — {m?.displayName ?? "…"}
          </div>
          <div className={styles.fieldHelp}>
            {m ? `${gb(m.sizeBytes)} · license ${m.license} · ${use}` : use}
            {m?.downloaded && " · already downloaded"}
          </div>
          {missing && <div className="error-text">AI engine missing — reinstall the app.</div>}
        </div>
        <input
          type="checkbox"
          checked={want[key] && !missing && !m?.downloaded}
          disabled={missing || !m || m.downloaded}
          onChange={(e) => setWant({ ...want, [key]: e.target.checked })}
        />
      </label>
    );
  };
  return (
    <>
      <h1>AI features</h1>
      <p className={styles.lead}>
        Optional. The AI runs on this Mac: nothing is sent to the internet, and it costs nothing. You can also do this
        later in Settings.
      </p>
      <div className={styles.group}>
        {row("llm", "Language model", llm, engines?.llama, "summaries, chat, word explanations, tutor")}
        {row("stt", "Speech recognition", stt, engines?.whisper, "the voice tutor hears you")}
        <div className={styles.field}>
          <div className={styles.fieldHelp}>
            {engines === null
              ? "Checking the AI engines… (the first time this can take half a minute)"
              : engines.freeDiskGb != null
                ? `Disk space available: ${Math.round(engines.freeDiskGb)} GB. The downloads continue in the background.`
                : "The downloads continue in the background."}
          </div>
        </div>
      </div>
    </>
  );
}

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
  const [engines, setEngines] = useState<Engines | null>(null);
  const [wantAi, setWantAi] = useState({ llm: true, stt: true });
  const models = useAi((s) => s.overview?.models);
  const llmModel = models?.find((m) => m.role === "chat" && m.default);
  const sttModel = models?.find((m) => m.role === "stt" && m.default);

  useEffect(() => {
    api
      .getOnboardingDefaults()
      .then((d) => {
        setTopics(d.topics);
        setFeeds(d.feeds);
        setChosenFeeds(new Set(d.feeds.map((f) => f.url)));
      })
      .catch(toastError);
    api
      .checkEngines()
      .then(setEngines)
      .catch(() => setEngines({ llama: { path: null, ok: false }, whisper: { path: null, ok: false }, freeDiskGb: null }));
    void useAi.getState().refresh().catch(() => {});
  }, []);

  const groups = useMemo(() => {
    const m = new Map<string, FeedSeed[]>();
    for (const f of feeds) m.set(f.group || "Other", [...(m.get(f.group || "Other") ?? []), f]);
    return [...m.entries()];
  }, [feeds]);

  // What "Download and finish" will download: chosen, not there yet, and its engine works.
  const toDownload = [
    wantAi.llm && engines?.llama.ok ? llmModel : undefined,
    wantAi.stt && engines?.whisper.ok ? sttModel : undefined,
  ].filter((m): m is ModelInfo => !!m && !m.downloaded);

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
      // The AI downloads run in the background (progress: widget footer and Settings › AI).
      for (const m of toDownload) void api.downloadModel(m.id).catch(toastError);
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

      {step === 4 && <AiStep engines={engines} llm={llmModel} stt={sttModel} want={wantAi} setWant={setWantAi} />}

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
          <>
            {toDownload.length > 0 && (
              <button disabled={saving} onClick={() => setWantAi({ llm: false, stt: false })}>
                Later
              </button>
            )}
            <button className="primary" disabled={saving} onClick={finish}>
              {saving ? "Saving…" : toDownload.length > 0 ? "Download and finish" : "Finish"}
            </button>
          </>
        )}
      </div>
      <Toasts />
    </div>
  );
}
