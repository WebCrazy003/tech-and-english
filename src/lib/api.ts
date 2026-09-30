// The ONLY file that calls invoke() or listen(). Types mirror the Rust serde DTOs (camelCase).

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { openUrl } from "@tauri-apps/plugin-opener";

export type Mode = "standard" | "hibernate";
export type Priority = 1 | 2 | 3;
export type WidgetStyle = "card" | "pill" | "hidden";

export interface RankingWeights {
  topicRelevance: number;
  freshness: number;
  popularity: number;
  sourcePreference: number;
  novelty: number;
  userHistory: number;
}

export interface WidgetSettings {
  style: WidgetStyle;
  alwaysOnTop: boolean;
  position: [number, number] | null;
}

export interface Settings {
  onboardingDone: boolean;
  pickTime: string;
  fetchIntervalStandardMin: number;
  fetchIntervalHibernateMin: number;
  ingestMaxAgeDays: number;
  lessonMaxAgeDays: number;
  hnIncludeNew: boolean;
  rankingWeights: RankingWeights;
  notifyDailyPick: boolean;
  notifyHighInterest: boolean;
  notifyThreshold: number;
  notifyMaxPerDay: number;
  notifyMinGapMin: number;
  quietHours: [string, string] | null;
  widget: WidgetSettings;
  showDockIcon: boolean;
  ai: AiSettings;
  readerAiPanelOpen: boolean;
  tts: TtsSettings;
  learning: LearningSettings;
}

export interface TtsSettings {
  voiceUri: string | null;
  /** Listen speed, 0.5–1.2 */
  rate: number;
  /** Single words (popup, quiz), 0.5–1.2 */
  wordRate: number;
  volume: number;
  pauseMs: number;
}

export interface LearningSettings {
  quizSize: number;
  desiredRetention: number;
  autoPronounce: boolean;
}

// ---------------------------------------------------------------- words (P4)

export type VocabKind = "word" | "phrase" | "sentence" | "term" | "pronunciation" | "correction";
export type VocabStatus = "new" | "learning" | "known";
export type Grade = "forgot" | "unsure" | "remember";

export interface DictEntry {
  headword: string;
  syllables: string | null;
  pronunciation: string | null;
  partOfSpeech: string | null;
  senses: string[];
  examples: string[];
  /** False when the dictionary text had an unusual format (senses[0] is the raw start). */
  parsed: boolean;
}

export interface DefineTermOut {
  meaningSimple: string;
  meaningB1: string;
  partOfSpeech: string;
  ipa: string | null;
  syllables: string | null;
  examples: string[];
  collocations: string[];
}

export interface VocabItem {
  id: number;
  kind: VocabKind;
  text: string;
  textKey: string;
  meaningSimple: string | null;
  meaningB1: string | null;
  partOfSpeech: string | null;
  ipa: string | null;
  syllables: string | null;
  examples: string[];
  collocations: string[];
  notes: string | null;
  status: VocabStatus;
  dueAt: string | null;
  lastReviewedAt: string | null;
  lastGrade: Grade | null;
  reviewCount: number;
  rememberCount: number;
  unsureCount: number;
  forgotCount: number;
  createdAt: string;
  updatedAt: string;
}

export interface VocabContext {
  id: number;
  sentence: string | null;
  articleId: number | null;
  articleTitle: string | null;
  articleUrl: string | null;
  conversationId: number | null;
  createdAt: string;
}

export interface VocabDetail {
  item: VocabItem;
  contexts: VocabContext[];
  reviews: { grade: Grade; reviewedAt: string; dueAfter: string | null }[];
}

export interface NewVocabItem {
  kind: VocabKind;
  text: string;
  meaningSimple?: string | null;
  meaningB1?: string | null;
  partOfSpeech?: string | null;
  ipa?: string | null;
  syllables?: string | null;
  examples?: string[];
  collocations?: string[];
  notes?: string | null;
  context?: { sentence?: string | null; articleId?: number | null; conversationId?: number | null };
}

export interface VocabPatch {
  kind?: VocabKind;
  text?: string;
  meaningSimple?: string;
  meaningB1?: string;
  partOfSpeech?: string;
  ipa?: string;
  examples?: string[];
  notes?: string;
}

export interface VocabFilter {
  query?: string;
  kind?: VocabKind;
  status?: VocabStatus;
  dueOnly?: boolean;
  articleId?: number;
  pendingOnly?: boolean;
}

export interface DueCount {
  due: number;
  new: number;
  total: number;
  /** Items with a meaning; a quiz needs at least 3. */
  ready: number;
}

export interface QuizCard {
  itemId: number;
  kind: VocabKind;
  text: string;
  prompt: string;
  original: string | null;
  answer: { meaning: string; example: string | null; partOfSpeech: string | null; ipa: string | null; context: string | null };
}

