// Text-to-speech with the macOS voices through the webview's speechSynthesis (P4 dev spec §5).

import { clearSpoken, markSentence, markWord, sentencesOf } from "./spokenHighlight";

export interface TtsOptions {
  rate: number;
  voiceURI?: string | null;
  volume?: number;
  /** Called when the audio actually starts (latency measurement, P5). */
  onStart?: () => void;
  /** Called for each spoken word: its position in the text (read-along highlighting). */
  onWord?: (charIndex: number, charLength: number) => void;
}

const synth = (): SpeechSynthesis | null => (typeof window !== "undefined" && "speechSynthesis" in window ? window.speechSynthesis : null);

/** macOS novelty voices: fun, but not for learning pronunciation. */
const NOVELTY = new Set([
  "Albert", "Bad News", "Bahh", "Bells", "Boing", "Bubbles", "Cellos", "Good News", "Jester", "Organ",
  "Superstar", "Trinoids", "Whisper", "Wobble", "Zarvox",
]);

export function isLearningVoice(v: SpeechSynthesisVoice): boolean {
  return v.lang.toLowerCase().startsWith("en") && !NOVELTY.has(v.name);
}

/** English voices; waits for `voiceschanged` (at most 2 s) because the list loads late. */
export function listEnglishVoices(): Promise<SpeechSynthesisVoice[]> {
  const s = synth();
  if (!s) return Promise.resolve([]);
  const english = () => s.getVoices().filter(isLearningVoice);
  if (english().length) return Promise.resolve(english());
  return new Promise((resolve) => {
    const done = () => {
      s.removeEventListener("voiceschanged", done);
      resolve(english());
    };
    s.addEventListener("voiceschanged", done);
    setTimeout(done, 2000);
  });
}

/**
 * Premium, then Enhanced (the name or the voice ID says so, e.g. com.apple.voice.premium.en-US.Zoe),
 * then a normal en-US, en-GB, any English voice. The robotic "eloquence" voices come last.
 */
export function pickDefaultVoice(voices: SpeechSynthesisVoice[]): SpeechSynthesisVoice | undefined {
  const en = voices.filter(isLearningVoice);
  const id = (v: SpeechSynthesisVoice) => `${v.name} ${v.voiceURI}`;
  const natural = en.filter((v) => !/eloquence/i.test(v.voiceURI));
  return (
    en.find((v) => /premium/i.test(id(v))) ??
    en.find((v) => /enhanced/i.test(id(v))) ??
    natural.find((v) => v.lang === "en-US") ??
    natural.find((v) => v.lang === "en-GB") ??
    natural[0] ??
    en[0]
  );
}

/** Accent group for the voice picker. */
export function accentOf(lang: string): string {
  const region = lang.split(/[-_]/)[1]?.toUpperCase();
  const names: Record<string, string> = { US: "US", GB: "UK", AU: "Australia", IE: "Ireland", IN: "India", ZA: "South Africa" };
  return (region && names[region]) || "Other";
}

async function voiceFor(uri?: string | null): Promise<SpeechSynthesisVoice | undefined> {
  const voices = await listEnglishVoices();
  return voices.find((v) => v.voiceURI === uri) ?? pickDefaultVoice(voices);
}

// WebKit may garbage-collect a speaking utterance, and then "end" never fires: keep a reference.
const alive = new Set<SpeechSynthesisUtterance>();

/** Speak once; resolves at the end (also when stopped), rejects on an error. */
export async function speak(text: string, opts: TtsOptions): Promise<void> {
  const s = synth();
  if (!s || !text.trim()) return;
  const voice = await voiceFor(opts.voiceURI);
  return new Promise((resolve, reject) => {
    const u = new SpeechSynthesisUtterance(text);
    u.rate = opts.rate;
    u.volume = opts.volume ?? 1;
    if (voice) {
      u.voice = voice;
      u.lang = voice.lang;
    } else {
      u.lang = "en-US";
    }
    if (opts.onStart) u.onstart = opts.onStart;
    if (opts.onWord) {
      const onWord = opts.onWord;
      u.onboundary = (e) => {
        if (e.name === "word") onWord(e.charIndex, e.charLength ?? 0);
      };
    }
    u.onend = () => {
      alive.delete(u);
      resolve();
    };
    u.onerror = (e) => {
      alive.delete(u);
      if (e.error === "interrupted" || e.error === "canceled") resolve();
      else reject(new Error(e.error));
    };
    alive.add(u);
    s.speak(u);
  });
}

let stopped = false;

/** Speak sentence by sentence with a short pause between them. */
export async function speakSentences(sentences: string[], opts: TtsOptions & { pauseMs: number }): Promise<void> {
  stopped = false;
  for (const s of sentences) {
    if (stopped) return;
    await speak(s, opts);
    if (stopped) return;
    await new Promise((r) => setTimeout(r, opts.pauseMs));
  }
}

export function stop(): void {
  stopped = true;
  readAlongRun++;
  synth()?.cancel();
  clearSpoken();
}

let readAlongRun = 0;

/** Read the text of `root` as shown on screen, highlighting the sentence and word being spoken. */
export async function readAlong(root: HTMLElement, opts: TtsOptions & { pauseMs: number }): Promise<void> {
  stop();
  const run = readAlongRun;
  try {
    for (const s of sentencesOf(root)) {
      if (run !== readAlongRun) return;
      if (s.text === "Based on the short description only.") continue;
      markSentence(s);
      await speak(s.text, { ...opts, onWord: (i, len) => run === readAlongRun && markWord(s, i, len) });
      if (run !== readAlongRun) return;
      await new Promise((r) => setTimeout(r, opts.pauseMs));
    }
  } finally {
    if (run === readAlongRun) clearSpoken();
  }
}

/** Split text into sentences for `speakSentences`. Markdown marks are removed. */
export function toSentences(text: string): string[] {
  const plain = text
    .replace(/\*\*|__|`|#+ /g, "")
    .replace(/^\s*[-•*]\s+/gm, "")
    .replace(/_Based on the short description only._/g, "");
  const seg = typeof Intl !== "undefined" && "Segmenter" in Intl ? new Intl.Segmenter("en", { granularity: "sentence" }) : null;
  const parts = seg
    ? Array.from(seg.segment(plain), (s) => s.segment)
    : plain.split(/(?<=[.!?])\s+/);
  return parts.map((s) => s.replace(/\s+/g, " ").trim()).filter(Boolean);
}
