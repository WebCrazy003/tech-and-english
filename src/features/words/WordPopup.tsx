import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ApiError, api, type DefineTermOut, type DictEntry, type VocabKind } from "../../lib/api";
import { useAi } from "../../stores/ai";
import { useApp } from "../../stores/app";
import { toast, toastError } from "../../stores/toast";
import { textKey, useSpeech, useWords } from "../../stores/words";
import type { WordSelection } from "./selection";
import styles from "./words.module.css";

const WIDTH = 360;
const KIND_LABEL: Record<string, string> = { word: "word", phrase: "phrase", term: "technical term" };

/** Popup for a selected word or phrase: dictionary meaning, 🔊, AI explanation, Add to Word Book (SPEC §8.5). */
export default function WordPopup({
  sel,
  articleId,
  onClose,
  onAskAi,
}: {
  sel: WordSelection;
  articleId: number | null;
  onClose: () => void;
  onAskAi?: (question: string) => void;
}) {
  const mode = useApp((s) => s.mode);
  const hasModel = useAi((s) => s.overview?.models.some((m) => m.downloaded) ?? true);
  const saved = useWords((s) => s.keys.has(textKey(sel.term)));
  const speech = useSpeech();
  const [dict, setDict] = useState<DictEntry | null | undefined>(undefined);
  const [ai, setAi] = useState<DefineTermOut | null>(null);
  const [aiBusy, setAiBusy] = useState(false);
  const [aiError, setAiError] = useState<ApiError | null>(null);
  const [kind, setKind] = useState<VocabKind>(sel.term.includes(" ") ? "phrase" : "word");
  const [adding, setAdding] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: 0, top: 0 });
  const hibernate = mode === "hibernate";

  useEffect(() => {
    let alive = true;
    api
      .dictionaryLookup(sel.term)
      .then((e) => alive && setDict(e))
      .catch(() => alive && setDict(null));
    return () => {
      alive = false;
    };
  }, [sel.term]);

  // Below the selection if it fits, else above; always inside the window.
  useLayoutEffect(() => {
    const h = ref.current?.offsetHeight ?? 240;
    const left = Math.min(Math.max(8, sel.rect.left + sel.rect.width / 2 - WIDTH / 2), window.innerWidth - WIDTH - 8);
    const below = sel.rect.bottom + 8;
    const top = below + h < window.innerHeight - 8 ? below : Math.max(8, sel.rect.top - h - 8);
    setPos({ left, top });
  }, [sel, dict, ai, aiError]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDown);
    };
  }, [onClose]);

  const explain = async (force = false) => {
    setAiBusy(true);
    setAiError(null);
    try {
      setAi(await api.defineTerm(sel.term, sel.sentence, articleId, force));
    } catch (e) {
      setAiError(e instanceof ApiError ? e : new ApiError("unknown", String(e)));
    } finally {
      setAiBusy(false);
    }
  };

  const add = async () => {
    setAdding(true);
    const d = dict?.parsed ? dict : null;
    try {
      const r = await api.addVocabItem({
        kind,
        text: sel.term,
        meaningSimple: ai?.meaningSimple ?? d?.senses[0] ?? null,
        meaningB1: ai?.meaningB1 || null,
        partOfSpeech: ai?.partOfSpeech || d?.partOfSpeech || null,
        ipa: d?.pronunciation ?? ai?.ipa ?? null,
        syllables: ai?.syllables ?? d?.syllables ?? null,
        examples: ai?.examples ?? d?.examples ?? [],
        collocations: ai?.collocations ?? [],
        context: { sentence: sel.sentence || null, articleId },
      });
      toast(r.outcome === "created" ? `Added "${r.item.text}" to your Word Book` : `Added a new example for "${r.item.text}"`);
      onClose();
    } catch (e) {
      toastError(e);
    } finally {
      setAdding(false);
    }
  };

  const aiReason = hibernate
    ? "Switch to Standard to use the AI"
    : !hasModel
      ? "Download an AI model in Settings › AI first"
      : undefined;

  return (
    <div
      ref={ref}
      className={styles.popup}
      style={{ left: pos.left, top: pos.top, width: WIDTH }}
      role="dialog"
      aria-label={`Meaning of ${sel.term}`}
      onMouseUp={(e) => e.stopPropagation()}
    >
      <div className={styles.head}>
        <strong className={styles.term}>{dict?.parsed ? dict.headword : sel.term}</strong>
        {dict?.pronunciation && <span className={styles.pron}>/{dict.pronunciation}/</span>}
        {(dict?.partOfSpeech || ai?.partOfSpeech) && <em className={styles.pos}>{dict?.partOfSpeech ?? ai?.partOfSpeech}</em>}
        <span className="spacer" />
        <button
          className="ghost"
          onClick={() => speech.word(sel.term)}
          disabled={speech.disabled}
          title={speech.disabled ? "Switch to Standard" : "Listen"}
          aria-label={`Listen to ${sel.term}`}
        >
          🔊
        </button>
      </div>

      {dict === undefined ? (
        <div className={styles.skeleton} aria-label="Looking up">
          <span />
          <span />
        </div>
      ) : dict === null ? (
        <p className={styles.none}>Not in the dictionary.{aiReason ? "" : " Try Explain simply."}</p>
      ) : dict.parsed ? (
        <ol className={styles.senses}>
          {dict.senses.map((s) => (
            <li key={s}>{s}</li>
          ))}
        </ol>
      ) : (
        <p className={styles.raw}>
          {dict.senses[0]}
          <br />
          <span className="faint">This dictionary entry has an unusual format. Explain simply may be clearer.</span>
        </p>
      )}

      {ai && (
        <div className={styles.ai}>
          <div>
            <strong>Simple:</strong> {ai.meaningSimple}
          </div>
          {ai.meaningB1 && <div className={styles.b1}>{ai.meaningB1}</div>}
          {ai.examples.length > 0 && (
            <ul>
              {ai.examples.map((e) => (
                <li key={e}>{e}</li>
              ))}
            </ul>
          )}
          {ai.collocations.length > 0 && <div className="faint">Often with: {ai.collocations.join(" · ")}</div>}
          {ai.syllables && <div className="faint">Syllables (hint): {ai.syllables}</div>}
        </div>
      )}
      {aiBusy && <p className="faint">Asking the AI… (the first time it starts, about 10 seconds)</p>}
      {aiError && (
        <p className="error-text" style={{ fontSize: 12 }}>
          {aiError.message}{" "}
          {aiError.code === "low_memory" && (
            <button className="ghost" onClick={() => void explain(true)}>
              Start anyway
            </button>
          )}
        </p>
      )}

      <div className={styles.row}>
        {!ai && (
          <button onClick={() => void explain()} disabled={aiBusy || !!aiReason} title={aiReason}>
            ✦ Explain simply
          </button>
        )}
        {onAskAi && (
          <button className="ghost" onClick={() => onAskAi(`What does "${sel.term}" mean in this story?`)}>
            Ask AI
          </button>
        )}
      </div>

      {sel.sentence && <p className={styles.context}>“{sel.sentence}”</p>}

      <div className={styles.row}>
        <label className={styles.kind}>
          Type
          <select value={kind} onChange={(e) => setKind(e.target.value as VocabKind)} aria-label="Type">
            {Object.entries(KIND_LABEL).map(([k, l]) => (
              <option key={k} value={k}>
                {l}
              </option>
            ))}
          </select>
        </label>
        <span className="spacer" />
        <button
          className="primary"
          onClick={() => void add()}
          disabled={adding || hibernate}
          title={hibernate ? "Switch to Standard to add words" : undefined}
        >
          {saved ? "Add this context" : "Add to Word Book"}
        </button>
      </div>
    </div>
  );
}
