import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { api, type VocabFilter, type VocabItem, type VocabKind, type VocabStatus } from "../../lib/api";
import { useApp } from "../../stores/app";
import { toast, toastError } from "../../stores/toast";
import { useWords } from "../../stores/words";
import styles from "./words.module.css";

export const KIND_ICON: Record<VocabKind, string> = {
  word: "Aa",
  phrase: "“ ”",
  term: "⚙︎",
  sentence: "¶",
  pronunciation: "🔊",
  correction: "✓",
};
export const KIND_LABEL: Record<VocabKind, string> = {
  word: "Word",
  phrase: "Phrase",
  term: "Technical term",
  sentence: "Sentence",
  pronunciation: "Pronunciation",
  correction: "Correction",
};

export function dueLabel(item: VocabItem): string {
  if (!item.dueAt || item.reviewCount === 0) return "not practised yet";
  const ms = new Date(item.dueAt).getTime() - Date.now();
  if (ms <= 0) return "due now";
  const days = Math.round(ms / 86_400_000);
  if (days < 1) return "due today";
  return days === 1 ? "due tomorrow" : `due in ${days} days`;
}

const STATUS: { id: VocabStatus | ""; label: string }[] = [
  { id: "", label: "All" },
  { id: "new", label: "New" },
  { id: "learning", label: "Learning" },
  { id: "known", label: "Known" },
];

/** Add a word by hand (no article). */
function QuickAdd({ onAdded }: { onAdded: () => void }) {
  const mode = useApp((s) => s.mode);
  const [text, setText] = useState("");
  const [meaning, setMeaning] = useState("");
  const add = async () => {
    try {
      const r = await api.addVocabItem({
        kind: text.trim().includes(" ") ? "phrase" : "word",
        text,
        meaningSimple: meaning || null,
      });
      toast(r.outcome === "created" ? `Added "${r.item.text}"` : `"${r.item.text}" was already there`);
      setText("");
      setMeaning("");
      onAdded();
    } catch (e) {
      toastError(e);
    }
  };
  return (
    <div className={styles.quickAdd}>
      <input value={text} onChange={(e) => setText(e.target.value)} placeholder="New word or phrase" aria-label="New word" />
      <input
        value={meaning}
        onChange={(e) => setMeaning(e.target.value)}
        placeholder="Meaning (optional — the dictionary or AI can fill it)"
        aria-label="Meaning"
        onKeyDown={(e) => e.key === "Enter" && text.trim() && void add()}
      />
      <button
        onClick={() => void add()}
        disabled={!text.trim() || mode === "hibernate"}
        title={mode === "hibernate" ? "Switch to Standard to add words" : undefined}
      >
        Add
      </button>
    </div>
  );
}

export default function WordBook() {
  const navigate = useNavigate();
  const due = useWords((s) => s.due);
  const keysVersion = useWords((s) => s.keys);
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<VocabKind | "">("");
  const [status, setStatus] = useState<VocabStatus | "">("");
  const [dueOnly, setDueOnly] = useState(false);
  const [pendingOnly, setPendingOnly] = useState(false);
  const [items, setItems] = useState<VocabItem[]>([]);
  const [total, setTotal] = useState(0);
  const [cursor, setCursor] = useState<number | null>(null);

  const spec = JSON.stringify({ query: query.trim(), kind, status, dueOnly, pendingOnly });
  const load = useCallback(
    (c: number | null) => {
      const f = JSON.parse(spec) as { query: string; kind: VocabKind | ""; status: VocabStatus | ""; dueOnly: boolean; pendingOnly: boolean };
      const filter: VocabFilter = {
        query: f.query || undefined,
        kind: f.kind || undefined,
        status: f.status || undefined,
        dueOnly: f.dueOnly,
        pendingOnly: f.pendingOnly,
      };
      return api
        .listVocab(filter, c, 100)
        .then((p) => {
          setItems((prev) => (c ? [...prev, ...p.items] : p.items));
          setTotal(p.total);
          setCursor(p.nextCursor);
        })
        .catch(toastError);
    },
    [spec],
  );
  useEffect(() => {
    const t = setTimeout(() => void load(null), 200);
    return () => clearTimeout(t);
  }, [load, keysVersion]);

  const exportCsv = () =>
    api
      .exportVocabCsv()
      .then((r) => toast(`Saved ${r.path.split("/").pop()} in Downloads`))
      .catch(toastError);

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <h1>Word Book</h1>
        <span className="muted">
          {due ? `${due.total} saved · ${due.due} due · ${due.new} new` : ""}
        </span>
        <span className="spacer" />
        <button onClick={() => navigate("/practice")} disabled={!due || due.ready < 3}>
          Practice
        </button>
        <button className="ghost" onClick={exportCsv} disabled={!total}>
          Export CSV
        </button>
      </div>

      <QuickAdd onAdded={() => void load(null)} />

      <div className={styles.filters}>
        <input type="search" placeholder="Search words and meanings…" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search" />
        <select value={kind} onChange={(e) => setKind(e.target.value as VocabKind | "")} aria-label="Type">
          <option value="">All types</option>
          {Object.entries(KIND_LABEL).map(([k, l]) => (
            <option key={k} value={k}>
              {l}
            </option>
          ))}
        </select>
        <div className={styles.chips} role="radiogroup" aria-label="Status">
          {STATUS.map((s) => (
            <button key={s.id} role="radio" aria-checked={status === s.id} className={status === s.id ? styles.chipOn : ""} onClick={() => setStatus(s.id)}>
              {s.label}
            </button>
          ))}
        </div>
        <label className={styles.check}>
          <input type="checkbox" checked={dueOnly} onChange={(e) => setDueOnly(e.target.checked)} /> Due
        </label>
        <label className={styles.check}>
          <input type="checkbox" checked={pendingOnly} onChange={(e) => setPendingOnly(e.target.checked)} /> Meaning pending
        </label>
      </div>

      <div className={styles.list}>
        {items.map((i) => (
          <button key={i.id} className={styles.item} onClick={() => navigate(`/wordbook/${i.id}`)}>
            <span className={styles.kindIcon} title={KIND_LABEL[i.kind]}>
              {KIND_ICON[i.kind]}
            </span>
            <span className={styles.itemMain}>
              <span className={styles.itemText}>{i.text}</span>
              <span className={styles.itemMeaning}>
                {i.meaningSimple ?? i.meaningB1 ?? <em className="faint">Meaning pending</em>}
              </span>
            </span>
            <span className={`${styles.status} ${styles[`st_${i.status}`]}`} title={i.status}>
              {i.status}
            </span>
            <span className={styles.due}>{dueLabel(i)}</span>
          </button>
        ))}
        {items.length === 0 && (
          <div className={styles.empty}>
            {total === 0 && !query && !kind && !status && !dueOnly && !pendingOnly ? (
              <>
                <p>Your Word Book is empty.</p>
                <p className="faint">Select a word in any story and press “Add to Word Book”.</p>
              </>
            ) : (
              <p>No words match these filters.</p>
            )}
          </div>
        )}
        {cursor && (
          <button className="ghost" onClick={() => void load(cursor)}>
            Show more
          </button>
        )}
      </div>
    </div>
  );
}
