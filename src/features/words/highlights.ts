// Light underline for words already in the Word Book (P4 dev spec §4.1). Uses the CSS Custom
// Highlight API, so the article DOM is not changed. Skipped where the API is missing.

import { useEffect } from "react";
import { useWords } from "../../stores/words";

const NAME = "known-vocab";
const MAX_RANGES = 2000;
const MAX_NGRAM = 4;
const TOKEN = /[\p{L}\p{N}][\p{L}\p{N}'’.-]*[\p{L}\p{N}]|[\p{L}\p{N}]/gu;

interface Tok {
  node: Text;
  start: number;
  end: number;
  lower: string;
}

/** Ranges of `root`'s text whose words or 2–4-word phrases are in `keys`. */
export function findKnown(root: Node, keys: Set<string>, max = MAX_RANGES): Range[] {
  const out: Range[] = [];
  if (!keys.size) return out;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode() as Text | null; n; n = walker.nextNode() as Text | null) {
    const toks: Tok[] = [];
    for (const m of n.data.matchAll(TOKEN)) {
      toks.push({ node: n, start: m.index, end: m.index + m[0].length, lower: m[0].toLowerCase() });
    }
    for (let i = 0; i < toks.length; i++) {
      // Longest match first, so "data lake" wins over "data".
      for (let len = Math.min(MAX_NGRAM, toks.length - i); len >= 1; len--) {
        const phrase = toks
          .slice(i, i + len)
          .map((t) => t.lower)
          .join(" ");
        if (keys.has(phrase)) {
          const r = document.createRange();
          r.setStart(n, toks[i].start);
          r.setEnd(n, toks[i + len - 1].end);
          out.push(r);
          i += len - 1;
          break;
        }
      }
      if (out.length >= max) return out;
    }
  }
  return out;
}

type HighlightCtor = new (...ranges: Range[]) => unknown;
interface HighlightRegistry {
  set(name: string, h: unknown): void;
  delete(name: string): void;
}

/** Underline known words inside `ref` whenever the text (`version`) or the Word Book changes. */
export function useKnownHighlights(ref: React.RefObject<HTMLElement | null>, version: unknown) {
  const keys = useWords((s) => s.keys);
  useEffect(() => {
    const registry = (CSS as unknown as { highlights?: HighlightRegistry }).highlights;
    const Highlight = (window as unknown as { Highlight?: HighlightCtor }).Highlight;
    const el = ref.current;
    if (!registry || !Highlight || !el) return;
    registry.set(NAME, new Highlight(...findKnown(el, keys)));
    return () => registry.delete(NAME);
  }, [ref, keys, version]);
}
