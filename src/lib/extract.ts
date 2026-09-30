// Article extraction: Rust fetches the HTML, Readability finds the article, DOMPurify cleans it.
// (P2 dev spec §3.) The result is saved in Rust, which decides the body status.

import { Readability } from "@mozilla/readability";
import DOMPurify from "dompurify";
import { api, type ReaderArticle } from "./api";

const ALLOWED_TAGS = [
  "p", "h2", "h3", "h4", "ul", "ol", "li", "blockquote", "pre", "code", "em", "strong", "b", "i",
  "a", "img", "figure", "figcaption", "table", "thead", "tbody", "tr", "th", "td", "br",
];
const ALLOWED_ATTR = ["href", "src", "alt", "title"];
const MIN_WORDS = 80;

export interface Extracted {
  text: string;
  html: string;
}

/** Run Readability on a page. Returns null when no article can be found. */
export function extractArticle(html: string, baseUrl: string): Extracted | null {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const base = doc.createElement("base");
  base.href = baseUrl;
  doc.head.prepend(base);
  const result = new Readability(doc, { charThreshold: 300 }).parse();
  if (!result?.content) return null;
  const clean = DOMPurify.sanitize(result.content, { ALLOWED_TAGS, ALLOWED_ATTR });
  const text = blocksToText(clean);
  // Under 80 words is not a real article (link lists, cookie walls…) — same rule as the backend.
  return text.split(/\s+/).length >= MIN_WORDS ? { text, html: clean } : null;
}

const BLOCKS = "p, li, h2, h3, h4, blockquote, pre, figcaption, td";

/** Plain text with one paragraph per block, separated by blank lines (the AI uses paragraphs). */
export function blocksToText(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const blocks = Array.from(doc.body.querySelectorAll(BLOCKS)).filter(
    (el) => !el.parentElement?.closest(BLOCKS),
  );
  const paras = (blocks.length ? blocks : [doc.body])
    .map((el) => (el.textContent ?? "").replace(/\s+/g, " ").trim())
    .filter(Boolean);
  return paras.join("\n\n");
}

const inflight = new Map<number, Promise<ReaderArticle>>();

/** Make sure the article body is extracted and saved (once, even if called twice). */
export function ensureBody(articleId: number): Promise<ReaderArticle> {
  const running = inflight.get(articleId);
  if (running) return running;
  const job = (async () => {
    const current = await api.getReaderArticle(articleId);
    if (current.bodyStatus !== "none") return current;
    try {
      const page = await api.fetchArticleHtml(articleId);
      const found = extractArticle(page.html, page.finalUrl);
      await api.saveArticleBody({
        articleId,
        text: found?.text ?? null,
        html: found?.html ?? null,
        canonicalUrl: page.canonicalUrl,
        paywallHint: page.paywallHint,
        failed: !found,
      });
    } catch {
      await api.saveArticleBody({ articleId, failed: true });
    }
    return api.getReaderArticle(articleId);
  })().finally(() => inflight.delete(articleId));
  inflight.set(articleId, job);
  return job;
}
