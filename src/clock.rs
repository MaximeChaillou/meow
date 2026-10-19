//! Local wall-clock time, for daily quick actions and the day plan.

use std::mem::MaybeUninit;

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A moment as read on the Mac's clock, in its time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    /// 0 is Sunday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

impl LocalTime {
    pub fn minutes_of_day(&self) -> u32 {
        self.hour * 60 + self.minute
    }

    /// "Monday 19 October".
    pub fn date_label(&self) -> String {
        let weekday = WEEKDAYS[self.weekday as usize % 7];
        let month = MONTHS[(self.month as usize + 11) % 12];
        format!("{weekday} {} {month}", self.day)
    }

    /// "Mon".
    pub fn weekday_short(&self) -> &'static str {
        &WEEKDAYS[self.weekday as usize % 7][..3]
    }

    /// "19 Oct".
    pub fn short_date(&self) -> String {
        let month = MONTHS[(self.month as usize + 11) % 12];
        format!("{} {}", self.day, &month[..3])
    }

    pub fn same_day(&self, other: &LocalTime) -> bool {
        (self.year, self.month, self.day) == (other.year, other.month, other.day)
    }
}

fn local_tm(at: u64) -> Option<libc::tm> {
    let time = libc::time_t::try_from(at).ok()?;
    let mut tm = MaybeUninit::<libc::tm>::zeroed();
    // SAFETY: both pointers are valid for the call; `localtime_r` is the thread-safe variant
    // and fills `tm` entirely when it succeeds.
    unsafe {
        if libc::localtime_r(&time, tm.as_mut_ptr()).is_null() {
            return None;
        }
        Some(tm.assume_init())
    }
}

/// The local date and time of `at` (seconds since the epoch).
pub fn local(at: u64) -> Option<LocalTime> {
    let tm = local_tm(at)?;
    Some(LocalTime {
        year: tm.tm_year + 1900,
        month: u32::try_from(tm.tm_mon + 1).ok()?,
        day: u32::try_from(tm.tm_mday).ok()?,
        weekday: u32::try_from(tm.tm_wday).ok()?,
        hour: u32::try_from(tm.tm_hour).ok()?,
        minute: u32::try_from(tm.tm_min).ok()?,
    })
}

/// `hour:minute` on the local day of `at`, in seconds since the epoch.
pub fn same_day_at(at: u64, hour: u32, minute: u32) -> Option<u64> {
    let mut tm = local_tm(at)?;
    tm.tm_hour = i32::try_from(hour).ok()?;
    tm.tm_min = i32::try_from(minute).ok()?;
    tm.tm_sec = 0;
    // Let the system work out daylight saving time for that hour.
    tm.tm_isdst = -1;
    // SAFETY: `tm` is a valid, initialized struct owned by this frame.
    let time = unsafe { libc::mktime(&mut tm) };
    u64::try_from(time).ok()
}

/// "Today is Monday 19 October 2026, 07:04 local time."
pub fn describe(at: u64) -> String {
    match local(at) {
        Some(t) => format!(
            "Today is {} {}, {:02}:{:02} local time.",
            t.date_label(),
            t.year,
            t.hour,
            t.minute
        ),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON_19_OCT_2026_UTC: u64 = 1_792_411_200;

    #[test]
    fn same_day_at_lands_on_that_local_time_and_day() {
        let at = NOON_19_OCT_2026_UTC;
        let seven = same_day_at(at, 7, 0).unwrap();
        let (day, slot) = (local(at).unwrap(), local(seven).unwrap());
        assert_eq!((slot.hour, slot.minute), (7, 0));
        assert_eq!(
            (slot.year, slot.month, slot.day),
            (day.year, day.month, day.day)
        );
    }

    #[test]
    fn labels_read_naturally() {
        let t = LocalTime {
            year: 2026,
            month: 10,
            day: 19,
            weekday: 1,
            hour: 7,
            minute: 4,
        };
        assert_eq!(t.date_label(), "Monday 19 October");
        assert_eq!(t.weekday_short(), "Mon");
        assert_eq!(t.short_date(), "19 Oct");
        assert!(t.same_day(&LocalTime { hour: 23, ..t }));
        assert!(!t.same_day(&LocalTime { day: 20, ..t }));
        assert_eq!(t.minutes_of_day(), 424);
        assert!(describe(NOON_19_OCT_2026_UTC).starts_with("Today is "));
    }
}
