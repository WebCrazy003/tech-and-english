// The live voice session (P5): messages, status flags, the speech queue and push-to-talk.
import { create } from "zustand";
import {
  ApiError,
  api,
  type Correction,
  type DrillInfo,
  type SessionReview,
  type SessionSettings,
  type TalkPhase,
  type VoiceEvent,
} from "../lib/api";
import { clampRate, SpeechQueue } from "../features/talk/speechQueue";
import { toast, toastError } from "./toast";

export interface TalkMessage {
  key: string;
  role: "tutor" | "user" | "notice";
  text: string;
  turn?: number;
  correction?: Correction | null;
  local?: boolean;
  streaming?: boolean;
}

interface TalkStore {
  conversationId: number | null;
  articleTitle: string | null;
  settings: SessionSettings | null;
  messages: TalkMessage[];
  phase: TalkPhase;
  repeatTarget: string | null;
  drill: DrillInfo | null;
  /** Engines being started: "llm" | "stt" | "summary". */
  loading: string[];
  starting: boolean;
  recording: boolean;
  /** A turn request is running (stop_recording / send_text_turn). */
  waiting: boolean;
  transcribing: boolean;
  speaking: boolean;
  ending: boolean;
  level: number;
  /** Set when the mic is not allowed (shows the help screen). */
  micDenied: boolean;
  /** Last tutor reply, for "say that again". */
  lastReply: string[];
}

const initial = (): TalkStore => ({
  conversationId: null,
  articleTitle: null,
  settings: null,
  messages: [],
  phase: "discuss",
  repeatTarget: null,
  drill: null,
  loading: [],
  starting: false,
  recording: false,
  waiting: false,
  transcribing: false,
  speaking: false,
  ending: false,
  level: 0,
  micDenied: false,
  lastReply: [],
});

export const useTalk = create<TalkStore>(() => initial());

let msgKey = 0;
const key = () => `m${++msgKey}`;

/** Voice turn whose "end of speech → first audio" time is being measured. */
let measuring: { turn: number | null; since: number } | null = null;

const queue = new SpeechQueue({
  voiceURI: () => useTalk.getState().settings?.voiceUri ?? null,
  pauseMs: () => useTalk.getState().settings?.pauseMs ?? 400,
  onSpeaking: (speaking) => useTalk.setState({ speaking }),
  onTurnStart: (turn) => {
    if (measuring && (measuring.turn === null || measuring.turn === turn) && Number.isInteger(turn)) {
      const ms = Math.round(performance.now() - measuring.since);
      measuring = null;
      void api.reportLatency(turn, ms).catch(() => {});
    }
  },
});

const rate = () => useTalk.getState().settings?.rate ?? 0.85;

function addMessage(m: Omit<TalkMessage, "key">) {
  useTalk.setState((s) => ({ messages: [...s.messages, { ...m, key: key() }] }));
}

function onEvent(e: VoiceEvent): void {
  const st = useTalk.getState();
  switch (e.kind) {
    case "loading":
      useTalk.setState({ loading: [...new Set([...st.loading, e.component])] });
      break;
    case "ready":
      useTalk.setState({ loading: [], conversationId: e.conversationId, settings: e.settings });
      break;
    case "transcript":
      useTalk.setState({ transcribing: false });
      addMessage({ role: "user", text: e.text });
      break;
    case "notice":
      useTalk.setState({ transcribing: false });
      addMessage({ role: "notice", text: e.text });
      break;
    case "tutorSentence": {
      if (measuring && measuring.turn === null) measuring.turn = e.turn;
      const r = e.rate ?? rate() + (e.rateDelta ?? 0);
      queue.push(e.turn, e.text, r);
      useTalk.setState((s) => {
        const i = s.messages.findIndex((m) => m.role === "tutor" && m.turn === e.turn);
        if (i < 0)
          return { messages: [...s.messages, { key: key(), role: "tutor", text: e.text, turn: e.turn, streaming: true }] };
        const messages = [...s.messages];
        messages[i] = { ...messages[i], text: `${messages[i].text} ${e.text}` };
        return { messages };
      });
      break;
    }
    case "tutorDone":
      useTalk.setState((s) => {
        const messages = s.messages.map((m) =>
          m.role === "tutor" && m.turn === e.turn
            ? { ...m, text: e.text, correction: e.correction, local: e.local, streaming: false }
            : m,
        );
        const sentences = messages.find((m) => m.turn === e.turn)?.text ?? e.text;
        return {
          messages,
          phase: e.phase,
          repeatTarget: e.repeatTarget,
          drill: e.drill,
          lastReply: splitSentences(sentences),
        };
      });
      break;
    case "localAction":
      if (e.action === "replay") queue.replay(st.lastReply, rate());
      if (e.action === "slower" || e.action === "faster") {
        const next = e.rate ?? rate();
        useTalk.setState((s) => ({ settings: s.settings ? { ...s.settings, rate: next } : s.settings }));
        toast(`Speaking speed ${next.toFixed(2)}×`);
        if (e.action === "slower") queue.replay(st.lastReply, next);
      }
      if (e.action === "end") void endTalk();
      break;
    case "error":
      useTalk.setState({ transcribing: false, loading: [] });
      if (e.code === "mic_denied") useTalk.setState({ micDenied: true });
      else if (e.code !== "cancelled") toast(e.message);
      break;
  }
}

