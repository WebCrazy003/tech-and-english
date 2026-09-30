import { useEffect, useState, type ReactNode } from "react";
import { useNavigate, useParams } from "react-router";
import { api, type VocabDetail, type VocabKind } from "../../lib/api";
import { timeAgo } from "../../lib/format";
import { useApp } from "../../stores/app";
import { toast, toastError } from "../../stores/toast";
import { useSpeech } from "../../stores/words";
import { KIND_LABEL, dueLabel } from "./WordBook";
import styles from "./words.module.css";

/** The sentence with the saved word in bold. */
function Marked({ sentence, term }: { sentence: string; term: string }) {
  const i = sentence.toLowerCase().indexOf(term.toLowerCase());
  if (i < 0) return <>{sentence}</>;
  const parts: ReactNode[] = [sentence.slice(0, i), <strong key="t">{sentence.slice(i, i + term.length)}</strong>, sentence.slice(i + term.length)];
  return <>{parts}</>;
}

const GRADE: Record<string, string> = { forgot: "Forgot", unsure: "Not sure", remember: "Remembered" };

export default function WordDetail() {
  const { id } = useParams();
  const itemId = Number(id);
  const navigate = useNavigate();
  const mode = useApp((s) => s.mode);
  const speech = useSpeech();
  const [d, setD] = useState<VocabDetail | null>(null);
  const [form, setForm] = useState({ text: "", kind: "word" as VocabKind, meaningSimple: "", meaningB1: "", partOfSpeech: "", ipa: "", examples: "", notes: "" });
  const [dirty, setDirty] = useState(false);
  const readOnly = mode === "hibernate";

  useEffect(() => {
    api
      .getVocabItem(itemId)
      .then((r) => {
        setD(r);
        const i = r.item;
        setForm({
          text: i.text,
          kind: i.kind,
          meaningSimple: i.meaningSimple ?? "",
          meaningB1: i.meaningB1 ?? "",
          partOfSpeech: i.partOfSpeech ?? "",
          ipa: i.ipa ?? "",
          examples: i.examples.join("\n"),
          notes: i.notes ?? "",
        });
        setDirty(false);
      })
      .catch(toastError);
  }, [itemId]);

  if (!d) return <div className={styles.page}>Loading…</div>;
  const i = d.item;
  const set = (k: keyof typeof form, v: string) => {
    setForm({ ...form, [k]: v });
    setDirty(true);
  };

  const save = () =>
    api
      .updateVocabItem(itemId, { ...form, examples: form.examples.split("\n").map((x) => x.trim()).filter(Boolean) })
      .then((item) => {
        setD({ ...d, item });
        setDirty(false);
        toast("Saved");
      })
      .catch(toastError);

  const remove = () => {
    if (!window.confirm(`Delete "${i.text}" from your Word Book? Its practice history is deleted too.`)) return;
    api
      .deleteVocabItem(itemId)
      .then(() => navigate("/wordbook"))
      .catch(toastError);
  };

  const field = (k: keyof typeof form, label: string, opts: { area?: boolean; hint?: string } = {}) => (
    <label className={styles.field}>
      <span>{label}</span>
      {opts.area ? (
        <textarea value={form[k]} onChange={(e) => set(k, e.target.value)} rows={3} disabled={readOnly} />
      ) : (
        <input value={form[k]} onChange={(e) => set(k, e.target.value)} disabled={readOnly} />
      )}
      {opts.hint && <small className="faint">{opts.hint}</small>}
    </label>
  );

  return (
    <div className={styles.page}>
      <button className="ghost" onClick={() => navigate(-1)}>
        ← Back
      </button>
      <div className={styles.detailHead}>
        <h1>{i.text}</h1>
        <button className="ghost" onClick={() => speech.word(i.text)} disabled={speech.disabled} aria-label="Listen">
          🔊
        </button>
        {i.ipa && <span className="muted">/{i.ipa}/</span>}
        {i.syllables && <span className="faint">{i.syllables}</span>}
      </div>
      <div className={styles.meta}>
        <span className={`${styles.status} ${styles[`st_${i.status}`]}`}>{i.status}</span>
        <span>{dueLabel(i)}</span>
        <span>
          practised {i.reviewCount}× · ✓ {i.rememberCount} · ~ {i.unsureCount} · ✗ {i.forgotCount}
        </span>
        {i.collocations.length > 0 && <span>Often with: {i.collocations.join(" · ")}</span>}
      </div>

      <div className={styles.card}>
        <div className={styles.grid2}>
          {field("text", "Word or phrase")}
          <label className={styles.field}>
            <span>Type</span>
            <select value={form.kind} onChange={(e) => set("kind", e.target.value)} disabled={readOnly}>
              {Object.entries(KIND_LABEL).map(([k, l]) => (
                <option key={k} value={k}>
                  {l}
                </option>
              ))}
            </select>
          </label>
        </div>
        {field("meaningSimple", "Simple meaning")}
        {field("meaningB1", "B1 meaning", { area: true })}
        <div className={styles.grid2}>
          {field("partOfSpeech", "Part of speech")}
          {field("ipa", "Pronunciation")}
        </div>
        {field("examples", "Examples", { area: true, hint: "One per line" })}
        {field("notes", "Notes", { area: true })}
        <div className="row">
          <button className="danger" onClick={remove} disabled={readOnly}>
            Delete
          </button>
          <span className="spacer" />
          {readOnly && <span className="faint">Read-only in Hibernate</span>}
          <button className="primary" onClick={() => void save()} disabled={!dirty || readOnly}>
            Save
          </button>
        </div>
      </div>

      <h2 className={styles.h2}>Where you saw it</h2>
      {d.contexts.length === 0 && <p className="faint">Added by hand.</p>}
      {d.contexts.map((c) => (
        <div key={c.id} className={styles.context}>
          {c.sentence && (
            <p>
              “<Marked sentence={c.sentence} term={i.text} />”
            </p>
          )}
          <div className="faint">
            {c.articleId && c.articleTitle ? (
              <button className="ghost" onClick={() => navigate(`/reader/${c.articleId}`)}>
                {c.articleTitle} →
              </button>
            ) : c.articleUrl ? null : (
              "No story"
            )}{" "}
            · {timeAgo(c.createdAt)}
          </div>
        </div>
      ))}

      {d.reviews.length > 0 && (
        <>
          <h2 className={styles.h2}>Practice history</h2>
          <ul className={styles.reviews}>
            {d.reviews.map((r) => (
              <li key={r.reviewedAt}>
                <span className={styles[`g_${r.grade}`]}>{GRADE[r.grade]}</span> · {new Date(r.reviewedAt).toLocaleString()}
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
