// Dev-only fake voice tutor for browser previews (P5). Scripted, no audio capture.
// `?micdenied` shows the microphone help screen.

import { emit } from "@tauri-apps/api/event";
import type { Channel } from "@tauri-apps/api/core";
import type {
  Conversation,
  ConversationTurn,
  Correction,
  SessionReview,
  SessionSettings,
  Settings,
  VoiceEvent,
} from "../lib/api";

type Args = Record<string, unknown>;
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const nowIso = () => new Date().toISOString().replace(/\.\d{3}Z$/, "Z");

const SPOKEN = [
  "Yesterday I deploy the application.",
  "Yesterday I deployed the application.",
  "What does deployment mean?",
  "How do I say scalability?",
  "stability",
  "scalability",
];

export function createVoiceMock(getSettings: () => Settings, articleTitle: (id: number) => string | null) {
  let active: { id: number; articleId: number | null; articleTitle: string | null; settings: SessionSettings } | null = null;
  let phase: "discuss" | "awaitRepeat" | "drill" = "discuss";
  let target = "";
  let drillWord = "";
  let drillTries = 0;
  let turn = 1;
  let spoken = 0;
  let levelTimer: ReturnType<typeof setInterval> | null = null;
  let convId = 100;
  const conversations: Conversation[] = [
    {
      id: 7,
      articleId: null,
      articleTitle: "Postgres 19 adds faster vacuum",
      settings: { level: 2 },
      startedAt: new Date(Date.now() - 86400000).toISOString(),
      endedAt: new Date(Date.now() - 86000000).toISOString(),
      userSpeakingSeconds: 312,
      reviewStatus: "pending",
      corrections: 2,
    },
  ];
  const turns: Record<number, ConversationTurn[]> = { 7: [] };

  const save = (role: "user" | "tutor", text: string, meta: ConversationTurn["meta"] = null) => {
    if (!active) return 0;
    const list = (turns[active.id] ??= []);
    const t = { id: list.length + 1, seq: list.length + 1, role, text, meta, createdAt: nowIso() };
    list.push(t);
    return t.id;
  };

  async function say(ch: Channel<VoiceEvent>, parts: [string, number | null, number | null][], extra: Partial<Extract<VoiceEvent, { kind: "tutorDone" }>> = {}) {
    const n = turn++;
    for (const [text, rateDelta, rate] of parts) {
      await wait(350);
      ch.onmessage({ kind: "tutorSentence", turn: n, text, rateDelta, rate });
    }
    const text = parts.map((p) => p[0]).join(" ");
    const turnId = save("tutor", text, { correction: extra.correction ?? null, local: extra.local ?? false });
    ch.onmessage({
      kind: "tutorDone",
      turn: n,
      turnId,
      text,
      correction: null,
      phase,
      repeatTarget: phase === "awaitRepeat" ? target : null,
      drill: phase === "drill" ? { word: drillWord, hint: "scal·a·bil·i·ty  /ˌskeɪləˈbɪlɪti/", attempt: drillTries + 1, maxAttempts: 3 } : null,
      local: false,
      ...extra,
    });
  }

  async function reply(ch: Channel<VoiceEvent>, text: string) {
    await wait(700);
    const t = text.toLowerCase();
    if (phase === "awaitRepeat") {
      if (t.includes("deployed")) {
        phase = "discuss";
        await say(ch, [["Good. Let's continue.", null, null]], { local: true });
        await say(ch, [["Great job.", null, null], ["What did the application do for your team?", null, null]]);
      } else {
        await say(ch, [["Almost. Listen again:", null, null], [target, -0.15, null]], { local: true });
      }
      return;
    }
    if (phase === "drill") {
      drillTries++;
      if (t.includes(drillWord)) {
        phase = "discuss";
        await say(ch, [["Good!", null, null]], { local: true });
        await say(ch, [["Scalability matters when many people use your app.", null, null], ["Does your app have many users?", null, null]]);
      } else {
        await say(ch, [[`I heard "${text}". Listen again:`, null, null], [drillWord, null, 0.6]], { local: true });
      }
      return;
    }
    if (/^(please )?(speak|talk) (more )?slow/.test(t)) {
      const rate = Math.max(0.5, (active?.settings.rate ?? 0.85) - 0.1);
      if (active) active.settings.rate = rate;
      ch.onmessage({ kind: "localAction", action: "slower", rate });
      return;
    }
    const how = t.match(/how do i say (.+?)\??$/);
    if (how) {
      phase = "drill";
      drillWord = how[1];
      drillTries = 0;
      await say(ch, [["Listen:", null, null], [drillWord, null, 0.6]], { local: true });
      return;
    }
    if (/\bdeploy\b/.test(t)) {
      const correction: Correction = {
        original: text,
        corrected: "Yesterday I deployed the application.",
        explanation: "Use the past tense because you are talking about yesterday.",
        askRepeat: true,
      };
      phase = "awaitRepeat";
      target = correction.corrected;
      await say(
        ch,
        [
          ["Nice work!", null, null],
          ["We say deployed, because it happened yesterday.", null, null],
          ["Please say: Yesterday I deployed the application.", null, null],
        ],
        { correction },
      );
      return;
    }
    if (t.includes("mean")) {
      await say(ch, [
        ["Deployment means putting software on a server so people can use it.", null, null],
        ["For example, a team deploys a new version of an app every Friday.", null, null],
        ["So, do you use a local AI model at work?", null, null],
      ]);
      return;
    }
    await say(ch, [["That is interesting.", null, null], ["Why do you think so?", null, null]]);
  }

  const review = (id: number): SessionReview => ({
    conversationId: id,
    articleTitle: conversations.find((c) => c.id === id)?.articleTitle ?? null,
    startedAt: conversations.find((c) => c.id === id)?.startedAt ?? nowIso(),
    reviewStatus: conversations.find((c) => c.id === id)?.reviewStatus ?? "pending",
    stats: { speakingMinutes: 2.4, newWords: 2, corrections: 1, drills: 1 },
    source: "llm",
    suggestions: [
      { id: "s1", kind: "word", text: "deployment", meaningSimple: "putting software where people can use it", note: null, preselected: true, observationIds: [1] },
      { id: "s2", kind: "term", text: "scalability", meaningSimple: "how well a system works when it gets bigger", note: null, preselected: true, observationIds: [2] },
      { id: "s3", kind: "word", text: "server", meaningSimple: "a computer that gives data to other computers", note: null, preselected: false, observationIds: [3] },
      { id: "s4", kind: "correction", text: "Yesterday I deployed the application.", meaningSimple: null, note: "Use the past tense for yesterday.", preselected: true, observationIds: [4] },
      { id: "s5", kind: "sentence", text: "It depends on how many people use it.", meaningSimple: null, note: null, preselected: false, observationIds: [5] },
    ],
  });

  return (cmd: string, p: Args): { handled: boolean; value?: unknown } => {
    const ok = (value: unknown = null) => ({ handled: true, value });
    switch (cmd) {
      case "voice_setup": {
        const s = getSettings();
        const level = s.ai.englishLevel;
        return ok({
          settings: { level, rate: s.voice.rate ?? [0.7, 0.85, 1.0][level - 1], pauseMs: s.tts.pauseMs, correction: s.voice.correction, voiceUri: s.tts.voiceUri },
          active,
          sttModel: true,
          sttEngine: "/opt/homebrew/bin/whisper-server",
          mic: new URLSearchParams(location.search).has("micdenied") ? "denied" : "granted",
        });
      }
      case "start_voice_session": {
        const a = p.args as { articleId: number | null; settings: SessionSettings };
        const ch = p.channel as Channel<VoiceEvent>;
        const id = ++convId;
        phase = "discuss";
        void (async () => {
          ch.onmessage({ kind: "loading", component: "llm" });
          await wait(500);
          ch.onmessage({ kind: "loading", component: "stt" });
          if (a.articleId) {
            await wait(400);
            ch.onmessage({ kind: "loading", component: "summary" });
          }
          await wait(700);
          const title = a.articleId ? articleTitle(a.articleId) : null;
          active = { id, articleId: a.articleId, articleTitle: title, settings: { ...a.settings } };
          conversations.unshift({
            id, articleId: a.articleId, articleTitle: title, settings: a.settings, startedAt: nowIso(), endedAt: null,
            userSpeakingSeconds: 0, reviewStatus: "pending", corrections: 0,
          });
          void emit("voice://active", { active: true, conversationId: id });
          ch.onmessage({ kind: "ready", conversationId: id, settings: a.settings });
          await say(ch, [
            ["This story is about small AI models that run on a laptop.", null, null],
            ["They are private and cheap, but slower than cloud models.", null, null],
            ["Do you use AI tools in your work?", null, null],
          ]);
        })();
        return ok(new Promise((r) => setTimeout(() => r(id), 1800)));
      }
      case "get_active_session":
        return ok(active ? { ...active, conversationId: active.id, recording: levelTimer !== null } : null);
      case "start_recording":
        if (new URLSearchParams(location.search).has("micdenied"))
          throw { code: "mic_denied", message: "Tech English can't use the microphone." };
        levelTimer = setInterval(() => void emit("voice://level", { rms: 0.02 + Math.random() * 0.12 }), 50);
        return ok();
      case "stop_recording": {
        if (levelTimer) clearInterval(levelTimer);
        levelTimer = null;
        const ch = p.channel as Channel<VoiceEvent>;
        const text = SPOKEN[spoken++ % SPOKEN.length];
        return ok(
          (async () => {
            await wait(300);
            ch.onmessage({ kind: "transcript", text });
            save("user", text);
            await reply(ch, text);
          })(),
        );
      }
      case "send_text_turn": {
        const ch = p.channel as Channel<VoiceEvent>;
        const text = String(p.text);
        return ok(
          (async () => {
            ch.onmessage({ kind: "transcript", text });
            save("user", text);
            await reply(ch, text);
          })(),
        );
      }
      case "start_drill": {
        const ch = p.channel as Channel<VoiceEvent>;
        phase = "drill";
        drillWord = String(p.word).toLowerCase();
        drillTries = 0;
        return ok(say(ch, [["Listen:", null, null], [drillWord, null, 0.6]], { local: true }));
      }
      case "update_session_settings": {
        if (!active) throw { code: "invalid", message: "No conversation is running" };
        active.settings = { ...active.settings, ...(p.patch as Partial<SessionSettings>) };
        return ok(active.settings);
      }
      case "end_voice_session": {
        const id = active?.id ?? null;
        const c = conversations.find((x) => x.id === id);
        if (c) {
          c.endedAt = nowIso();
          c.userSpeakingSeconds = 144;
          c.corrections = 1;
        }
        active = null;
        void emit("voice://active", { active: false, conversationId: null });
        if (id === null || p.reason !== "user") return ok(null);
        return ok(new Promise((r) => setTimeout(() => r(review(id)), 1200)));
      }
      case "get_session_review":
        return ok(review(Number(p.conversationId)));
      case "apply_session_review": {
        const c = conversations.find((x) => x.id === Number(p.conversationId));
        const n = (p.selected as unknown[]).length;
        if (c) c.reviewStatus = n ? "done" : "skipped";
        return ok(n);
      }
      case "list_conversations":
        return ok(conversations);
      case "get_conversation": {
        const id = Number(p.id);
        return ok({ conversation: conversations.find((c) => c.id === id), turns: turns[id] ?? [] });
      }
      case "delete_conversations":
        return ok(conversations.splice(0).length);
      case "report_latency":
        return ok();
      case "voice_latency":
        return ok({
          total: { count: 6, p50: 2600, p90: 4100 },
          stt: { count: 6, p50: 180, p90: 320 },
          firstToken: { count: 6, p50: 900, p90: 1200 },
          firstSentence: { count: 6, p50: 1500, p90: 3900 },
          llmDone: { count: 6, p50: 6000, p90: 9000 },
          recent: [],
        });
      default:
        return { handled: false };
    }
  };
}