function splitSentences(text: string): string[] {
  const seg = typeof Intl !== "undefined" && "Segmenter" in Intl ? new Intl.Segmenter("en", { granularity: "sentence" }) : null;
  const parts = seg ? Array.from(seg.segment(text), (s) => s.segment) : text.split(/(?<=[.!?])\s+/);
  return parts.map((s) => s.trim()).filter(Boolean);
}

async function runTurn(req: Promise<void>): Promise<void> {
  useTalk.setState({ waiting: true });
  try {
    await req;
  } catch (e) {
    // Errors were also sent on the channel; only report the ones that were not.
    if (!(e instanceof ApiError)) toastError(e);
  } finally {
    useTalk.setState({ waiting: false, transcribing: false });
  }
}

export async function startTalk(articleId: number | null, settings: SessionSettings, force = false): Promise<boolean> {
  queue.stop();
  useTalk.setState({ ...initial(), settings, starting: true });
  try {
    await api.startVoiceSession({ articleId, settings, force }, onEvent);
    return true;
  } catch (e) {
    useTalk.setState({ starting: false, loading: [] });
    throw e;
  } finally {
    useTalk.setState({ starting: false });
  }
}

/** After a window reload: show the running session (the transcript so far comes from the DB). */
export async function attachTalk(): Promise<boolean> {
  const active = await api.getActiveSession();
  if (!active) return false;
  if (useTalk.getState().conversationId === active.conversationId) return true;
  const { turns } = await api.getConversation(active.conversationId);
  useTalk.setState({
    ...initial(),
    conversationId: active.conversationId,
    articleTitle: active.articleTitle,
    settings: active.settings,
    messages: turns.map((t) => ({
      key: key(),
      role: t.role,
      text: t.text,
      correction: t.meta?.correction ?? null,
      local: t.meta?.local ?? false,
    })),
  });
  return true;
}

/** Mic down (Space pressed / first click). Stops the tutor at once (barge-in). */
export async function micDown(): Promise<void> {
  const s = useTalk.getState();
  if (s.recording || s.conversationId === null) return;
  queue.stop();
  useTalk.setState({ recording: true, level: 0 });
  try {
    await api.startRecording();
  } catch (e) {
    useTalk.setState({ recording: false });
    if (e instanceof ApiError && e.code === "mic_denied") useTalk.setState({ micDenied: true });
    else toastError(e);
  }
}

/** Mic up (Space released / second click / auto-stop). */
export async function micUp(): Promise<void> {
  if (!useTalk.getState().recording) return;
  useTalk.setState({ recording: false, transcribing: true, level: 0 });
  measuring = { turn: null, since: performance.now() };
  await runTurn(api.stopRecording(onEvent));
}

export async function sendTyped(text: string): Promise<void> {
  if (!text.trim()) return;
  queue.stop();
  measuring = null;
  await runTurn(api.sendTextTurn(text.trim(), onEvent));
}

export async function practise(word: string): Promise<void> {
  queue.stop();
  await runTurn(api.startDrill(word, onEvent));
}

export function replayLast(): void {
  queue.replay(useTalk.getState().lastReply, rate());
}

export function speakText(text: string, r = rate()): void {
  queue.replay([text], r);
}

export async function changeRate(delta: number): Promise<void> {
  const next = clampRate(rate() + delta);
  try {
    const settings = await api.updateSessionSettings({ rate: next });
    useTalk.setState({ settings });
  } catch (e) {
    toastError(e);
  }
}

export async function changeCorrection(correction: SessionSettings["correction"]): Promise<void> {
  try {
    const settings = await api.updateSessionSettings({ correction });
    useTalk.setState({ settings });
  } catch (e) {
    toastError(e);
  }
}

let endListeners: ((r: SessionReview | null, id: number | null) => void)[] = [];

/** The session page navigates to the review when the session ends. */
export function onTalkEnded(f: (r: SessionReview | null, id: number | null) => void): () => void {
  endListeners.push(f);
  return () => {
    endListeners = endListeners.filter((x) => x !== f);
  };
}

export async function endTalk(reason: "user" | "hibernate" | "quit" = "user"): Promise<SessionReview | null> {
  const id = useTalk.getState().conversationId;
  queue.stop();
  if (useTalk.getState().recording) void api.stopRecording(() => {}).catch(() => {});
  useTalk.setState({ ending: true, recording: false });
  try {
    const review = await api.endVoiceSession(reason);
    endListeners.forEach((f) => f(review, id));
    return review;
  } catch (e) {
    toastError(e);
    return null;
  } finally {
    useTalk.setState({ ...initial() });
  }
}

export function setLevel(rms: number): void {
  useTalk.setState({ level: rms });
}

export function stopSpeaking(): void {
  queue.stop();
}
