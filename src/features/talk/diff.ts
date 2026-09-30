// Word-level diff for the correction card: which words were removed from what the learner said,
// and which were added in the corrected sentence.

export interface DiffWord {
  text: string;
  changed: boolean;
}

const norm = (w: string) => w.toLowerCase().replace(/[^\p{L}\p{N}']/gu, "");

/** Longest common subsequence of the normalized words. */
function lcs(a: string[], b: string[]): boolean[][] {
  const n = a.length;
  const m = b.length;
  const t: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--) t[i][j] = a[i] === b[j] ? t[i + 1][j + 1] + 1 : Math.max(t[i + 1][j], t[i][j + 1]);
  const keepA = new Array<boolean>(n).fill(false);
  const keepB = new Array<boolean>(m).fill(false);
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      keepA[i++] = true;
      keepB[j++] = true;
    } else if (t[i + 1][j] >= t[i][j + 1]) i++;
    else j++;
  }
  return [keepA, keepB];
}

export function wordDiff(original: string, corrected: string): { original: DiffWord[]; corrected: DiffWord[] } {
  const a = original.split(/\s+/).filter(Boolean);
  const b = corrected.split(/\s+/).filter(Boolean);
  const [keepA, keepB] = lcs(a.map(norm), b.map(norm));
  return {
    original: a.map((text, i) => ({ text, changed: !keepA[i] })),
    corrected: b.map((text, i) => ({ text, changed: !keepB[i] })),
  };
}
