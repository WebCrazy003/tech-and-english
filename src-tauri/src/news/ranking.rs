//! Rule-based article scoring (SPEC §7.6) and affinity deltas (SPEC §7.7). Pure functions.

use super::model::{InteractionKind, ScoreBreakdown};
use crate::settings::RankingWeights;

#[derive(Debug, Clone, Default)]
pub struct RankInputs {
    pub relevance: f64,
    pub age_hours: f64,
    pub hn_points: Option<i64>,
    pub hn_comments: Option<i64>,
    pub source_weight: f64,
    /// Max title similarity to articles picked/opened in the last 7 days.
    pub max_recent_similarity: f64,
    pub topic_affinity: f64,
    pub source_affinity: f64,
}

pub const FRESHNESS_HALF_LIFE_HOURS: f64 = 24.0;

pub fn freshness(age_hours: f64) -> f64 {
    0.5f64.powf(age_hours.max(0.0) / FRESHNESS_HALF_LIFE_HOURS)
}

pub fn popularity(points: Option<i64>, comments: Option<i64>) -> f64 {
    match points {
        None => 0.4,
        Some(p) => {
            let p = p.max(0) as f64;
            let c = comments.unwrap_or(0).max(0) as f64;
            ((1.0 + p).log10() / 501f64.log10()).min(1.0) * 0.8 + (c / 200.0).min(1.0) * 0.2
        }
    }
}

pub fn score(i: &RankInputs, w: &RankingWeights) -> ScoreBreakdown {
    let mut b = ScoreBreakdown {
        topic_relevance: i.relevance.clamp(0.0, 1.0),
        freshness: freshness(i.age_hours),
        popularity: popularity(i.hn_points, i.hn_comments),
        source_preference: i.source_weight.clamp(0.0, 1.0),
        novelty: (1.0 - i.max_recent_similarity).clamp(0.0, 1.0),
        user_history: 0.5 + 0.5 * (i.topic_affinity + i.source_affinity).clamp(-1.0, 1.0),
        total: 0.0,
    };
    b.total = 100.0
        * (w.topic_relevance * b.topic_relevance
            + w.freshness * b.freshness
            + w.popularity * b.popularity
            + w.source_preference * b.source_preference
            + w.novelty * b.novelty
            + w.user_history * b.user_history);
    b
}

#[derive(Debug, Clone, Default)]
pub struct LessonInputs {
    /// Max relevance over topics with Learn on.
    pub learn_relevance: f64,
    pub learning_score: f64,
    pub source_weight: f64,
    pub hn_points: Option<i64>,
    pub hn_comments: Option<i64>,
    pub topic_affinity: f64,
    pub source_affinity: f64,
}

/// 0..100 (SPEC §7.11). Freshness is left out on purpose: good tutorials stay useful.
pub fn lesson_score(i: &LessonInputs) -> f64 {
    let user_history = 0.5 + 0.5 * (i.topic_affinity + i.source_affinity).clamp(-1.0, 1.0);
    100.0
        * (0.35 * i.learn_relevance.clamp(0.0, 1.0)
            + 0.30 * i.learning_score.clamp(0.0, 1.0)
            + 0.15 * i.source_weight.clamp(0.0, 1.0)
            + 0.10 * popularity(i.hn_points, i.hn_comments)
            + 0.10 * user_history)
}

/// (topic delta, source delta). The topic delta is multiplied by the article's
/// relevance to its primary topic by the caller.
pub fn affinity_deltas(kind: InteractionKind) -> Option<(f64, f64)> {
    use InteractionKind::*;
    match kind {
        Read => Some((0.05, 0.03)),
        Saved | Liked | Discussed => Some((0.10, 0.05)),
        Skipped => Some((-0.02, -0.01)),
        NotInterested => Some((-0.15, -0.08)),
        Opened => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn freshness_half_life() {
        assert!(close(freshness(0.0), 1.0));
        assert!(close(freshness(24.0), 0.5));
        assert!(close(freshness(48.0), 0.25));
        assert!(close(freshness(-5.0), 1.0), "future dates count as fresh");
    }

    #[test]
    fn popularity_bounds() {
        assert!(close(popularity(None, None), 0.4));
        assert!(close(popularity(Some(500), Some(200)), 1.0));
        assert!(close(popularity(Some(0), Some(0)), 0.0));
        assert!(popularity(Some(5000), Some(5000)) <= 1.0);
    }

    #[test]
    fn total_in_range_and_weighted() {
        let w = RankingWeights::default();
        let best = RankInputs {
            relevance: 1.0,
            age_hours: 0.0,
            hn_points: Some(500),
            hn_comments: Some(200),
            source_weight: 1.0,
            max_recent_similarity: 0.0,
            topic_affinity: 1.0,
            source_affinity: 0.0,
        };
        assert!(close(score(&best, &w).total, 100.0));
        let worst = RankInputs {
            relevance: 0.0,
            age_hours: 10_000.0,
            hn_points: Some(0),
            hn_comments: Some(0),
            source_weight: 0.0,
            max_recent_similarity: 1.0,
            topic_affinity: -1.0,
            source_affinity: -1.0,
        };
        assert!(score(&worst, &w).total < 0.01);
        let mid = RankInputs {
            relevance: 0.5,
            age_hours: 24.0,
            source_weight: 0.5,
            ..Default::default()
        };
        let b = score(&mid, &w);
        // .30*.5 + .15*.5 + .15*.4 + .20*.5 + .10*1 + .10*.5
        assert!(close(b.total, 100.0 * (0.15 + 0.075 + 0.06 + 0.10 + 0.10 + 0.05)));
    }

    #[test]
    fn lesson_score_weights() {
        let best = LessonInputs {
            learn_relevance: 1.0,
            learning_score: 1.0,
            source_weight: 1.0,
            hn_points: Some(500),
            hn_comments: Some(200),
            topic_affinity: 1.0,
            source_affinity: 0.0,
        };
        assert!(close(lesson_score(&best), 100.0));
        let mid = LessonInputs {
            learn_relevance: 0.8,
            learning_score: 0.6,
            source_weight: 0.3,
            ..Default::default()
        };
        // .35*.8 + .30*.6 + .15*.3 + .10*.4 + .10*.5
        assert!(close(lesson_score(&mid), 100.0 * (0.28 + 0.18 + 0.045 + 0.04 + 0.05)));
    }

    #[test]
    fn deltas() {
        assert_eq!(affinity_deltas(InteractionKind::NotInterested), Some((-0.15, -0.08)));
        assert_eq!(affinity_deltas(InteractionKind::Opened), None);
    }
}
