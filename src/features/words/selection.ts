// Reading a text selection for the word popup (P4 dev spec §4).

export interface WordSelection {
  term: string;
  /** The sentence around the selection (the saved context). */
  sentence: string;
  /** Viewport position of the selection. */
  rect: { left: number; top: number; bottom: number; width: number };
}

export const MAX_WORDS = 8;
export const MAX_CHARS = 80;
const BLOCKS = "p,li,h1,h2,h3,h4,h5,h6,blockquote,td,th,figcaption,dd,dt,pre";

/** The sentence of `text` that contains character `offset`. */
export function sentenceAt(text: string, offset: number): string {
  const seg = typeof Intl !== "undefined" && "Segmenter" in Intl ? new Intl.Segmenter("en", { granularity: "sentence" }) : null;
  if (!seg) return text.trim().slice(0, 400);
  for (const s of seg.segment(text)) {
    if (offset >= s.index && offset < s.index + s.segment.length) return s.segment.replace(/\s+/g, " ").trim().slice(0, 400);
  }
  return text.replace(/\s+/g, " ").trim().slice(0, 400);
}

/**
 * The current selection inside `container`:
 * - a `WordSelection` for 1–8 words (≤ 80 chars),
 * - "long" for a longer selection (the "Ask AI about this" chip handles it),
 * - null when nothing is selected there.
 */
export function readSelection(container: HTMLElement): WordSelection | "long" | null {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return null;
  const range = sel.getRangeAt(0);
  if (!container.contains(range.commonAncestorContainer)) return null;
  const term = sel.toString().replace(/\s+/g, " ").trim().replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "");
  if (!term) return null;
  const words = term.split(" ").length;
  if (words > MAX_WORDS || term.length > MAX_CHARS) return "long";

  const start = range.startContainer;
  const startEl = start.nodeType === Node.ELEMENT_NODE ? (start as Element) : start.parentElement;
  const block = (startEl?.closest(BLOCKS) as HTMLElement | null) ?? container;
  const before = document.createRange();
  before.setStart(block, 0);
  before.setEnd(range.startContainer, range.startOffset);
  const sentence = sentenceAt(block.textContent ?? "", before.toString().length);

  const r = range.getBoundingClientRect();
  return { term, sentence, rect: { left: r.left, top: r.top, bottom: r.bottom, width: r.width } };
}
