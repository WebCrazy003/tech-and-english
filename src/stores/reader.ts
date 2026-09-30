import { create } from "zustand";

/**
 * Messages for the chat panel from the article:
 * - `quote`: "Ask AI about this" puts the text into the input,
 * - `question`: the word popup's "Ask AI" sends a question right away.
 */
interface ReaderStore {
  quote: string | null;
  setQuote: (q: string | null) => void;
  question: string | null;
  setQuestion: (q: string | null) => void;
}

export const useReader = create<ReaderStore>((set) => ({
  quote: null,
  setQuote: (quote) => set({ quote }),
  question: null,
  setQuestion: (question) => set({ question }),
}));