export interface QuizResult {
  sessionId: number;
  finishedAt: string | null;
  total: number;
  remember: number;
  unsure: number;
  forgot: number;
  score: number;
  missed: VocabItem[];
}

export interface AiSettings {
  activeModel: string | null;
  idleTimeoutMin: number;
  contextSize: number;
  englishLevel: 1 | 2 | 3;
  llmWhy: boolean;
  llamaServerPath: string | null;
  customModelPath: string | null;
}

export interface AiStatus {
  /** llm = the chat model (llama-server), stt = speech to text (whisper-server). */
  component?: "llm" | "stt";
  state: "unloaded" | "loading" | "ready" | "busy" | "error";
  modelId: string | null;
  message: string | null;
}

export interface ModelInfo {
  id: string;
  role: string;
  displayName: string;
  file: string;
  sizeBytes: number;
  license: string;
  licenseUrl: string;
  recommendedRamGb: number;
  default: boolean;
  downloaded: boolean;
  active: boolean;
  partialBytes: number;
  downloading: boolean;
}

export interface AiOverview {
  status: AiStatus;
  enginePath: string | null;
  models: ModelInfo[];
  availableMemoryGb: number;
}

export type StreamEvent =
  | { kind: "loading" }
  | { kind: "delta"; text: string }
  | { kind: "done"; cached: boolean; modelId: string }
  | { kind: "error"; code: string; message: string };

export type DerivKind = "summary_b1" | "easy_english";
export type QuickAction = "summarize" | "key_words" | "explain_simply";

export interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  content: string;
  createdAt: string;
}

export type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> | T[K] : T[K] };

export interface Topic {
  id: number;
  name: string;
  keywords: string[];
  excludedKeywords: string[];
  priority: Priority;
  enabled: boolean;
  notify: boolean;
  notifyThreshold: number | null;
  /** Also find learning materials (tutorials, explainers) for this topic. */
  learn: boolean;
}
export type TopicInput = Omit<Topic, "id"> & { id?: number };

export interface Feed {
  id: number;
  kind: "rss" | "hn";
  name: string;
  url: string;
  sourceWeight: number;
  enabled: boolean;
  lastFetchedAt: string | null;
  lastError: string | null;
  consecutiveFailures: number;
  /** This source mostly publishes learning material. */
  learning: boolean;
}
export interface FeedInput {
  id?: number;
  kind?: "rss" | "hn";
  name: string;
  url: string;
  sourceWeight?: number;
  enabled?: boolean;
  learning?: boolean;
}

export interface ScoreBreakdown {
  topicRelevance: number;
  freshness: number;
  popularity: number;
  sourcePreference: number;
  novelty: number;
  userHistory: number;
  total: number;
}

export interface ArticleListItem {
  id: number;
  url: string;
  title: string;
  sourceName: string;
  description: string | null;
  publishedAt: string | null;
  discoveredAt: string;
  primaryTopic: string | null;
  topics: string[];
  score: number | null;
  breakdown: ScoreBreakdown | null;
  hnId: number | null;
  hnPoints: number | null;
  hnComments: number | null;
  readStatus: "unread" | "opened" | "read";
  saved: boolean;
  hidden: boolean;
  bodyStatus: BodyStatus;
  difficulty: "easy" | "medium" | "hard" | null;
  readingMinutes: number | null;
  /** 0..1; from 0.5 on it counts as learning material. */
  learningScore: number | null;
}

export const LESSON_THRESHOLD = 0.5;
export const isLearning = (a: ArticleListItem) => (a.learningScore ?? 0) >= LESSON_THRESHOLD;

export type BodyStatus = "none" | "ok" | "failed" | "paywalled";

export interface ReaderArticle extends ArticleListItem {
  bodyHtml: string | null;
}

export interface FetchedHtml {
  html: string;
  finalUrl: string;
  canonicalUrl: string | null;
  paywallHint: boolean;
}

export interface SaveBody {
  articleId: number;
  text?: string | null;
  html?: string | null;
  canonicalUrl?: string | null;
  paywallHint?: boolean;
  failed?: boolean;
}

export interface DailyPick {
  date: string;
  kind: "story" | "lesson";
  article: ArticleListItem;
  why: string;
}

export interface ExamplePage {
  url: string;
  title: string;
  description: string | null;
  publishedAt: string | null;
  siteName: string;
  /** The pasted link is a feed itself. */
  isFeed: boolean;
  /** The page could be read, so it can be saved as an article. */
  canSave: boolean;
}

export interface FeedCandidate {
  url: string;
  title: string | null;
  itemCount: number;
  newestPublishedAt: string | null;
  alreadyAdded: boolean;
}

export interface AddFromExample {
  url: string;
  feedUrl: string | null;
  name: string;
  learning: boolean;
  saveArticle: boolean;
}

