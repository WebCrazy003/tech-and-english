import { create } from "zustand";

/** Text the user wants to ask the AI about ("Ask AI about this"). The chat panel consumes it. */
interface ReaderStore {
  quote: string | null;
  setQuote: (q: string | null) => void;
}

export const useReader = create<ReaderStore>((set) => ({
  quote: null,
  setQuote: (quote) => set({ quote }),
}));
