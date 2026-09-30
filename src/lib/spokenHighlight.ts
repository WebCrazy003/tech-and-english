// Read-along highlighting: marks the sentence and the word being spoken, in the text on screen.
// Uses the CSS Custom Highlight API (the DOM is not changed) and the `boundary` events of
// speechSynthesis, which give the position of each spoken word.

const SENTENCE = "speaking-sentence";
const WORD = "speaking-word";
const BLOCKS = "p,li,h1,h2,h3,h4,h5,h6,blockquote,td,th,figcaption,dd,dt";
/** Not read aloud: code blocks and things that are not text. */
const SKIP = "pre,script,style,svg,button,select,textarea";

/** The text of one block, with the text nodes it came from. */
export interface TextMap {
  text: string;
  nodes: { node: Text; start: number }[];
}

export interface SpokenSentence {
  /** Exactly as in the block's text, so a spoken char index maps back to the DOM. */
  text: string;
  /** Offset of `text` in `map.text`. */
  start: number;
  map: TextMap;
}

/** The text under `root`, grouped by block (paragraph, list item, heading…), in reading order. */
export function textMaps(root: HTMLElement): TextMap[] {
  const maps: TextMap[] = [];
  let current: { block: Element; map: TextMap } | null = null;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode() as Text | null; n; n = walker.nextNode() as Text | null) {
    const parent = n.parentElement;
    if (!parent || parent.closest(SKIP)) continue;
    const found = parent.closest(BLOCKS);
    const block = found && root.contains(found) ? found : root;
    if (!current || current.block !== block) {
      current = { block, map: { text: "", nodes: [] } };
      maps.push(current.map);
    }
    current.map.nodes.push({ node: n, start: current.map.text.length });
    current.map.text += n.data;
  }
  return maps.filter((m) => /[\p{L}\p{N}]/u.test(m.text));
}

function split(text: string): { start: number; end: number }[] {
  const seg = typeof Intl !== "undefined" && "Segmenter" in Intl ? new Intl.Segmenter("en", { granularity: "sentence" }) : null;
  if (!seg) return [{ start: 0, end: text.length }];
  return Array.from(seg.segment(text), (s) => ({ start: s.index, end: s.index + s.segment.length }));
}

/** Sentences to speak, each tied to its place in the DOM. */
export function sentencesOf(root: HTMLElement): SpokenSentence[] {
  const out: SpokenSentence[] = [];
  for (const map of textMaps(root)) {
    for (const { start, end } of split(map.text)) {
      const raw = map.text.slice(start, end);
      const text = raw.trim();
      if (!/[\p{L}\p{N}]/u.test(text)) continue;
      out.push({ text, start: start + raw.indexOf(text), map });
    }
  }
  return out;
}

/** DOM range for the characters [start, end) of a block's text. */
export function rangeAt(map: TextMap, start: number, end: number): Range | null {
  const locate = (offset: number, atEnd: boolean) => {
    for (let i = map.nodes.length - 1; i >= 0; i--) {
      const { node, start: s } = map.nodes[i];
      // An end offset on a node border belongs to the node before it.
      if (offset > s || (!atEnd && offset === s)) return { node, offset: Math.min(offset - s, node.data.length) };
    }
    const first = map.nodes[0];
    return first ? { node: first.node, offset: 0 } : null;
  };
  const a = locate(start, false);
  const b = locate(Math.max(end, start), true);
  if (!a || !b || !a.node.isConnected || !b.node.isConnected) return null;
  const r = document.createRange();
  r.setStart(a.node, a.offset);
  r.setEnd(b.node, b.offset);
  return r;
}

/** The word at `charIndex` of a spoken sentence, without the punctuation around it. */
export function wordSpan(sentence: string, charIndex: number, charLength: number): { start: number; end: number } | null {
  if (charIndex < 0 || charIndex >= sentence.length) return null;
  let end = charLength > 0 ? Math.min(sentence.length, charIndex + charLength) : charIndex;
  if (charLength <= 0) while (end < sentence.length && !/\s/.test(sentence[end])) end++;
  let start = charIndex;
  const edge = /[^\p{L}\p{N}]/u;
  while (start < end && edge.test(sentence[start])) start++;
  while (end > start && edge.test(sentence[end - 1])) end--;
  return end > start ? { start, end } : null;
}

type HighlightCtor = new (...ranges: Range[]) => { priority: number };
interface HighlightRegistry {
  set(name: string, h: unknown): void;
  delete(name: string): void;
}
function api(): { registry: HighlightRegistry; Highlight: HighlightCtor } | null {
  if (typeof CSS === "undefined" || typeof window === "undefined") return null;
  const registry = (CSS as unknown as { highlights?: HighlightRegistry }).highlights;
  const Highlight = (window as unknown as { Highlight?: HighlightCtor }).Highlight;
  return registry && Highlight ? { registry, Highlight } : null;
}

function show(name: string, range: Range | null, priority: number) {
  const a = api();
  if (!a) return;
  if (!range) return a.registry.delete(name);
  const h = new a.Highlight(range);
  h.priority = priority;
  a.registry.set(name, h);
}

/** Scroll only when the sentence is (partly) outside the window. */
function keepVisible(range: Range) {
  const r = range.getBoundingClientRect();
  if (r.height === 0 && r.width === 0) return;
  if (r.top < 90 || r.bottom > window.innerHeight - 90) {
    range.startContainer.parentElement?.scrollIntoView({ block: "center", behavior: "smooth" });
  }
}

export function markSentence(s: SpokenSentence, scroll = true): void {
  const range = rangeAt(s.map, s.start, s.start + s.text.length);
  show(WORD, null, 1);
  show(SENTENCE, range, 0);
  if (range && scroll) keepVisible(range);
}

export function markWord(s: SpokenSentence, charIndex: number, charLength: number): void {
  const w = wordSpan(s.text, charIndex, charLength);
  show(WORD, w ? rangeAt(s.map, s.start + w.start, s.start + w.end) : null, 1);
}

export function clearSpoken(): void {
  show(WORD, null, 1);
  show(SENTENCE, null, 0);
}

/** Where `sentence` is inside `root`'s text (searching from `from`), tied to the DOM. */
export function findSentence(root: HTMLElement, sentence: string, from = 0): SpokenSentence | null {
  const maps = textMaps(root);
  // A short element (a chat bubble) is one block; join them so the search covers all of it.
  const map: TextMap = { text: "", nodes: [] };
  for (const m of maps) {
    if (map.text) map.text += " ";
    for (const n of m.nodes) map.nodes.push({ node: n.node, start: map.text.length + n.start });
    map.text += m.text;
  }
  let at = map.text.indexOf(sentence, from);
  if (at < 0) at = map.text.indexOf(sentence);
  return at < 0 ? null : { text: sentence, start: at, map };
}
