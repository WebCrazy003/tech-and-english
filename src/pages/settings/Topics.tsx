import { useEffect, useState } from "react";
import { api, type Priority, type Topic, type TopicInput } from "../../lib/api";
import { PRIORITY_LABEL } from "../../lib/format";
import { toast, toastError } from "../../stores/toast";
import KeywordInput from "../../components/KeywordInput";
import styles from "../pages.module.css";

const EMPTY: TopicInput = {
  name: "",
  keywords: [],
  excludedKeywords: [],
  priority: 2,
  enabled: true,
  notify: false,
  notifyThreshold: null,
  learn: false,
};

/** A keyword topic like "learn data engineering" rarely matches news; the Learn switch works better. */
const looksLikeLearnTopic = (name: string) => /^\s*learn(ing)?\b/i.test(name);

export default function Topics() {
  const [topics, setTopics] = useState<Topic[]>([]);
  const [draft, setDraft] = useState<TopicInput>(EMPTY);
  const [preview, setPreview] = useState<{ matched: number; total: number } | null>(null);
  const [saving, setSaving] = useState(false);

  const load = async (selectId?: number) => {
    try {
      const list = await api.listTopics();
      setTopics(list);
      const sel = list.find((t) => t.id === selectId) ?? (selectId === undefined ? list[0] : undefined);
      setDraft(sel ? { ...sel } : EMPTY);
    } catch (e) {
      toastError(e);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const kwKey = JSON.stringify([draft.keywords, draft.excludedKeywords]);
  useEffect(() => {
    const [kw, ex] = JSON.parse(kwKey) as [string[], string[]];
    if (!kw.length) {
      setPreview(null);
      return;
    }
    const t = setTimeout(
      () =>
        api
          .previewTopicMatches(kw, ex)
          .then(setPreview)
          .catch(() => setPreview(null)),
      300,
    );
    return () => clearTimeout(t);
  }, [kwKey]);

  const save = async () => {
    setSaving(true);
    try {
      const t = await api.upsertTopic(draft);
      toast(`Saved "${t.name}"`);
      await load(t.id);
    } catch (e) {
      toastError(e);
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (!draft.id || !window.confirm(`Delete the topic "${draft.name}"?`)) return;
    try {
      await api.deleteTopic(draft.id);
      await load();
    } catch (e) {
      toastError(e);
    }
  };

  return (
    <div className={styles.split}>
      <div className={styles.list}>
        {topics.map((t) => (
          <button
            key={t.id}
            className={`${styles.listItem} ${draft.id === t.id ? styles.listItemActive : ""}`}
            onClick={() => setDraft({ ...t })}
          >
            <span style={{ opacity: t.enabled ? 1 : 0.4 }}>{t.name}</span>
            {t.learn && <span title="Learn on: lessons for this topic">📘</span>}
            {t.notify && <span title="Notifications on">🔔</span>}
          </button>
        ))}
        <button className={styles.listItem} onClick={() => setDraft(EMPTY)}>
          ＋ New topic
        </button>
      </div>

      <div className={`${styles.card} ${styles.editor}`}>
        <div>
          <label htmlFor="tname">Name</label>
          <input
            id="tname"
            type="text"
            value={draft.name}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            placeholder="For example: AI Agents"
            style={{ width: "100%" }}
          />
        </div>
        <div>
          <label>Keywords</label>
          <KeywordInput
            label="Keywords"
            value={draft.keywords}
            onChange={(keywords) => setDraft({ ...draft, keywords })}
            placeholder="Type a word and press Enter"
          />
          <div className={styles.preview}>
            {preview
              ? `Would match ${preview.matched} of the ${preview.total} stories from the last 3 days.`
              : "Add keywords to see how many stories match."}
          </div>
          {draft.id && looksLikeLearnTopic(draft.name) && preview?.matched === 0 && (
            <div className={styles.tip} role="note">
              <strong>Tip:</strong> learning topics work better as a switch. Turn on <strong>Learn</strong> for your
              subject topic (e.g. Data Engineering) and delete this one.
            </div>
          )}
        </div>
        <div>
          <label>Hide stories that contain</label>
          <KeywordInput
            label="Excluded keywords"
            value={draft.excludedKeywords}
            onChange={(excludedKeywords) => setDraft({ ...draft, excludedKeywords })}
            placeholder="Optional, for example: crypto"
          />
        </div>
        <label className={styles.switchRow}>
          <input
            type="checkbox"
            checked={draft.learn}
            onChange={(e) => setDraft({ ...draft, learn: e.target.checked })}
          />
          <span>
            <strong>Learn</strong>
            <span className={styles.fieldHelp}>
              Also find learning materials (tutorials, explainers) for this topic. One is chosen each day as
              Today's lesson.
            </span>
          </span>
        </label>
        <div className="row" style={{ gap: 20, flexWrap: "wrap" }}>
          <label className={styles.check}>
            Priority
            <select
              value={draft.priority}
              onChange={(e) => setDraft({ ...draft, priority: Number(e.target.value) as Priority })}
            >
              {([3, 2, 1] as Priority[]).map((p) => (
                <option key={p} value={p}>
                  {PRIORITY_LABEL[p]}
                </option>
              ))}
            </select>
          </label>
          <label className={styles.check}>
            <input
              type="checkbox"
              checked={draft.enabled}
              onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })}
            />
            Enabled
          </label>
          <label className={styles.check}>
            <input
              type="checkbox"
              checked={draft.notify}
              onChange={(e) => setDraft({ ...draft, notify: e.target.checked })}
            />
            Notify me about great stories
          </label>
          {draft.notify && (
            <label className={styles.check}>
              when score ≥
              <input
                type="number"
                min={0}
                max={100}
                placeholder="85"
                value={draft.notifyThreshold ?? ""}
                onChange={(e) =>
                  setDraft({ ...draft, notifyThreshold: e.target.value === "" ? null : Number(e.target.value) })
                }
                style={{ width: 70 }}
              />
            </label>
          )}
        </div>
        <div className="row">
          {draft.id && (
            <button className="danger" onClick={remove}>
              Delete
            </button>
          )}
          <span className="spacer" />
          <button className="primary" onClick={save} disabled={saving || !draft.name.trim() || !draft.keywords.length}>
            {saving ? "Saving…" : draft.id ? "Save changes" : "Add topic"}
          </button>
        </div>
      </div>
    </div>
  );
}
