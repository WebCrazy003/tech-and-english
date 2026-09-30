import { useCallback, useEffect, useRef, useState } from "react";
import { api, type StreamEvent } from "../../lib/api";

export type Phase = "idle" | "loading" | "streaming" | "done";

export interface StreamState {
  phase: Phase;
  text: string;
  error: { code: string; message: string } | null;
  cached: boolean;
}

const EMPTY: StreamState = { phase: "idle", text: "", error: null, cached: false };

/**
 * Runs one streaming AI request at a time. `start` receives an event handler and returns the job id.
 * Stop cancels the job; switching away (unmount) cancels too.
 */
export function useAiStream() {
  const [state, setState] = useState<StreamState>(EMPTY);
  const job = useRef<number | null>(null);
  const gen = useRef(0);

  const stop = useCallback(() => {
    if (job.current != null) void api.cancelJob(job.current);
  }, []);

  useEffect(() => () => stop(), [stop]);

  const run = useCallback(
    async (start: (onEvent: (e: StreamEvent) => void) => Promise<number>, onFinish?: (s: StreamState) => void) => {
      const my = ++gen.current;
      let current: StreamState = { phase: "streaming", text: "", error: null, cached: false };
      setState(current);
      const update = (next: StreamState) => {
        if (my !== gen.current) return;
        current = next;
        setState(next);
      };
      const onEvent = (e: StreamEvent) => {
        switch (e.kind) {
          case "loading":
            update({ ...current, phase: "loading" });
            break;
          case "delta":
            update({ ...current, phase: "streaming", text: current.text + e.text });
            break;
          case "done":
            update({ ...current, phase: "done", cached: e.cached });
            job.current = null;
            onFinish?.(current);
            break;
          case "error":
            update({ ...current, phase: "done", error: e.code === "cancelled" ? null : { code: e.code, message: e.message } });
            job.current = null;
            onFinish?.(current);
            break;
        }
      };
      try {
        job.current = await start(onEvent);
      } catch (e) {
        const err = e as { code?: string; message?: string };
        update({ ...current, phase: "done", error: { code: err.code ?? "unknown", message: err.message ?? String(e) } });
        onFinish?.(current);
      }
    },
    [],
  );

  const reset = useCallback(() => {
    gen.current++;
    setState(EMPTY);
  }, []);

  return { state, run, stop, reset, busy: state.phase === "loading" || state.phase === "streaming" };
}