export type InteractionKind = "opened" | "read" | "saved" | "liked" | "not_interested";

export interface ArticleFilter {
  topicId?: number;
  feedId?: number;
  unreadOnly?: boolean;
  savedOnly?: boolean;
  learningOnly?: boolean;
  minScore?: number;
  query?: string;
  since?: string;
}

export interface Page<T> {
  items: T[];
  nextCursor: string | null;
}

export interface FeedTestResult {
  title: string | null;
  itemCount: number;
  newestPublishedAt: string | null;
}

export interface TopicSeed {
  name: string;
  priority: Priority;
  keywords: string[];
  excludedKeywords: string[];
}
export interface FeedSeed {
  kind: "rss" | "hn";
  name: string;
  url: string;
  sourceWeight: number;
  group: string;
}

export interface NewsStatus {
  articleCount: number;
  feedCount: number;
  feedsWithErrors: number;
  lastFetchedAt: string | null;
}

export interface OnboardingInput {
  topicNames: string[];
  feedUrls: string[];
  pickTime: string;
  notifyDailyPick: boolean;
  notifyHighInterest: boolean;
  launchAtLogin: boolean;
}

/** Error returned by every command: `{ code, message }`. */
export class ApiError extends Error {
  constructor(
    public code: string,
    message: string,
  ) {
    super(message);
  }
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    if (e && typeof e === "object" && "message" in e) {
      const err = e as { code?: string; message: string };
      throw new ApiError(err.code ?? "unknown", err.message);
    }
    throw new ApiError("unknown", String(e));
  }
}

