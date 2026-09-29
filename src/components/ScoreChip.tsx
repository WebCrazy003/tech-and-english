import type { ScoreBreakdown } from "../lib/api";
import { percent, scoreLabel } from "../lib/format";
import styles from "./components.module.css";

const ROWS: [keyof ScoreBreakdown, string][] = [
  ["topicRelevance", "Topic match"],
  ["freshness", "Freshness"],
  ["popularity", "Popularity"],
  ["sourcePreference", "Source"],
  ["novelty", "Novelty"],
  ["userHistory", "Your history"],
];

export function Breakdown({ b }: { b: ScoreBreakdown }) {
  return (
    <div className={styles.breakdown}>
      {ROWS.map(([key, label]) => (
        <div key={key} className={styles.bdRow}>
          <span>{label}</span>
          <span className={styles.bdBar}>
            <span style={{ width: percent(b[key]) }} />
          </span>
          <span className={styles.bdVal}>{percent(b[key])}</span>
        </div>
      ))}
    </div>
  );
}

/** Score badge; hover shows how the score was calculated. */
export default function ScoreChip({ score, breakdown }: { score: number | null; breakdown: ScoreBreakdown | null }) {
  const level = score == null ? "" : score >= 75 ? styles.scoreHigh : score >= 50 ? styles.scoreMid : "";
  return (
    <span className={styles.scoreWrap} tabIndex={0} aria-label={`Score ${scoreLabel(score)} of 100`}>
      <span className={`${styles.score} ${level}`}>{scoreLabel(score)}</span>
      {breakdown && (
        <span className={styles.scoreTip} role="tooltip">
          <strong>Why this score</strong>
          <Breakdown b={breakdown} />
        </span>
      )}
    </span>
  );
}
