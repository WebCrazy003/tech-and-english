//! All "now" reads go through [`Clock`] so time-based logic is testable.
//! `Utc::now()` must not be called anywhere else (checked by scripts/check.sh).

use std::sync::Mutex;

use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate, NaiveTime, SecondsFormat, Utc};

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    /// Offset of the user's local time zone at `now()`.
    fn local_offset(&self) -> FixedOffset;

    fn now_local(&self) -> DateTime<FixedOffset> {
        self.now().with_timezone(&self.local_offset())
    }
    fn today_local(&self) -> NaiveDate {
        self.now_local().date_naive()
    }
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
    fn local_offset(&self) -> FixedOffset {
        *Local::now().offset()
    }
}

/// Settable clock for tests.
pub struct FakeClock {
    now: Mutex<DateTime<Utc>>,
    offset: FixedOffset,
}

impl FakeClock {
    pub fn new(now: DateTime<Utc>, offset_hours: i32) -> Self {
        Self {
            now: Mutex::new(now),
            offset: FixedOffset::east_opt(offset_hours * 3600).expect("valid offset"),
        }
    }
    pub fn at(rfc3339: &str) -> Self {
        Self::new(parse_ts(rfc3339).expect("valid timestamp"), 0)
    }
    pub fn set(&self, t: DateTime<Utc>) {
        *self.now.lock().unwrap() = t;
    }
    pub fn advance(&self, d: Duration) {
        let mut n = self.now.lock().unwrap();
        *n += d;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }
    fn local_offset(&self) -> FixedOffset {
        self.offset
    }
}

/// Canonical storage format: RFC 3339, UTC, whole seconds ("2026-09-29T08:00:00Z").
/// Strings in this format sort chronologically, so SQL can compare them directly.
pub fn fmt_ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

/// Parse "HH:MM" (24 h).
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_advances() {
        let c = FakeClock::at("2026-09-29T08:00:00Z");
        c.advance(Duration::minutes(90));
        assert_eq!(fmt_ts(c.now()), "2026-09-29T09:30:00Z");
    }

    #[test]
    fn local_date_uses_offset() {
        let c = FakeClock::new(parse_ts("2026-09-29T23:30:00Z").unwrap(), 9);
        assert_eq!(c.today_local().to_string(), "2026-09-30");
    }

    #[test]
    fn hhmm() {
        assert!(parse_hhmm("08:00").is_some());
        assert!(parse_hhmm("25:00").is_none());
    }
}
