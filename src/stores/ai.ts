import { create } from "zustand";
import { api, EVENTS, onEvent, type AiOverview, type AiStatus } from "../lib/api";
import { toast } from "./toast";

interface Progress {
  bytes: number;
  total: number;
}

interface AiStore {
  status: AiStatus;
  overview: AiOverview | null;
  progress: Record<string, Progress>;
  refresh: () => Promise<void>;
}

export const useAi = create<AiStore>((set) => ({
  status: { state: "unloaded", modelId: null, message: null },
  overview: null,
  progress: {},
  refresh: async () => {
    const overview = await api.aiOverview();
    set({ overview, status: overview.status });
  },
}));

let started = false;

export function startAiStore(): void {
  if (started) return;
  started = true;
  void useAi.getState().refresh().catch(() => {});
  void onEvent<AiStatus>(EVENTS.aiStatus, (status) => useAi.setState({ status }));
  void onEvent<{ modelId: string; bytes?: number; total?: number; done?: boolean; error?: string }>(
    EVENTS.aiDownload,
    (p) => {
      if (p.bytes != null && p.total != null) {
        useAi.setState((s) => ({ progress: { ...s.progress, [p.modelId]: { bytes: p.bytes!, total: p.total! } } }));
        return;
      }
      useAi.setState((s) => {
        const progress = { ...s.progress };
        delete progress[p.modelId];
        return { progress };
      });
      if (p.done) toast("AI model downloaded. You can use the AI now.");
      else if (p.error && p.error !== "Download cancelled") toast(`Download failed: ${p.error}`);
      void useAi.getState().refresh();
    },
  );
}

/** "2.7 GB" */
export function gb(bytes: number): string {
  return `${(bytes / 1e9).toFixed(1)} GB`;
}
