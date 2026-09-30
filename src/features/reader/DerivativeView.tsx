import { useCallback, useEffect } from "react";
import { api, type DerivKind } from "../../lib/api";
import RichText from "../../components/RichText";
import { AiProblem } from "../ai/AiSetup";
import { useAiStream } from "./useAiStream";
import styles from "./reader.module.css";

/** B1 Summary / Easy English tab: cached text, or streamed on first use. */
export default function DerivativeView({ articleId, kind }: { articleId: number; kind: DerivKind }) {
  const { state, run, stop, busy } = useAiStream();

  const load = useCallback(
    (opts: { regenerate?: boolean; force?: boolean } = {}) =>
      void run(async (onEvent) => (await api.getDerivative(articleId, kind, onEvent, opts)).jobId),
    [articleId, kind, run],
  );

  useEffect(() => load(), [load]);

  return (
    <div className={styles.derivative}>
      {state.phase === "loading" && <p className={styles.status}>Starting the AI… (about 10 seconds the first time)</p>}
      {state.phase === "streaming" && !state.text && (
        <p className={styles.status}>Reading the article… (up to 15 seconds for a long article)</p>
      )}
      {state.text && (
        <div className={`${styles.body} ${busy ? styles.typing : ""}`} data-deriv={kind}>
          <RichText text={state.text} />
        </div>
      )}
      {state.error && <AiProblem code={state.error.code} message={state.error.message} retry={(force) => load({ force })} />}
      <div className={styles.derivActions}>
        {busy && (
          <button className="ghost" onClick={stop}>
            ■ Stop
          </button>
        )}
        {state.phase === "done" && !state.error && state.text && (
          <>
            <span className="faint">{state.cached ? "Saved summary" : "Written by the AI on your Mac"}</span>
            <button className="ghost" onClick={() => load({ regenerate: true })}>
              ↻ Write again
            </button>
          </>
        )}
      </div>
    </div>
  );
}