export const api = {
  getSettings: () => call<Settings>("get_settings"),
  updateSettings: (patch: DeepPartial<Settings>) => call<Settings>("update_settings", { patch }),
  getMode: () => call<Mode>("get_mode"),
  setMode: (mode: Mode) => call<Mode>("set_mode", { mode }),
  getAutostart: () => call<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => call<boolean>("set_autostart", { enabled }),

  listTopics: () => call<Topic[]>("list_topics"),
  upsertTopic: (topic: TopicInput) => call<Topic>("upsert_topic", { topic }),
  deleteTopic: (id: number) => call<void>("delete_topic", { id }),
  previewTopicMatches: (keywords: string[], excludedKeywords: string[]) =>
    call<{ matched: number; total: number }>("preview_topic_matches", { keywords, excludedKeywords }),

  listFeeds: () => call<Feed[]>("list_feeds"),
  upsertFeed: (feed: FeedInput) => call<Feed>("upsert_feed", { feed }),
  deleteFeed: (id: number) => call<void>("delete_feed", { id }),
  testFeed: (url: string) => call<FeedTestResult>("test_feed", { url }),
  discoverFeeds: (url: string) =>
    call<{ page: ExamplePage; candidates: FeedCandidate[] }>("discover_feeds", { url }),
  addFeedFromExample: (input: AddFromExample) =>
    call<{ feed: Feed | null; article: ArticleListItem | null }>("add_feed_from_example", { input }),

  refreshNow: () => call<{ newCount: number }>("refresh_now"),
  listArticles: (filter: ArticleFilter, cursor?: string | null, limit?: number) =>
    call<Page<ArticleListItem>>("list_articles", { filter, cursor: cursor ?? null, limit: limit ?? 50 }),
  getArticle: (id: number) => call<ArticleListItem>("get_article", { id }),
  recordInteraction: (articleId: number, kind: InteractionKind) =>
    call<void>("record_interaction", { articleId, kind }),
  setSaved: (articleId: number, saved: boolean) => call<void>("set_saved", { articleId, saved }),
  openArticle: (articleId: number) => call<void>("open_article", { articleId }),
  getTodayPick: () => call<DailyPick | null>("get_today_pick"),
  getTodayLesson: () => call<DailyPick | null>("get_today_lesson"),
  getPickPreview: () => call<ArticleListItem | null>("get_pick_preview"),
  newsStatus: () => call<NewsStatus>("news_status"),

  getReaderArticle: (id: number) => call<ReaderArticle>("get_reader_article", { id }),
  fetchArticleHtml: (articleId: number) => call<FetchedHtml>("fetch_article_html", { articleId }),
  saveArticleBody: (input: SaveBody) => call<ArticleListItem>("save_article_body", { input }),
  openExternal: (url: string) => openUrl(url),

  aiOverview: () => call<AiOverview>("ai_overview"),
  downloadModel: (modelId: string) => call<void>("download_model", { modelId }),
  cancelDownload: (modelId: string) => call<void>("cancel_download", { modelId }),
  deleteModel: (modelId: string) => call<void>("delete_model", { modelId }),
  setActiveModel: (modelId: string) => call<void>("set_active_model", { modelId }),
  startAi: (force: boolean) => call<AiStatus>("start_ai", { force }),
  unloadAi: () => call<void>("unload_ai"),
  getCachedDerivative: (articleId: number, kind: DerivKind) =>
    call<string | null>("get_cached_derivative", { articleId, kind }),
  getDerivative: (
    articleId: number,
    kind: DerivKind,
    onEvent: (e: StreamEvent) => void,
    opts: { regenerate?: boolean; force?: boolean } = {},
  ) => {
    const channel = new Channel<StreamEvent>();
    channel.onmessage = onEvent;
    return call<{ jobId: number }>("get_derivative", {
      articleId,
      kind,
      regenerate: opts.regenerate ?? false,
      force: opts.force ?? false,
      channel,
    });
  },
  listArticleChat: (articleId: number) => call<ChatMessage[]>("list_article_chat", { articleId }),
  sendArticleChat: (
    articleId: number,
    msg: { text?: string; action?: QuickAction; retry?: boolean; force?: boolean },
    onEvent: (e: StreamEvent) => void,
  ) => {
    const channel = new Channel<StreamEvent>();
    channel.onmessage = onEvent;
    return call<{ jobId: number; userMessage: ChatMessage | null }>("send_article_chat", {
      articleId,
      text: msg.text ?? null,
      action: msg.action ?? null,
      retry: msg.retry ?? false,
      force: msg.force ?? false,
      channel,
    });
  },
  clearArticleChat: (articleId: number) => call<void>("clear_article_chat", { articleId }),
  cancelJob: (jobId: number) => call<void>("cancel_job", { jobId }),

  dictionaryLookup: (term: string) => call<DictEntry | null>("dictionary_lookup", { term }),
  defineTerm: (term: string, sentence: string | null, articleId: number | null, force = false) =>
    call<DefineTermOut>("define_term", { term, sentence, articleId, force }),
  addVocabItem: (item: NewVocabItem) =>
    call<{ outcome: "created" | "merged"; item: VocabItem }>("add_vocab_item", { item }),
  updateVocabItem: (id: number, patch: VocabPatch) => call<VocabItem>("update_vocab_item", { id, patch }),
  deleteVocabItem: (id: number) => call<void>("delete_vocab_item", { id }),
  listVocab: (filter: VocabFilter, cursor?: number | null, limit?: number) =>
    call<{ items: VocabItem[]; nextCursor: number | null; total: number }>("list_vocab", {
      filter,
      cursor: cursor ?? null,
      limit: limit ?? 100,
    }),
  getVocabItem: (id: number) => call<VocabDetail>("get_vocab_item", { id }),
  listVocabKeys: () => call<string[]>("list_vocab_keys"),
  dueCount: () => call<DueCount>("due_count"),
  exportVocabCsv: () => call<{ path: string }>("export_vocab_csv"),
  startQuiz: (size: number | null, itemIds: number[] | null = null) =>
    call<{ sessionId: number; cards: QuizCard[] }>("start_quiz", { size, itemIds }),
  gradeQuizItem: (sessionId: number, itemId: number, grade: Grade) =>
    call<{ nextDueAt: string; status: VocabStatus }>("grade_quiz_item", { sessionId, itemId, grade }),
  finishQuiz: (sessionId: number) => call<QuizResult>("finish_quiz", { sessionId }),
  listQuizHistory: (limit = 5) => call<QuizResult[]>("list_quiz_history", { limit }),

  setWidgetStyle: (args: { style?: WidgetStyle; alwaysOnTop?: boolean }) =>
    call<WidgetSettings>("set_widget_style", { args }),
  showMain: (route?: string) => call<void>("show_main", { route: route ?? null }),
  takePendingRoute: () => call<string | null>("take_pending_route"),
  quitApp: () => call<void>("quit_app"),

  getOnboardingDefaults: () => call<{ topics: TopicSeed[]; feeds: FeedSeed[] }>("get_onboarding_defaults"),
  completeOnboarding: (input: OnboardingInput) => call<void>("complete_onboarding", { input }),
};

/** Ask macOS for notification permission if we don't have it yet. */
export async function ensureNotificationPermission(): Promise<boolean> {
  try {
    if (await isPermissionGranted()) return true;
    return (await requestPermission()) === "granted";
  } catch {
    return false;
  }
}

export const EVENTS = {
  newsUpdated: "news://updated",
  pickChanged: "pick://changed",
  modeChanged: "mode://changed",
  settingsChanged: "settings://changed",
  navigate: "navigate",
  articleBody: "article://body",
  needsBody: "article://needs-body",
  aiStatus: "ai://status",
  aiDownload: "ai://download",
  vocabChanged: "vocab://changed",
} as const;

export function onEvent<T>(name: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(name, (e) => handler(e.payload));
}
