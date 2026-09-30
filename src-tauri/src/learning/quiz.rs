//! Quiz selection (SPEC §10.2). Pure: the random generator is passed in, so tests are deterministic.

use chrono::{DateTime, Duration, Utc};

use crate::clock::parse_ts;
use crate::db::repo::vocab::VocabItem;

pub const MIN_ITEMS: usize = 3;
const MAX_NEW: usize = 3;
const RECENT_FORGOT: Duration = Duration::days(3);

/// Pick `size` item ids from `items` (only items with a meaning are eligible), in shuffled order.
/// Buckets, in order: 1) Forgot in the last 3 days, 2) Not Sure, 3) due (most overdue first),
/// 4) new (oldest first, at most 3), 5) the rest at random. Within a bucket, `need` items are
/// picked at random from its top `2 × need`.
pub fn select(items: &[VocabItem], size: usize, now: DateTime<Utc>, rng: &mut fastrand::Rng) -> Vec<i64> {
    let pool: Vec<&VocabItem> = items.iter().filter(|i| i.has_meaning()).collect();
    let ts = |s: &Option<String>| s.as_deref().and_then(parse_ts);
    let mut chosen: Vec<i64> = Vec::new();

    let take = |mut bucket: Vec<&VocabItem>, limit: usize, chosen: &mut Vec<i64>, rng: &mut fastrand::Rng| {
        bucket.retain(|i| !chosen.contains(&i.id));
        let need = limit.min(size - chosen.len()).min(bucket.len());
        if need == 0 {
            return;
        }
        let mut top: Vec<&VocabItem> = bucket.into_iter().take(2 * need).collect();
        rng.shuffle(&mut top);
        chosen.extend(top.into_iter().take(need).map(|i| i.id));
    };

    let mut b1: Vec<&VocabItem> = pool
        .iter()
        .copied()
        .filter(|i| {
            i.last_grade.as_deref() == Some("forgot")
                && ts(&i.last_reviewed_at).is_some_and(|t| t >= now - RECENT_FORGOT)
        })
        .collect();
    b1.sort_by_key(|i| std::cmp::Reverse(i.last_reviewed_at.clone()));
    take(b1, size, &mut chosen, rng);

    let mut b2: Vec<&VocabItem> = pool
        .iter()
        .copied()
        .filter(|i| i.last_grade.as_deref() == Some("unsure"))
        .collect();
    b2.sort_by_key(|i| i.due_at.clone());
    take(b2, size, &mut chosen, rng);

    let mut b3: Vec<&VocabItem> = pool
        .iter()
        .copied()
        .filter(|i| i.review_count > 0 && ts(&i.due_at).is_some_and(|d| d <= now))
        .collect();
    b3.sort_by_key(|i| i.due_at.clone());
    take(b3, size, &mut chosen, rng);

    let mut b4: Vec<&VocabItem> = pool.iter().copied().filter(|i| i.review_count == 0).collect();
    b4.sort_by_key(|i| (i.created_at.clone(), i.id));
    take(b4, MAX_NEW, &mut chosen, rng);

    let mut b5: Vec<&VocabItem> = pool.clone();
    rng.shuffle(&mut b5);
    take(b5, size, &mut chosen, rng);

    rng.shuffle(&mut chosen);
    chosen
}

