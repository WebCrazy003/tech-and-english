// Dev-only fake backend so the UI can be previewed in a normal browser (no Tauri).
// Active only in `vite dev` AND outside the Tauri webview. Add `?fresh` to start at onboarding.

import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import defaultTopics from "../../src-tauri/resources/default_topics.json";
import defaultFeeds from "../../src-tauri/resources/default_feeds.json";
import type { ArticleListItem, DailyPick, Feed, Settings, Topic } from "../lib/api";

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
  (window as unknown as Record<string, unknown>).__MOCKED__ = true;

  let settings: Settings = {
    onboardingDone: !fresh,
    pickTime: "08:00",
    fetchIntervalStandardMin: 20,
    fetchIntervalHibernateMin: 45,
    ingestMaxAgeDays: 7,
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
  };
  let mode: "standard" | "hibernate" = "standard";
  let nextId = 100;
  let topics: Topic[] = fresh
    ? []
    : (defaultTopics as Omit<Topic, "id" | "enabled" | "notify" | "notifyThreshold">[]).slice(0, 6).map((t, i) => ({
        ...t,
        id: i + 1,
        enabled: true,
        notify: t.priority === 3,
        notifyThreshold: null,
      }));
  let feeds: Feed[] = fresh
    ? []
    : (defaultFeeds as { kind: "rss" | "hn"; name: string; url: string; sourceWeight: number }[]).slice(0, 8).map((f, i) => ({
        ...f,
        id: i + 1,
        enabled: true,
        lastFetchedAt: hoursAgo(0.2),
        lastError: i === 5 ? "HTTP 503" : null,
        consecutiveFailures: i === 5 ? 2 : 0,
      }));

  const samples: [string, string, string, number, number | null][] = [
    ["New local AI model runs twice as fast on Apple Silicon", "Simon Willison", "LLMs", 91, 412],
    ["How we built a multi-agent system with MCP and tool calling", "Latent Space", "AI Agents", 86, 230],
    ["DuckDB 2.0 brings a new storage format for faster analytics", "DuckDB Blog", "Data Engineering", 80, 156],
    ["Kafka 5.0: simpler streaming pipelines without ZooKeeper", "Confluent Blog", "Data Engineering", 74, null],
    ["Python 3.15 makes the free-threaded build the default", "DEV Community: #python", "Python", 70, 98],
    ["A practical guide to evaluating RAG systems", "Hugging Face Blog", "LLMs", 66, null],
    ["Why our data warehouse moved to a lakehouse with Iceberg", "Netflix TechBlog", "Data Engineering", 61, 45],
    ["Fine-tuning small open-weight models on a laptop", "Towards Data Science", "Machine Learning", 55, null],
    ["The hidden cost of context windows in production LLM apps", "Interconnects", "LLMs", 52, 77],
    ["Apple announces new Mac mini with M5 chip", "The Verge", "AI", 44, 310],
  ];
  const articles: ArticleListItem[] = fresh
    ? []
    : samples.map(([title, source, topic, score, hn], i) => ({
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
      }));
  const pick: DailyPick | null = fresh
    ? null
    : { date: new Date().toISOString().slice(0, 10), article: articles[0], why: "Matches your topics: LLMs, AI · 412 points on Hacker News" };

  const find = (id: number) => articles.find((a) => a.id === id)!;
  const changed = () => void emit("news://updated", { newCount: 0 });

  mockIPC(
    (cmd, raw) => {
      const p = (raw ?? {}) as Record<string, never>;
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
          const f = (p.filter ?? {}) as { query?: string; savedOnly?: boolean; unreadOnly?: boolean; topicId?: number };
          let items = articles.filter((a) => !a.hidden);
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
