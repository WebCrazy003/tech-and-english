// Dev-only fake backend so the UI can be previewed in a normal browser (no Tauri).
// Active only in `vite dev` AND outside the Tauri webview. Add `?fresh` to start at onboarding,
// `?nolesson` to see the day without a lesson.

import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import defaultTopics from "../../src-tauri/resources/default_topics.json";
import defaultFeeds from "../../src-tauri/resources/default_feeds.json";
import type { Channel } from "@tauri-apps/api/core";
import { createVoiceMock } from "./mockVoice";
import type {
  AddFromExample,
  DictEntry,
  NewVocabItem,
  QuizCard,
  VocabItem,
  ArticleListItem,
  ChatMessage,
  DailyPick,
  ExamplePage,
  Feed,
  FeedCandidate,
  ModelInfo,
  Settings,
  StreamEvent,
  Topic,
} from "../lib/api";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function hoursAgo(h: number): string {
  return new Date(Date.now() - h * 3600000).toISOString().replace(/\.\d{3}Z$/, "Z");
}

function breakdown(total: number) {
  const f = total / 100;
  return {
    topicRelevance: Math.min(1, f + 0.1),
    freshness: Math.min(1, f + 0.05),
    popularity: f * 0.8,
    sourcePreference: 0.5,
    novelty: 1,
    userHistory: 0.5,
    total,
  };
}

