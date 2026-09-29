// Small, pure formatting helpers (unit-tested in format.test.ts).

/** "just now", "5 min ago", "3 h ago", "2 days ago". */
export function timeAgo(iso: string | null | undefined, now: Date = new Date()): string {
  if (!iso) return "";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const mins = Math.max(0, Math.round((now.getTime() - then) / 60000));
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins} min ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  return days === 1 ? "1 day ago" : `${days} days ago`;
}

/** Short age for tight spaces: "5m", "3h", "2d". */
export function shortAge(iso: string | null | undefined, now: Date = new Date()): string {
  if (!iso) return "";
  const mins = Math.max(0, Math.round((now.getTime() - new Date(iso).getTime()) / 60000));
  if (mins < 60) return `${Math.max(1, mins)}m`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
}

export function scoreLabel(score: number | null | undefined): string {
  return score == null ? "–" : String(Math.round(score));
}

export function percent(x: number): string {
  return `${Math.round(x * 100)}%`;
}

/** RFC 3339 timestamp `days` before now (for "last N days" filters). */
export function daysAgoIso(days: number, now: Date = new Date()): string {
  return new Date(now.getTime() - days * 86400000).toISOString().replace(/\.\d{3}Z$/, "Z");
}

export const PRIORITY_LABEL: Record<1 | 2 | 3, string> = { 1: "Low", 2: "Normal", 3: "High" };

/** Keywords typed as "a, b; c" → ["a", "b", "c"], trimmed, no duplicates (case-insensitive). */
export function splitKeywords(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of text.split(/[,;\n]/)) {
    const k = raw.trim().replace(/\s+/g, " ");
    if (k && !seen.has(k.toLowerCase())) {
      seen.add(k.toLowerCase());
      out.push(k);
    }
  }
  return out;
}
