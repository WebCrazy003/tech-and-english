import { create } from "zustand";
import { api, EVENTS, onEvent, type ArticleListItem, type DailyPick, type Mode, type Settings } from "../lib/api";

interface AppStore {
  settings: Settings | null;
  mode: Mode;
  pick: DailyPick | null;
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
    const pick = await api.getTodayPick();
    const preview = pick ? null : await api.getPickPreview();
    set({ pick, preview });
  },
  setMode: async (m) => {
    const mode = await api.setMode(m);
    set({ mode });
  },
}));

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
  void onEvent<DailyPick>(EVENTS.pickChanged, (pick) => useApp.setState({ pick, preview: null }));
  void onEvent<Mode>(EVENTS.modeChanged, (mode) => useApp.setState({ mode }));
  void onEvent<Settings>(EVENTS.settingsChanged, (settings) => {
    useApp.setState({ settings });
    void useApp.getState().refreshPick();
  });
}
