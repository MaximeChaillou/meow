//! Daily quick actions: an action with a time runs once a day, at that time,
//! or as soon as Meow is running after it (Mac asleep, logged in late…).

use crate::clock;

/// A failed or cancelled daily run is tried again after this long.
pub const RETRY_SECS: u64 = 15 * 60;

/// A time of day on the local clock, e.g. 07:00.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DailyTime {
    pub hour: u32,
    pub minute: u32,
}

impl DailyTime {
    pub const fn new(hour: u32, minute: u32) -> Self {
        Self { hour, minute }
    }

    /// "7:00" or "07:30", 24-hour clock.
    pub fn parse(text: &str) -> Option<Self> {
        let (hour, minute) = text.trim().split_once(':')?;
        if minute.len() != 2 {
            return None;
        }
        let (hour, minute) = (hour.parse().ok()?, minute.parse().ok()?);
        (hour < 24 && minute < 60).then_some(Self { hour, minute })
    }

    /// Today's occurrence on the local clock, in seconds since the epoch.
    pub fn today(&self, now: u64) -> Option<u64> {
        clock::same_day_at(now, self.hour, self.minute)
    }

    /// "07:00".
    pub fn label(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }
}

/// Whether a daily action should run now: today's time has passed, it has not
/// run since, and it was not just tried.
pub fn due(slot: u64, now: u64, last_ran: Option<u64>, last_try: Option<u64>) -> bool {
    let ran_since = last_ran.is_some_and(|ran| ran >= slot);
    let tried_recently =
        last_try.is_some_and(|tried| tried >= slot && now.saturating_sub(tried) < RETRY_SECS);
    now >= slot && !ran_since && !tried_recently
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOT: u64 = 1_000_000;

    #[test]
    fn parses_times_of_day() {
        assert_eq!(DailyTime::parse("07:00"), Some(DailyTime::new(7, 0)));
        assert_eq!(DailyTime::parse(" 7:30 "), Some(DailyTime::new(7, 30)));
        for bad in ["24:00", "7:60", "7", "7:5", "seven", "-1:00"] {
            assert_eq!(DailyTime::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn not_due_before_the_time() {
        assert!(!due(SLOT, SLOT - 1, None, None));
    }

    #[test]
    fn due_once_the_time_passed_until_it_ran() {
        assert!(due(SLOT, SLOT, None, None));
        assert!(
            due(SLOT, SLOT + 3 * 3600, Some(SLOT - 86_400), None),
            "late login"
        );
        assert!(
            due(SLOT, SLOT + 10, Some(SLOT - 60), None),
            "ran before 7 doesn't count"
        );
        assert!(!due(SLOT, SLOT + 3600, Some(SLOT + 5), None));
    }

    #[test]
    fn a_failed_run_waits_before_trying_again() {
        let tried = SLOT + 10;
        assert!(!due(SLOT, tried + 60, None, Some(tried)));
        assert!(due(SLOT, tried + RETRY_SECS, None, Some(tried)));
        assert!(
            due(SLOT, SLOT, None, Some(SLOT - 86_400)),
            "yesterday's try"
        );
    }

    #[test]
    fn today_lands_on_the_time() {
        let now = 1_792_411_200;
        let slot = DailyTime::new(7, 0).today(now).unwrap();
        let local = clock::local(slot).unwrap();
        assert_eq!((local.hour, local.minute), (7, 0));
    }
}
