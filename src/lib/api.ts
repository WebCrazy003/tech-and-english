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
}
export interface FeedInput {
  id?: number;
  kind?: "rss" | "hn";
  name: string;
  url: string;
  sourceWeight?: number;
  enabled?: boolean;
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
}

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
  article: ArticleListItem;
  why: string;
}

export type InteractionKind = "opened" | "read" | "saved" | "liked" | "not_interested";

export interface ArticleFilter {
  topicId?: number;
  feedId?: number;
  unreadOnly?: boolean;
  savedOnly?: boolean;
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

  refreshNow: () => call<{ newCount: number }>("refresh_now"),
  listArticles: (filter: ArticleFilter, cursor?: string | null, limit?: number) =>
    call<Page<ArticleListItem>>("list_articles", { filter, cursor: cursor ?? null, limit: limit ?? 50 }),
  getArticle: (id: number) => call<ArticleListItem>("get_article", { id }),
  recordInteraction: (articleId: number, kind: InteractionKind) =>
    call<void>("record_interaction", { articleId, kind }),
  setSaved: (articleId: number, saved: boolean) => call<void>("set_saved", { articleId, saved }),
  openArticle: (articleId: number) => call<void>("open_article", { articleId }),
  getTodayPick: () => call<DailyPick | null>("get_today_pick"),
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
} as const;

export function onEvent<T>(name: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(name, (e) => handler(e.payload));
}
