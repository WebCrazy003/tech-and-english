// Read-along for the tutor's bubbles: the sentence and word being spoken are highlighted.

import { clearSpoken, findSentence, markSentence, markWord, type SpokenSentence } from "../../lib/spokenHighlight";

let current: SpokenSentence | null = null;
/** How far each bubble has been spoken, so a repeated sentence is found at the right place. */
const cursors = new WeakMap<Element, number>();

/** The bubble of `turn`; a replay (no bubble of its own) uses the last tutor bubble. */
function bubble(turn: number): HTMLElement | null {
  const own = document.querySelector<HTMLElement>(`[data-turn="${turn}"]`);
  if (own) return own;
  const all = document.querySelectorAll<HTMLElement>("[data-tutor]");
  return all.length ? all[all.length - 1] : null;
}

/** The sentence waiting for its bubble: speech can start just before React draws the text. */
let pending: { turn: number; text: string } | null = null;

function resolve(): boolean {
  if (!pending) return false;
  const el = bubble(pending.turn);
  const found = el ? findSentence(el, pending.text, cursors.get(el) ?? 0) : null;
  if (!el || !found) return false;
  current = found;
  cursors.set(el, found.start + pending.text.length);
  pending = null;
  markSentence(found, false);
  return true;
}

export function bubbleSentence(turn: number, text: string): void {
  current = null;
  pending = { turn, text };
  if (resolve()) return;
  clearSpoken();
  setTimeout(() => {
    if (pending?.text === text && pending.turn === turn) resolve();
  }, 80);
}

export function bubbleWord(charIndex: number, charLength: number): void {
  if (!current) resolve();
  if (current) markWord(current, charIndex, charLength);
}

export function bubbleDone(): void {
  current = null;
  pending = null;
  clearSpoken();
}
