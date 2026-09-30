import { create } from "zustand";
import { api, EVENTS, onEvent, type ArticleListItem, type DailyPick, type Mode, type Settings } from "../lib/api";

interface AppStore {
  settings: Settings | null;
  mode: Mode;
  pick: DailyPick | null;
  /** Today's lesson (P3), or null when there is none. */
  lesson: DailyPick | null;
  preview: ArticleListItem | null;
  /** Bumped on every news://updated so lists know to reload. */
  newsVersion: number;
  lastNewCount: number;
  loaded: boolean;
  load: () => Promise<void>;
  refreshPick: () => Promise<void>;
  setMode: (m: Mode) => Promise<void>;
}

export const useApp = create<AppStore>((set, get) => ({
  settings: null,
  mode: "standard",
  pick: null,
  lesson: null,
  preview: null,
  newsVersion: 0,
  lastNewCount: 0,
  loaded: false,
  load: async () => {
    const [settings, mode] = await Promise.all([api.getSettings(), api.getMode()]);
    set({ settings, mode, loaded: true });
    await get().refreshPick();
  },
  refreshPick: async () => {
    const [pick, lesson] = await Promise.all([api.getTodayPick(), api.getTodayLesson()]);
    const preview = pick ? null : await api.getPickPreview();
    set({ pick, lesson, preview });
  },
  setMode: async (m) => {
    const mode = await api.setMode(m);
    set({ mode });
  },
}));

/** Text size of stories, quiz cards and the talk transcript (Settings › General). */
function applyReadingScale(settings: Settings | null) {
  if (typeof document === "undefined") return;
  document.documentElement.style.setProperty("--reading-scale", String(settings?.readingScale ?? 1));
}
useApp.subscribe((s, prev) => {
  if (s.settings?.readingScale !== prev.settings?.readingScale) applyReadingScale(s.settings);
});

let started = false;

/** Load state and subscribe to backend events once per window. */
export function startAppStore(): void {
  if (started) return;
  started = true;
  const s = useApp.getState();
  void s.load();
  void onEvent<{ newCount: number }>(EVENTS.newsUpdated, (p) => {
    useApp.setState((st) => ({ newsVersion: st.newsVersion + 1, lastNewCount: p.newCount }));
    void useApp.getState().refreshPick();
  });
  void onEvent<DailyPick>(EVENTS.pickChanged, (p) =>
    useApp.setState(p.kind === "lesson" ? { lesson: p } : { pick: p, preview: null }),
  );
  void onEvent<Mode>(EVENTS.modeChanged, (mode) => useApp.setState({ mode }));
  void onEvent<Settings>(EVENTS.settingsChanged, (settings) => {
    useApp.setState({ settings });
    void useApp.getState().refreshPick();
  });
}
