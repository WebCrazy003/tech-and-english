// Plays the tutor's sentences in order as they stream in (P5 dev spec §12). Pressing the mic
// stops speech at once (barge-in) and mutes the rest of that reply.

import { speak, stop as stopSpeech } from "../../lib/tts";

export interface QueueOptions {
  voiceURI: () => string | null;
  pauseMs: () => number;
  onSpeaking: (speaking: boolean) => void;
  /** The first sentence of `turn` started playing. */
  onTurnStart: (turn: number) => void;
}

interface Item {
  turn: number;
  text: string;
  rate: number;
}

export const clampRate = (r: number) => Math.min(1.2, Math.max(0.5, Math.round(r * 100) / 100));

export class SpeechQueue {
  private items: Item[] = [];
  private playing = false;
  /** Sentences of turns up to this one are ignored (interrupted replies). */
  private mutedThrough = 0;
  private started = new Set<number>();

  constructor(private opts: QueueOptions) {}

  push(turn: number, text: string, rate: number): void {
    if (turn <= this.mutedThrough || !text.trim()) return;
    this.items.push({ turn, text, rate: clampRate(rate) });
    void this.run();
  }

  /** Stop now and drop everything queued so far, including the rest of the current reply. */
  stop(): void {
    const maxTurn = Math.max(this.mutedThrough, ...this.items.map((i) => i.turn), this.current ?? 0);
    this.mutedThrough = maxTurn;
    this.items = [];
    stopSpeech();
  }

  /** Speak again (replay), ignoring earlier mutes. */
  replay(sentences: string[], rate: number): void {
    this.stop();
    const turn = this.mutedThrough + 0.5; // after the mute line, before the next real turn
    for (const s of sentences) this.items.push({ turn, text: s, rate: clampRate(rate) });
    void this.run();
  }

  get speaking(): boolean {
    return this.playing;
  }

  private current: number | null = null;

  private async run(): Promise<void> {
    if (this.playing) return;
    this.playing = true;
    this.opts.onSpeaking(true);
    try {
      while (this.items.length) {
        const it = this.items.shift()!;
        this.current = it.turn;
        const first = !this.started.has(it.turn);
        if (first) this.started.add(it.turn);
        await speak(it.text, {
          rate: it.rate,
          voiceURI: this.opts.voiceURI(),
          onStart: first ? () => this.opts.onTurnStart(it.turn) : undefined,
        }).catch(() => {});
        if (this.items.length) await new Promise((r) => setTimeout(r, this.opts.pauseMs()));
      }
    } finally {
      this.current = null;
      this.playing = false;
      this.opts.onSpeaking(false);
    }
  }
}