/// (remember + 0.5 × unsure) / graded; 0 when nothing was graded.
pub fn score(remember: u32, unsure: u32, forgot: u32) -> f64 {
    let graded = remember + unsure + forgot;
    if graded == 0 {
        0.0
    } else {
        (remember as f64 + 0.5 * unsure as f64) / graded as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        parse_ts("2026-09-30T09:00:00Z").unwrap()
    }

    fn item(id: i64, grade: Option<&str>, reviewed_days_ago: Option<i64>, due_days: Option<i64>) -> VocabItem {
        let at = |d: i64| crate::clock::fmt_ts(now() + Duration::days(d));
        VocabItem {
            id,
            kind: "word".into(),
            text: format!("w{id}"),
            text_key: format!("w{id}"),
            meaning_simple: Some("m".into()),
            meaning_b1: None,
            part_of_speech: None,
            ipa: None,
            syllables: None,
            examples: vec![],
            collocations: vec![],
            notes: None,
            status: if grade.is_some() { "learning" } else { "new" }.into(),
            srs_state: None,
            stability: None,
            difficulty: None,
            due_at: due_days.map(at),
            last_reviewed_at: reviewed_days_ago.map(|d| at(-d)),
            last_grade: grade.map(str::to_string),
            review_count: u32::from(grade.is_some()),
            remember_count: 0,
            unsure_count: 0,
            forgot_count: 0,
            created_at: format!("2026-09-{:02}T00:00:00Z", 1 + id % 28),
            updated_at: String::new(),
        }
    }

    /// 2 recent forgot, 1 old forgot (due), 2 unsure, 3 due, 5 new, 6 known-and-not-due.
    fn items() -> Vec<VocabItem> {
        let mut v = vec![
            item(1, Some("forgot"), Some(1), Some(-1)),
            item(2, Some("forgot"), Some(2), Some(-1)),
            item(3, Some("forgot"), Some(10), Some(-2)),
            item(4, Some("unsure"), Some(3), Some(2)),
            item(5, Some("unsure"), Some(3), Some(4)),
            item(6, Some("remember"), Some(9), Some(-5)),
            item(7, Some("remember"), Some(9), Some(-4)),
            item(8, Some("remember"), Some(9), Some(-3)),
        ];
        v.extend((9..=13).map(|id| item(id, None, None, None)));
        v.extend((14..=19).map(|id| item(id, Some("remember"), Some(5), Some(20))));
        v
    }

    #[test]
    fn buckets_fill_in_order() {
        for seed in 0..20 {
            let mut rng = fastrand::Rng::with_seed(seed);
            let got = select(&items(), 10, now(), &mut rng);
            assert_eq!(got.len(), 10);
            let has = |ids: &[i64]| ids.iter().all(|i| got.contains(i));
            assert!(has(&[1, 2]), "recent forgot first (seed {seed}): {got:?}");
            assert!(has(&[4, 5]), "then unsure");
            assert!(has(&[3, 6, 7, 8]), "then due (the old forgot is due)");
            let new = got.iter().filter(|i| (9..=13).contains(*i)).count();
            assert_eq!(new, 2, "new items fill the last 2 places (max 3)");
            assert!(!got.iter().any(|i| (14..=19).contains(i)));
        }
    }

    #[test]
    fn new_items_come_oldest_first_and_the_rest_fills_up() {
        // 6 new + 6 reviewed (not due), size 10: bucket 4 takes 3 of the 6 oldest new items,
        // bucket 5 fills the other 7 at random.
        let mut v: Vec<VocabItem> = (1..=6).map(|id| item(id, None, None, None)).collect();
        v.extend((7..=12).map(|id| item(id, Some("remember"), Some(2), Some(30))));
        for seed in 0..10 {
            let got = select(&v, 10, now(), &mut fastrand::Rng::with_seed(seed));
            assert_eq!(got.len(), 10);
            assert!(got.iter().filter(|i| **i <= 6).count() >= 3);
        }
        // Only new items and a small quiz: exactly the size, all new.
        let only_new: Vec<VocabItem> = (1..=6).map(|id| item(id, None, None, None)).collect();
        assert_eq!(select(&only_new, 5, now(), &mut fastrand::Rng::with_seed(3)).len(), 5);
    }

    #[test]
    fn items_without_meaning_are_skipped_and_small_pools_use_everything() {
        let mut v = items();
        v.truncate(4);
        v[0].meaning_simple = None;
        let mut rng = fastrand::Rng::with_seed(1);
        let mut got = select(&v, 10, now(), &mut rng);
        got.sort();
        assert_eq!(got, vec![2, 3, 4]);
    }

    #[test]
    fn deterministic_with_a_seed_and_shuffled() {
        let a = select(&items(), 10, now(), &mut fastrand::Rng::with_seed(42));
        let b = select(&items(), 10, now(), &mut fastrand::Rng::with_seed(42));
        assert_eq!(a, b);
        let orders: std::collections::HashSet<Vec<i64>> = (0..10)
            .map(|s| select(&items(), 10, now(), &mut fastrand::Rng::with_seed(s)))
            .collect();
        assert!(orders.len() > 1, "the final order is shuffled");
    }

    #[test]
    fn score_formula() {
        assert!((score(6, 2, 2) - 0.7).abs() < 1e-9);
        assert_eq!(score(0, 0, 0), 0.0);
        assert_eq!(score(3, 0, 0), 1.0);
    }
}