function install() {
  const fresh = new URLSearchParams(location.search).has("fresh");
  const noLesson = new URLSearchParams(location.search).has("nolesson");
  (window as unknown as Record<string, unknown>).__MOCKED__ = true;

  let settings: Settings = {
    onboardingDone: !fresh,
    pickTime: "08:00",
    fetchIntervalStandardMin: 20,
    fetchIntervalHibernateMin: 45,
    ingestMaxAgeDays: 7,
    lessonMaxAgeDays: 60,
    hnIncludeNew: false,
    rankingWeights: { topicRelevance: 0.35, freshness: 0.2, popularity: 0.15, sourcePreference: 0.1, novelty: 0.1, userHistory: 0.1 },
    notifyDailyPick: true,
    notifyHighInterest: true,
    notifyThreshold: 85,
    notifyMaxPerDay: 3,
    notifyMinGapMin: 90,
    quietHours: ["22:00", "08:00"],
    widget: { style: "card", alwaysOnTop: true, position: null },
    showDockIcon: false,
    ai: {
      activeModel: null,
      idleTimeoutMin: 10,
      contextSize: 8192,
      englishLevel: 2,
      llmWhy: true,
      llamaServerPath: null,
      customModelPath: null,
    },
    readerAiPanelOpen: true,
    tts: { voiceUri: null, rate: 0.85, wordRate: 0.7, volume: 1, pauseMs: 400 },
    learning: { quizSize: 10, desiredRetention: 0.9, autoPronounce: true },
    voice: { sttModel: null, correction: "high", rate: null, keepTranscriptsDays: 90, vadAutoStop: false },
    debug: { keepAudio: false },
    readingScale: 1,
  };
  let mode: "standard" | "hibernate" = "standard";
  let nextId = 100;
  let topics: Topic[] = fresh
    ? []
    : (defaultTopics as Omit<Topic, "id" | "enabled" | "notify" | "notifyThreshold" | "learn">[]).slice(0, 6).map((t, i) => ({
        ...t,
        id: i + 1,
        enabled: true,
        notify: t.priority === 3,
        notifyThreshold: null,
        learn: t.name === "Data Engineering" && !noLesson,
      }));
  if (!fresh) {
    topics.push({
      id: 7, name: "learn data engineering", keywords: ["data engineering course", "what is data engineering"],
      excludedKeywords: [], priority: 2, enabled: true, notify: false, notifyThreshold: null, learn: false,
    });
  }
  let feeds: Feed[] = fresh
    ? []
    : (defaultFeeds as { kind: "rss" | "hn"; name: string; url: string; sourceWeight: number; learning?: boolean }[])
        .filter((f, i) => i < 8 || f.learning)
        .map((f, i) => ({
          ...f,
          id: i + 1,
          enabled: true,
          learning: f.learning ?? false,
          lastFetchedAt: hoursAgo(0.2),
          lastError: i === 5 ? "HTTP 503" : null,
          consecutiveFailures: i === 5 ? 2 : 0,
        }));

  // title, source, topic, score, HN points, learning score
  const samples: [string, string, string, number, number | null, number][] = [
    ["New local AI model runs twice as fast on Apple Silicon", "Simon Willison", "LLMs", 91, 412, 0],
    ["How we built a multi-agent system with MCP and tool calling", "Latent Space", "AI Agents", 86, 230, 0.5],
    ["DuckDB 2.0 brings a new storage format for faster analytics", "DuckDB Blog", "Data Engineering", 80, 156, 0.35],
    ["Kafka 5.0: simpler streaming pipelines without ZooKeeper", "Confluent Blog", "Data Engineering", 74, null, 0],
    ["Python 3.15 makes the free-threaded build the default", "DEV Community: #python", "Python", 70, 98, 0],
    ["A practical guide to evaluating RAG systems", "Hugging Face Blog", "LLMs", 66, null, 0.5],
    ["Why our data warehouse moved to a lakehouse with Iceberg", "Netflix TechBlog", "Data Engineering", 61, 45, 0],
    ["Fine-tuning small open-weight models on a laptop", "Towards Data Science", "Machine Learning", 55, null, 0.35],
    ["The hidden cost of context windows in production LLM apps", "Interconnects", "LLMs", 52, 77, 0],
    ["Apple announces new Mac mini with M5 chip", "The Verge", "AI", 44, 310, 0],
    ["How to build a data pipeline with Airflow: a step-by-step tutorial", "Dagster Blog", "Data Engineering", 48, null, 1],
    ["Kafka explained: a beginner's guide to streaming", "Confluent Blog", "Data Engineering", 42, null, 0.85],
  ];
  const articles: ArticleListItem[] = fresh
    ? []
    : samples.map(([title, source, topic, score, hn, learning], i) => ({
        id: i + 1,
        url: `https://example.com/story-${i + 1}`,
        title,
        sourceName: source,
        description:
          "A short description of the article from the feed. It explains the main idea in one or two sentences so you can decide if you want to read it.",
        publishedAt: hoursAgo(1 + i * 2.5),
        discoveredAt: hoursAgo(0.5 + i * 2.5),
        primaryTopic: topic,
        topics: [topic],
        score,
        breakdown: breakdown(score),
        hnId: hn ? 1000 + i : null,
        hnPoints: hn,
        hnComments: hn ? Math.round(hn / 3) : null,
        readStatus: i === 3 ? "read" : "unread",
        saved: i === 2,
        hidden: false,
        bodyStatus: i === 5 ? "paywalled" : "none",
        difficulty: null,
        readingMinutes: null,
        learningScore: learning,
      }));
  const today = new Date().toISOString().slice(0, 10);
  const pick: DailyPick | null = fresh
    ? null
    : { date: today, kind: "story", article: articles[0], why: "Matches your topics: LLMs, AI · 412 points on Hacker News" };
  const lesson: DailyPick | null =
    fresh || noLesson
      ? null
      : {
          date: today,
          kind: "lesson",
          article: { ...articles[10], publishedAt: hoursAgo(24 * 9), difficulty: "medium", readingMinutes: 12 },
          why: "Tutorial · Data Engineering · from a learning source",
        };
  if (lesson) articles[10] = lesson.article;

  const find = (id: number) => articles.find((a) => a.id === id)!;
  const bodyHtml =
    "<p>DuckDB is an in-process analytical database. It runs inside your application, so there is no server to manage.</p>" +
    "<p>Many data engineers use it to explore Parquet files on a laptop. They write normal SQL, and DuckDB reads the files directly.</p>" +
    "<h2>Why it matters</h2><p>The new storage format makes files about 30 percent smaller, and many queries run twice as fast. " +
    'Read the <a href="https://duckdb.org/docs">documentation</a> for details.</p>';
  const chats: Record<number, ChatMessage[]> = {};
  let chatId = 1;
  const models: ModelInfo[] = [
    {
      id: "qwen3.5-4b", role: "chat", displayName: "Qwen3.5 4B (Q4_K_M)", file: "q.gguf", sizeBytes: 2740937888,
      license: "Apache-2.0", licenseUrl: "https://huggingface.co/Qwen", recommendedRamGb: 8, default: true,
      downloaded: !new URLSearchParams(location.search).has("nomodel"), active: true, partialBytes: 0, downloading: false,
    },
    {
      id: "gemma-4-e4b", role: "chat", displayName: "Gemma 4 E4B (Q4_K_M)", file: "g.gguf", sizeBytes: 4977171584,
      license: "Apache-2.0", licenseUrl: "https://huggingface.co/google", recommendedRamGb: 12, default: false,
      downloaded: false, active: false, partialBytes: 0, downloading: false,
    },
    {
      id: "whisper-base.en", role: "stt", displayName: "Whisper base.en (speech to text)", file: "ggml-base.en.bin", sizeBytes: 147964211,
      license: "MIT", licenseUrl: "https://github.com/ggml-org/whisper.cpp", recommendedRamGb: 1, default: true,
      downloaded: true, active: true, partialBytes: 0, downloading: false,
    },
  ];
  let aiState: "unloaded" | "loading" | "ready" = "unloaded";
  /** Fake streaming: sends `text` word by word to the channel. */
  const streamTo = (ch: Channel<StreamEvent>, text: string, after?: () => void) => {
    const send = (e: StreamEvent) => ch.onmessage(e);
    if (!models[0].downloaded) {
      send({ kind: "error", code: "no_model", message: "No AI model is downloaded yet." });
      return;
    }
    if (aiState === "unloaded") send({ kind: "loading" });
    const words = text.split(/(?<= )/);
    let i = 0;
    const tick = () => {
      aiState = "ready";
      if (i < words.length) {
        send({ kind: "delta", text: words[i++] });
        setTimeout(tick, 30);
      } else {
        after?.();
        send({ kind: "done", cached: false, modelId: "qwen3.5-4b" });
      }
    };
    setTimeout(tick, aiState === "unloaded" ? 1200 : 300);
  };
  const SUMMARY =
    "This article is about DuckDB, a small database that runs **inside your program**.\n\n" +
    "The new version makes files about 30 percent smaller. Many queries now run twice as fast.\n\n" +
    "**Key words**\nin-process — running inside your program\nParquet — a file format for tables";
  const changed = () => void emit("news://updated", { newCount: 0 });

  // ---- P4: Word Book, dictionary, quizzes
  const DICT: Record<string, DictEntry> = {
    inference: {
      headword: "inference", syllables: "in·fer·ence", pronunciation: "ˈinf(ə)rəns", partOfSpeech: "noun",
      senses: ["a conclusion reached on the basis of evidence and reasoning", "the process of inferring something"],
      examples: ["researchers are entrusted with drawing inferences from the data"], parsed: true,
    },
    database: {
      headword: "database", syllables: "da·ta·base", pronunciation: "ˈdadəˌbās", partOfSpeech: "noun",
      senses: ["a structured set of data held in a computer, especially one that is accessible in various ways"],
      examples: [], parsed: true,
    },
  };
  const key = (s: string) => s.trim().replace(/\s+/g, " ").replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "").toLowerCase();
  const nowIso = () => new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
  const vocabItem = (id: number, text: string, meaning: string | null, extra: Partial<VocabItem> = {}): VocabItem => ({
    id, kind: text.includes(" ") ? "phrase" : "word", text, textKey: key(text), meaningSimple: meaning, meaningB1: null,
    partOfSpeech: "noun", ipa: null, syllables: null, examples: [], collocations: [], notes: null, status: "new",
    dueAt: null, lastReviewedAt: null, lastGrade: null, reviewCount: 0, rememberCount: 0, unsureCount: 0, forgotCount: 0,
    createdAt: hoursAgo(48 + id), updatedAt: hoursAgo(1), ...extra,
  });
  let vocab: VocabItem[] = fresh
    ? []
    : [
        vocabItem(1, "latency", "the delay before data starts to move", { status: "learning", reviewCount: 3, rememberCount: 2, forgotCount: 1, lastGrade: "remember", dueAt: hoursAgo(2), ipa: "ˈlātnsē" }),
        vocabItem(2, "throughput", "how much work a system does in a time", { status: "learning", reviewCount: 2, rememberCount: 1, unsureCount: 1, lastGrade: "unsure", dueAt: hoursAgo(-20) }),
        vocabItem(3, "data lake", "a big store of raw data in many formats", { status: "known", reviewCount: 6, rememberCount: 6, lastGrade: "remember", dueAt: hoursAgo(-24 * 30) }),
        vocabItem(4, "idempotent", null),
        vocabItem(5, "in-process", "running inside your program", { dueAt: null }),
        vocabItem(6, "storage format", "the way data is written to disk"),
      ];
  const contexts: Record<number, { id: number; sentence: string; articleId: number | null; articleTitle: string | null; articleUrl: string | null; conversationId: null; createdAt: string }[]> = {
    1: [{ id: 1, sentence: "Low latency matters for streaming.", articleId: 4, articleTitle: "Kafka 5.0: simpler streaming pipelines without ZooKeeper", articleUrl: null, conversationId: null, createdAt: hoursAgo(50) }],
  };
  let vocabId = 100;
  let quizId = 1;
  const history: { sessionId: number; finishedAt: string; total: number; remember: number; unsure: number; forgot: number; score: number; missed: VocabItem[] }[] = fresh
    ? []
    : [0.6, 0.75, 0.9].map((score, i) => ({ sessionId: 90 + i, finishedAt: hoursAgo(24 * (3 - i)), total: 10, remember: 6, unsure: 2, forgot: 2, score, missed: [] }));
  const sessions: Record<number, { cards: QuizCard[]; grades: Record<number, string> }> = {};
  const vocabChanged = () => void emit("vocab://changed", { itemId: null });
  const dueCount = () => ({
    due: vocab.filter((v) => v.reviewCount > 0 && v.dueAt && v.dueAt <= nowIso() && (v.meaningSimple || v.meaningB1)).length,
    new: vocab.filter((v) => v.reviewCount === 0 && (v.meaningSimple || v.meaningB1)).length,
    total: vocab.length,
    ready: vocab.filter((v) => v.meaningSimple || v.meaningB1).length,
  });
  const hib = () => {
    if (mode === "hibernate") throw { code: "hibernating", message: "unavailable in hibernate mode" };
  };

  const voice = createVoiceMock(
    () => settings,
    (id) => articles.find((a) => a.id === id)?.title ?? null,
  );

  mockIPC(
    (cmd, raw) => {
      const p = (raw ?? {}) as Record<string, never>;
      const v = voice(cmd, p);
      if (v.handled) return v.value;
      switch (cmd) {
        case "get_settings":
          return settings;
        case "update_settings":
          settings = { ...settings, ...(p.patch as object), widget: { ...settings.widget, ...((p.patch as Partial<Settings>).widget ?? {}) } };
          return settings;
        case "get_mode":
          return mode;
        case "set_mode":
          mode = p.mode;
          void emit("mode://changed", mode);
          return mode;
        case "get_autostart":
        case "set_autostart":
          return p.enabled ?? true;
        case "list_topics":
          return topics;
        case "upsert_topic": {
          const t = p.topic as Topic;
          if (t.id) topics = topics.map((x) => (x.id === t.id ? t : x));
          else topics = [...topics, { ...t, id: nextId++ }];
          return topics.find((x) => x.name === t.name);
        }
        case "delete_topic":
          topics = topics.filter((t) => t.id !== p.id);
          return null;
        case "preview_topic_matches":
          // Keyword phrases like "data engineering course" match nothing in real news.
          if ((p.keywords as string[]).every((k) => k.split(" ").length >= 3)) return { matched: 0, total: 240 };
          return { matched: Math.min(articles.length, (p.keywords as string[]).length * 3), total: 240 };
        case "list_feeds":
          return feeds;
        case "upsert_feed": {
          const f = p.feed as Feed;
          if (f.id) feeds = feeds.map((x) => (x.id === f.id ? { ...x, ...f } : x));
          else feeds = [...feeds, { ...f, id: nextId++, lastFetchedAt: null, lastError: null, consecutiveFailures: 0 }];
          return feeds.find((x) => x.url === f.url);
        }
        case "delete_feed":
          feeds = feeds.filter((f) => f.id !== p.id);
          return null;
        case "test_feed":
          if (!(p.url as string).includes("feed")) throw { code: "invalid", message: "Not a feed (this is a web page)" };
          return { title: "Example Feed", itemCount: 20, newestPublishedAt: hoursAgo(3) };
        case "refresh_now":
          return { newCount: 3 };
        case "list_articles": {
          const f = (p.filter ?? {}) as {
            query?: string;
            savedOnly?: boolean;
            unreadOnly?: boolean;
            learningOnly?: boolean;
            topicId?: number;
          };
          let items = articles.filter((a) => !a.hidden);
          if (f.learningOnly) items = items.filter((a) => (a.learningScore ?? 0) >= 0.5);
          if (f.query) items = items.filter((a) => a.title.toLowerCase().includes(f.query!.toLowerCase()));
          if (f.savedOnly) items = items.filter((a) => a.saved);
          if (f.unreadOnly) items = items.filter((a) => a.readStatus === "unread");
          return { items: items.slice(0, (p.limit as number) ?? 50), nextCursor: null };
        }
        case "get_article":
          return find(p.id);
        case "record_interaction": {
          const a = find(p.articleId);
          if (p.kind === "not_interested") a.hidden = true;
          if (p.kind === "opened" && a.readStatus === "unread") a.readStatus = "opened";
          changed();
          return null;
        }
        case "set_saved":
          find(p.articleId).saved = p.saved;
          changed();
          return null;
        case "open_article":
          find(p.articleId).readStatus = "opened";
          changed();
          return null;
        case "get_today_pick":
          return pick && !pick.article.hidden ? pick : null;
        case "get_today_lesson":
          return lesson && !lesson.article.hidden ? lesson : null;
        case "discover_feeds": {
          const url = String(p.url);
          if (!/^https?:\/\//.test(url)) throw { code: "invalid", message: "That is not a valid URL" };
          const host = new URL(url).hostname.replace(/^www\./, "");
          const page: ExamplePage = {
            url,
            title: url.includes("startdataengineering") ? "Master Data Engineering: Always Be in Demand" : "Do You Actually Need Real-Time Data?",
            description: "A friendly explanation for data engineers.",
            publishedAt: hoursAgo(24 * 5),
            siteName: url.includes("startdataengineering") ? "Start Data Engineering" : host,
            isFeed: false,
            canSave: true,
          };
          const candidates: FeedCandidate[] = url.includes("startdataengineering")
            ? []
            : [
                { url: `https://${host}/feed`, title: `${host} newsletter`, itemCount: 20, newestPublishedAt: hoursAgo(30), alreadyAdded: url.includes("duckdb") },
                ...(url.includes("two")
                  ? [{ url: `https://${host}/comments/rss.xml`, title: "Short notes", itemCount: 8, newestPublishedAt: hoursAgo(200), alreadyAdded: false }]
                  : []),
              ];
          return new Promise((r) => setTimeout(() => r({ page, candidates }), 900));
        }
        case "add_feed_from_example": {
          const inp = p.input as AddFromExample;
          let feed: Feed | null = null;
          if (inp.feedUrl) {
            feed = { kind: "rss", name: inp.name, url: inp.feedUrl, sourceWeight: 0.6, enabled: true, learning: inp.learning,
              id: nextId++, lastFetchedAt: null, lastError: null, consecutiveFailures: 0 };
            feeds = [...feeds, feed];
          }
          let article: ArticleListItem | null = null;
          if (inp.saveArticle) {
            article = { ...articles[0], id: nextId++, url: inp.url, title: "Do You Actually Need Real-Time Data?", sourceName: "Example site",
              saved: true, score: null, breakdown: null, hnId: null, hnPoints: null, hnComments: null, learningScore: 0.5, bodyStatus: "none" };
            articles.push(article);
          }
          changed();
          return { feed, article };
        }
        case "get_pick_preview":
          return articles.find((a) => !a.hidden) ?? null;
        case "news_status":
          return { articleCount: articles.length, feedCount: feeds.length, feedsWithErrors: feeds.filter((f) => f.lastError).length, lastFetchedAt: hoursAgo(0.2) };
        case "set_widget_style":
          settings = { ...settings, widget: { ...settings.widget, ...(p.args as object) } };
          void emit("settings://changed", settings);
          return settings.widget;
        case "show_main":
          console.info("[mock] show_main", p.route);
          return null;
        case "take_pending_route":
          return null;
        case "quit_app":
          return null;
        case "get_reader_article": {
          const a = find(p.id);
          return { ...a, bodyHtml: a.bodyStatus === "ok" ? bodyHtml : null };
        }
        case "fetch_article_html":
          return {
            html: `<html><body><article><h1>x</h1>${bodyHtml.repeat(8)}</article></body></html>`,
            finalUrl: find(p.articleId).url,
            canonicalUrl: null,
            paywallHint: false,
          };
        case "save_article_body": {
          const a = find((p.input as { articleId: number }).articleId);
          if (a.bodyStatus === "none") Object.assign(a, { bodyStatus: "ok", difficulty: "medium", readingMinutes: 4 });
          return a;
        }
        case "ai_overview":
          return {
            status: { state: aiState, modelId: aiState === "unloaded" ? null : "qwen3.5-4b", message: null },
            enginePath: "/Users/me/llama-server",
            models,
            availableMemoryGb: 7.8,
          };
        case "get_cached_derivative":
          return p.kind === "summary_b1" ? SUMMARY : null;
        case "dictionary_lookup":
          return new Promise((r) => setTimeout(() => r(DICT[key(p.term as string)] ?? null), 120));
        case "define_term":
          if (mode === "hibernate") throw { code: "hibernating", message: "unavailable in hibernate mode" };
          return new Promise((r) =>
            setTimeout(
              () =>
                r({
                  meaningSimple: `the meaning of "${p.term}" in this story`,
                  meaningB1: `In this article, "${p.term}" is used about how DuckDB works inside your program.`,
                  partOfSpeech: String(p.term).includes(" ") ? "phrase" : "noun",
                  ipa: null,
                  syllables: "IN·fer·ence",
                  examples: [`DuckDB uses ${p.term} to read files quickly.`, `We talked about ${p.term} at work.`],
                  collocations: [`fast ${p.term}`],
                }),
              900,
            ),
          );
        case "add_vocab_item": {
          hib();
          const it = p.item as NewVocabItem;
          const k = key(it.text);
          let item = vocab.find((v) => v.kind === it.kind && v.textKey === k);
          const outcome = item ? "merged" : "created";
          if (!item) {
            item = vocabItem(vocabId++, it.text.trim(), it.meaningSimple ?? null, {
              kind: it.kind, meaningB1: it.meaningB1 ?? null, partOfSpeech: it.partOfSpeech ?? null, ipa: it.ipa ?? null,
              examples: it.examples ?? [], collocations: it.collocations ?? [], createdAt: nowIso(),
            });
            vocab = [item, ...vocab];
          } else if (!item.meaningSimple && it.meaningSimple) item.meaningSimple = it.meaningSimple;
          if (it.context?.sentence) {
            (contexts[item.id] ??= []).unshift({
              id: vocabId++, sentence: it.context.sentence, articleId: it.context.articleId ?? null,
              articleTitle: it.context.articleId ? find(it.context.articleId).title : null, articleUrl: null, conversationId: null, createdAt: nowIso(),
            });
          }
          vocabChanged();
          return { outcome, item };
        }
        case "update_vocab_item": {
          hib();
          const item = vocab.find((v) => v.id === p.id)!;
          Object.assign(item, p.patch as object);
          item.textKey = key(item.text);
          vocabChanged();
          return item;
        }
        case "delete_vocab_item":
          hib();
          vocab = vocab.filter((v) => v.id !== p.id);
          vocabChanged();
          return null;
        case "list_vocab": {
          const f = (p.filter ?? {}) as { query?: string; kind?: string; status?: string; dueOnly?: boolean; pendingOnly?: boolean };
          let items = vocab;
          if (f.query) items = items.filter((v) => `${v.text} ${v.meaningSimple ?? ""}`.toLowerCase().includes(f.query!.toLowerCase()));
          if (f.kind) items = items.filter((v) => v.kind === f.kind);
          if (f.status) items = items.filter((v) => v.status === f.status);
          if (f.dueOnly) items = items.filter((v) => v.dueAt && v.dueAt <= nowIso());
          if (f.pendingOnly) items = items.filter((v) => !v.meaningSimple && !v.meaningB1);
          return { items, nextCursor: null, total: items.length };
        }
        case "get_vocab_item": {
          const item = vocab.find((v) => v.id === p.id)!;
          const reviews = item.reviewCount
            ? [{ grade: item.lastGrade, reviewedAt: hoursAgo(26), dueAfter: item.dueAt }, { grade: "remember", reviewedAt: hoursAgo(100), dueAfter: hoursAgo(60) }]
            : [];
          return { item, contexts: contexts[item.id] ?? [], reviews };
        }
        case "list_vocab_keys":
          return vocab.map((v) => v.textKey);
        case "due_count":
          return dueCount();
        case "export_vocab_csv":
          hib();
          return { path: `/Users/me/Downloads/tech-english-wordbook-${today}.csv` };
        case "start_quiz": {
          hib();
          const pool = vocab.filter((v) => v.meaningSimple || v.meaningB1);
          const ids = (p.itemIds as number[] | null) ?? pool.slice(0, (p.size as number) ?? 10).map((v) => v.id);
          const cards: QuizCard[] = ids.map((id) => {
            const v = vocab.find((x) => x.id === id)!;
            return {
              itemId: v.id, kind: v.kind, text: v.text, prompt: v.text, original: null,
              answer: { meaning: v.meaningSimple ?? v.meaningB1 ?? "", example: v.examples[0] ?? null, partOfSpeech: v.partOfSpeech, ipa: v.ipa, context: contexts[v.id]?.[0]?.sentence ?? null },
            };
          });
          const sessionId = quizId++;
          sessions[sessionId] = { cards, grades: {} };
          return { sessionId, cards };
        }
        case "grade_quiz_item": {
          hib();
          sessions[p.sessionId as number].grades[p.itemId as number] = p.grade as string;
          const v = vocab.find((x) => x.id === p.itemId)!;
          v.reviewCount++;
          v.lastGrade = p.grade;
          v.status = v.status === "new" ? "learning" : v.status;
          v.dueAt = new Date(Date.now() + (p.grade === "forgot" ? 600_000 : 86_400_000 * 3)).toISOString();
          vocabChanged();
          return { nextDueAt: v.dueAt, status: v.status };
        }
        case "finish_quiz": {
          const s = sessions[p.sessionId as number];
          const g = Object.values(s.grades);
          const count = (x: string) => g.filter((y) => y === x).length;
          const [remember, unsure, forgot] = [count("remember"), count("unsure"), count("forgot")];
          const r = {
            sessionId: p.sessionId as number, finishedAt: nowIso(), total: s.cards.length, remember, unsure, forgot,
            score: g.length ? (remember + 0.5 * unsure) / g.length : 0,
            missed: vocab.filter((v) => ["forgot", "unsure"].includes(s.grades[v.id])),
          };
          history.push({ ...r, missed: [] });
          return r;
        }
        case "list_quiz_history":
          return history.slice(-((p.limit as number) ?? 5)).reverse();
        case "get_derivative":
          streamTo(
            p.channel as Channel<StreamEvent>,
            p.kind === "summary_b1"
              ? SUMMARY
              : "DuckDB is a database. It runs inside your app (in-process means inside your program). Files are smaller now. Queries are faster.",
          );
          return { jobId: 1 };
        case "list_article_chat":
          return chats[p.articleId] ?? [];
        case "send_article_chat": {
          const list = (chats[p.articleId] ??= []);
          const actionText: Record<string, string> = {
            summarize: "Summarize this article.",
            key_words: "List 5 important technical words from this article.",
            explain_simply: "Explain the main idea of this article in very simple English.",
          };
          const text = (p.action ? actionText[p.action as string] : p.text) as string | null;
          const um = p.retry ? null : { id: chatId++, role: "user" as const, content: text ?? "", createdAt: new Date().toISOString() };
          if (um) list.push(um);
          const answer =
            p.action === "summarize"
              ? SUMMARY
              : "The files are smaller because the **new storage format** compresses columns better. Less data is read from disk, so queries are faster.";
          streamTo(p.channel as Channel<StreamEvent>, answer, () =>
            list.push({ id: chatId++, role: "assistant", content: answer, createdAt: new Date().toISOString() }),
          );
          return { jobId: 2, userMessage: um };
        }
        case "clear_article_chat":
          chats[p.articleId] = [];
          return null;
        case "cancel_job":
        case "start_ai":
        case "unload_ai":
        case "set_active_model":
        case "delete_model":
        case "cancel_download":
          return null;
        case "download_model":
          models[0].downloaded = true;
          setTimeout(() => void emit("ai://download", { modelId: p.modelId, done: true }), 800);
          return null;
        case "about_info":
          return {
            version: "1.0.0", commit: "abc1234", buildDate: "2026-09-30", system: "macOS 14.7 · Apple M1 · 16 GB RAM",
            models: models.filter((m) => m.downloaded).map((m) => ({ name: m.displayName, license: m.license, licenseUrl: m.licenseUrl })),
          };
        case "third_party_licenses":
          return `Third-party ${p.kind} (mock)\n\nMIT License\n\nPermission is hereby granted…`;
        case "check_engines":
          return new Promise((r) =>
            setTimeout(
              () => r({ llama: { path: "/Applications/Tech English.app/Contents/MacOS/llama-server", ok: true }, whisper: { path: "/Applications/Tech English.app/Contents/MacOS/whisper-server", ok: !new URLSearchParams(location.search).has("noengine") }, freeDiskGb: 658 }),
              800,
            ),
          );
        case "get_diagnostics":
          return "Tech English 1.0.0 (mock)\nmode: standard";
        case "copy_text":
          return null;
        case "take_crash_notice":
          return new URLSearchParams(location.search).has("crash") ? { file: "crash-1790000000.txt", text: "Tech English 1.0.0\npanic: boom" } : null;
        case "get_onboarding_defaults":
          return { topics: defaultTopics, feeds: defaultFeeds };
        case "complete_onboarding": {
          settings = { ...settings, onboardingDone: true };
          setTimeout(() => void emit("news://updated", { newCount: 42 }), 1500);
          return null;
        }
        default:
          if (cmd.startsWith("plugin:notification")) return "granted";
          console.warn("[mock] unhandled command", cmd, raw);
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  console.info("[mock] fake backend installed", { fresh });
}

if (import.meta.env.DEV && !inTauri) install();
