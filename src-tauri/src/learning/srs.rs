//! Spaced repetition with FSRS-6 default parameters (SPEC §10.1, P4 dev spec §7).

use chrono::{DateTime, Duration, Utc};
use fsrs::{DEFAULT_PARAMETERS, FSRS, MemoryState};
use serde::Deserialize;

use crate::error::{AppError, AppResult};

/// Stability (days) from which an item counts as known.
pub const KNOWN_STABILITY: f64 = 21.0;
/// After Forgot the item comes back after this long, so it can return later the same day.
pub const RELEARN_AFTER: Duration = Duration::minutes(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grade {
    Forgot,
    Unsure,
    Remember,
}

impl Grade {
    pub fn as_str(self) -> &'static str {
        match self {
            Grade::Forgot => "forgot",
            Grade::Unsure => "unsure",
            Grade::Remember => "remember",
        }
    }
    /// Score in a quiz: Forgot 0, Not Sure 0.5, Remember 1.
    pub fn score(self) -> f64 {
        match self {
            Grade::Forgot => 0.0,
            Grade::Unsure => 0.5,
            Grade::Remember => 1.0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SrsState {
    pub stability: Option<f64>,
    pub difficulty: Option<f64>,
    pub last_reviewed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SrsUpdate {
    pub stability: f64,
    pub difficulty: f64,
    pub due_at: DateTime<Utc>,
    /// "learning" (first review), "review" or "relearning" (after Forgot).
    pub srs_state: &'static str,
}

/// Next memory state and due time. Forgot → Again, Not Sure → Hard, Remember → Good.
pub fn review(state: &SrsState, grade: Grade, now: DateTime<Utc>, desired_retention: f64) -> AppResult<SrsUpdate> {
    let fsrs = FSRS::new(&DEFAULT_PARAMETERS).map_err(|e| AppError::Internal(format!("fsrs: {e:?}")))?;
    let memory = match (state.stability, state.difficulty) {
        (Some(s), Some(d)) => Some(MemoryState {
            stability: s as f32,
            difficulty: d as f32,
        }),
        _ => None,
    };
    let days_elapsed = state
        .last_reviewed_at
        .map(|t| (now - t).num_days().max(0) as u32)
        .unwrap_or(0);
    let next = fsrs
        .next_states(memory, desired_retention as f32, days_elapsed)
        .map_err(|e| AppError::Internal(format!("fsrs: {e:?}")))?;
    let chosen = match grade {
        Grade::Forgot => next.again,
        Grade::Unsure => next.hard,
        Grade::Remember => next.good,
    };
    let due_at = match grade {
        Grade::Forgot => now + RELEARN_AFTER,
        _ => {
            let days = (chosen.interval.round() as i64).max(1);
            now + Duration::days(days)
        }
    };
    let srs_state = match (memory, grade) {
        (None, _) => "learning",
        (_, Grade::Forgot) => "relearning",
        _ => "review",
    };
    Ok(SrsUpdate {
        stability: chosen.memory.stability as f64,
        difficulty: chosen.memory.difficulty as f64,
        due_at,
        srs_state,
    })
}

/// Word Book status after a review (SPEC §10.1).
pub fn next_status(current: &str, grade: Grade, stability: f64) -> &'static str {
    match (current, grade) {
        ("known", Grade::Forgot) => "learning",
        ("known", _) => "known",
        (_, Grade::Forgot) => "learning",
        _ if stability >= KNOWN_STABILITY => "known",
        _ => "learning",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::parse_ts;

    fn t0() -> DateTime<Utc> {
        parse_ts("2026-09-30T09:00:00Z").unwrap()
    }

    #[test]
    fn new_item_remembered_is_due_in_a_day_or_more() {
        let u = review(&SrsState::default(), Grade::Remember, t0(), 0.9).unwrap();
        assert!(u.due_at - t0() >= Duration::days(1));
        assert_eq!(u.srs_state, "learning");
        assert_eq!(next_status("new", Grade::Remember, u.stability), "learning");
        // FSRS-6 defaults: Good on a new card → stability w[2].
        assert!((u.stability - 2.3065).abs() < 1e-3, "{}", u.stability);
    }

    #[test]
    fn remembering_again_and_again_grows_the_interval_until_known() {
        let mut s = SrsState::default();
        let mut now = t0();
        let mut status = "new";
        let mut last_gap = Duration::zero();
        for i in 0..12 {
            let u = review(&s, Grade::Remember, now, 0.9).unwrap();
            let gap = u.due_at - now;
            assert!(gap >= last_gap, "review {i}: interval shrank");
            last_gap = gap;
            status = next_status(status, Grade::Remember, u.stability);
            s = SrsState {
                stability: Some(u.stability),
                difficulty: Some(u.difficulty),
                last_reviewed_at: Some(now),
            };
            now = u.due_at;
            if status == "known" {
                assert!(u.stability >= KNOWN_STABILITY);
                break;
            }
        }
        assert_eq!(status, "known");
    }

    #[test]
    fn forgot_on_a_known_item_relearns_in_ten_minutes() {
        let s = SrsState {
            stability: Some(40.0),
            difficulty: Some(5.0),
            last_reviewed_at: Some(t0() - Duration::days(30)),
        };
        let u = review(&s, Grade::Forgot, t0(), 0.9).unwrap();
        assert_eq!(u.due_at, t0() + Duration::minutes(10));
        assert_eq!(u.srs_state, "relearning");
        assert!(u.stability < 40.0);
        assert_eq!(next_status("known", Grade::Forgot, u.stability), "learning");
        assert_eq!(next_status("known", Grade::Unsure, 3.0), "known");
    }

    #[test]
    fn grade_mapping() {
        let s = SrsState {
            stability: Some(5.0),
            difficulty: Some(5.0),
            last_reviewed_at: Some(t0() - Duration::days(5)),
        };
        let hard = review(&s, Grade::Unsure, t0(), 0.9).unwrap();
        let good = review(&s, Grade::Remember, t0(), 0.9).unwrap();
        assert!(
            hard.due_at < good.due_at,
            "Not Sure (Hard) comes back sooner than Remember (Good)"
        );
        assert_eq!(
            (Grade::Forgot.score(), Grade::Unsure.score(), Grade::Remember.score()),
            (0.0, 0.5, 1.0)
        );
        assert_eq!(Grade::Unsure.as_str(), "unsure");
    }
}
