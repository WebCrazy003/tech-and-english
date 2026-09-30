import { useEffect, useMemo, useState } from "react";
import { accentOf, listEnglishVoices, pickDefaultVoice, speak, stop } from "../../lib/tts";
import { useApp } from "../../stores/app";
import { toastError } from "../../stores/toast";
import styles from "../pages.module.css";
import { useSettingsPatch } from "./useSettingsPatch";

const SAMPLE = "The model runs inference on your laptop, so your data stays private.";

export function VoiceSettings() {
  const { settings, mode } = useApp();
  const patch = useSettingsPatch();
  const [voices, setVoices] = useState<SpeechSynthesisVoice[]>([]);
  useEffect(() => {
    void listEnglishVoices().then(setVoices);
    return () => stop();
  }, []);
  const groups = useMemo(() => {
    const m = new Map<string, SpeechSynthesisVoice[]>();
    for (const v of voices) m.set(accentOf(v.lang), [...(m.get(accentOf(v.lang)) ?? []), v]);
    return [...m.entries()];
  }, [voices]);
  if (!settings) return null;
  const tts = settings.tts;
  const fallback = pickDefaultVoice(voices);
  const test = (rate: number, text = SAMPLE) =>
    void speak(text, { rate, voiceURI: tts.voiceUri, volume: tts.volume }).catch(toastError);

  return (
    <div>
      <div className={styles.groupTitle}>Voice</div>
      <div className={styles.group}>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="voice">English voice</label>
            <div className={styles.fieldHelp}>
              {voices.length ? `${voices.length} English voices on this Mac.` : "Loading the voices…"}
            </div>
          </div>
          <select id="voice" value={tts.voiceUri ?? ""} onChange={(e) => void patch({ tts: { voiceUri: e.target.value || null } })}>
            <option value="">Automatic{fallback ? ` (${fallback.name})` : ""}</option>
            {groups.map(([accent, list]) => (
              <optgroup key={accent} label={accent}>
                {list.map((v) => (
                  <option key={v.voiceURI} value={v.voiceURI}>
                    {v.name}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="rate">Reading speed (Listen)</label>
            <div className={styles.fieldHelp}>{tts.rate.toFixed(2)}× — slower is easier to follow</div>
          </div>
          <input
            id="rate"
            type="range"
            min={0.5}
            max={1.2}
            step={0.05}
            defaultValue={tts.rate}
            onMouseUp={(e) => void patch({ tts: { rate: Number((e.target as HTMLInputElement).value) } })}
            onKeyUp={(e) => void patch({ tts: { rate: Number((e.target as HTMLInputElement).value) } })}
          />
          <button onClick={() => test(tts.rate)} disabled={mode === "hibernate"}>
            Test
          </button>
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="wrate">Word speed (🔊 in the popup and quizzes)</label>
            <div className={styles.fieldHelp}>{tts.wordRate.toFixed(2)}×</div>
          </div>
          <input
            id="wrate"
            type="range"
            min={0.5}
            max={1.2}
            step={0.05}
            defaultValue={tts.wordRate}
            onMouseUp={(e) => void patch({ tts: { wordRate: Number((e.target as HTMLInputElement).value) } })}
            onKeyUp={(e) => void patch({ tts: { wordRate: Number((e.target as HTMLInputElement).value) } })}
          />
          <button onClick={() => test(tts.wordRate, "scalability")} disabled={mode === "hibernate"}>
            Test
          </button>
        </div>
      </div>
      <p className="faint" style={{ fontSize: 12, margin: "0 4px 20px" }}>
        Better voices: System Settings › Accessibility › Spoken Content › System Voice › Manage Voices… Download an
        English “Premium” voice, then choose it here.
        {mode === "hibernate" && " Speaking is off in Hibernate."}
      </p>
    </div>
  );
}

export function LearningSettings() {
  const settings = useApp((s) => s.settings);
  const patch = useSettingsPatch();
  if (!settings) return null;
  const l = settings.learning;
  return (
    <div>
      <div className={styles.groupTitle}>Practice</div>
      <div className={styles.group}>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="qsize">Words per quiz</label>
          </div>
          <select id="qsize" value={l.quizSize} onChange={(e) => void patch({ learning: { quizSize: Number(e.target.value) } })}>
            {[5, 10, 15, 20, 30].map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </div>
        <div className={styles.field}>
          <div className={styles.fieldText}>
            <label htmlFor="ret">How much you want to remember</label>
            <div className={styles.fieldHelp}>
              Higher means more reviews, spaced closer together. 90 % is a good balance.
            </div>
          </div>
          <select
            id="ret"
            value={l.desiredRetention}
            onChange={(e) => void patch({ learning: { desiredRetention: Number(e.target.value) } })}
          >
            {[0.8, 0.85, 0.9, 0.95].map((r) => (
              <option key={r} value={r}>
                {Math.round(r * 100)} %
              </option>
            ))}
          </select>
        </div>
        <label className={styles.field}>
          <div className={styles.fieldText}>
            <div className={styles.fieldLabel}>Say the word when the answer is shown</div>
          </div>
          <input
            type="checkbox"
            checked={l.autoPronounce}
            onChange={(e) => void patch({ learning: { autoPronounce: e.target.checked } })}
          />
        </label>
      </div>
    </div>
  );
}
