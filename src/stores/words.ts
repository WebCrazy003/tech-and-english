import { create } from "zustand";
import { useCallback, useEffect, useState } from "react";
import { api, EVENTS, onEvent, type DueCount } from "../lib/api";
import { readAlong, speak, speakSentences, stop, toSentences } from "../lib/tts";
import { useApp } from "./app";
import { toastError } from "./toast";

interface WordsStore {
  /** text_key of every Word Book item (for "already saved" and highlights). */
  keys: Set<string>;
  due: DueCount | null;
  refresh: () => Promise<void>;
}

export const useWords = create<WordsStore>((set) => ({
  keys: new Set(),
  due: null,
  refresh: async () => {
    const [keys, due] = await Promise.all([api.listVocabKeys(), api.dueCount()]);
    set({ keys: new Set(keys), due });
  },
}));

let started = false;

/** Load keys and due counts once per window, and follow `vocab://changed`. */
export function startWordsStore(): void {
  if (started) return;
  started = true;
  void useWords.getState().refresh().catch(() => {});
  void onEvent(EVENTS.vocabChanged, () => void useWords.getState().refresh().catch(() => {}));
}

/** Same rule as the Rust `text_key`. */
export function textKey(s: string): string {
  return s
    .split(/\s+/)
    .filter(Boolean)
    .join(" ")
    .replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "")
    .toLowerCase();
}

/** Speaking with the user's voice settings. Off in Hibernate (SPEC §6). */
export function useSpeech() {
  const settings = useApp((s) => s.settings);
  const mode = useApp((s) => s.mode);
  const [speaking, setSpeaking] = useState(false);
  const disabled = mode === "hibernate";
  const tts = settings?.tts;

  useEffect(() => () => stop(), []);

  const word = useCallback(
    (text: string) => {
      if (disabled || !tts) return;
      void speak(text, { rate: tts.wordRate, voiceURI: tts.voiceUri, volume: tts.volume }).catch(toastError);
    },
    [disabled, tts],
  );

  const read = useCallback(
    async (text: string) => {
      if (disabled || !tts) return;
      stop();
      setSpeaking(true);
      try {
        await speakSentences(toSentences(text), { rate: tts.rate, voiceURI: tts.voiceUri, volume: tts.volume, pauseMs: tts.pauseMs });
      } catch (e) {
        toastError(e);
      } finally {
        setSpeaking(false);
      }
    },
    [disabled, tts],
  );

  /** Read an element's text as shown on screen, highlighting the word being spoken. */
  const readElement = useCallback(
    async (el: HTMLElement) => {
      if (disabled || !tts) return;
      setSpeaking(true);
      try {
        await readAlong(el, { rate: tts.rate, voiceURI: tts.voiceUri, volume: tts.volume, pauseMs: tts.pauseMs });
      } catch (e) {
        toastError(e);
      } finally {
        setSpeaking(false);
      }
    },
    [disabled, tts],
  );

  const halt = useCallback(() => {
    stop();
    setSpeaking(false);
  }, []);

  return { word, read, readElement, stop: halt, speaking, disabled };
}
