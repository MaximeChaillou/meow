//! Shapes a conversation for reading: messages from one author in quick
//! succession share a single header, and each day opens with a divider.
//! Also the short time labels of the conversation list.

use crate::clock::{self, LocalTime};

use super::model::Message;

/// Consecutive messages closer together than this share one header.
const GROUP_GAP_SECS: u64 = 5 * 60;
const DAY_SECS: u64 = 24 * 60 * 60;
/// Within this many days, the list shows a weekday rather than a date.
const WEEKDAY_WITHIN_DAYS: u64 = 6;

/// One piece of a conversation as shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry<'a> {
    /// "Today", "Yesterday", "Monday 12 October".
    Day(String),
    /// A run of messages by one author, with the time of the first.
    Run {
        author: Author<'a>,
        time: String,
        messages: Vec<&'a Message>,
    },
}

/// Who wrote a run: a user id to resolve, or a bot's own name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Author<'a> {
    User(&'a str),
    Bot(&'a str),
    Unknown,
}

impl<'a> Author<'a> {
    fn of(message: &'a Message) -> Self {
        match (message.user.as_deref(), message.bot_name.as_deref()) {
            (Some(user), _) => Author::User(user),
            (None, Some(bot)) => Author::Bot(bot),
            (None, None) => Author::Unknown,
        }
    }
}

/// Groups `messages` (oldest first) into day dividers and author runs.
pub fn timeline(messages: &[Message], now: u64) -> Vec<Entry<'_>> {
    let today = clock::local(now);
    let mut entries = Vec::new();
    let mut last: Option<(Author, u64, Option<LocalTime>)> = None;
    for message in messages {
        let at = ts_secs(&message.ts);
        let local = at.and_then(clock::local);
        let author = Author::of(message);
        let new_day = match (&last, &local) {
            (Some((_, _, Some(prev))), Some(day)) => !prev.same_day(day),
            (None, Some(_)) => true,
            _ => false,
        };
        if new_day && let (Some(day), Some(today)) = (&local, &today) {
            entries.push(Entry::Day(day_label(day, today, now)));
        }
        let continues = !new_day
            && matches!(last, Some((prev, prev_at, _))
                if prev == author && at.is_some_and(|at| at.saturating_sub(prev_at) <= GROUP_GAP_SECS));
        match entries.last_mut() {
            Some(Entry::Run { messages, .. }) if continues => messages.push(message),
            _ => entries.push(Entry::Run {
                author,
                time: local.map(|t| clock_time(&t)).unwrap_or_default(),
                messages: vec![message],
            }),
        }
        last = Some((author, at.unwrap_or(0), local));
    }
    entries
}

/// Groups a thread (parent first): the parent on its own, a divider counting
/// the replies, then the replies grouped as usual, their day shown only when
/// it is not the parent's.
pub fn thread(messages: &[Message], now: u64) -> Vec<Entry<'_>> {
    let Some((parent, replies)) = messages.split_first() else {
        return Vec::new();
    };
    let mut entries = timeline(std::slice::from_ref(parent), now);
    let Some(first_reply) = replies.first() else {
        return entries;
    };
    entries.push(Entry::Day(replies_label(replies.len())));
    let day = |message: &Message| ts_secs(&message.ts).and_then(clock::local);
    let same_day = matches!((day(parent), day(first_reply)),
        (Some(parent), Some(reply)) if parent.same_day(&reply));
    let mut rest = timeline(replies, now);
    if same_day && matches!(rest.first(), Some(Entry::Day(_))) {
        rest.remove(0);
    }
    entries.extend(rest);
    entries
}

/// "1 reply", "3 replies".
pub fn replies_label(count: usize) -> String {
    match count {
        1 => "1 reply".to_string(),
        n => format!("{n} replies"),
    }
}

/// The first of `wanted` (message `ts`, best first) that is among `messages`.
pub fn first_loaded<'a>(messages: &[Message], wanted: &'a [String]) -> Option<&'a str> {
    wanted
        .iter()
        .find(|ts| messages.iter().any(|m| &m.ts == *ts))
        .map(String::as_str)
}

/// A compact label for the list: "14:32", "Yesterday", "Mon", "12 Oct".
pub fn list_label(ts: &str, now: u64) -> String {
    let (Some(at), Some(today)) = (ts_secs(ts), clock::local(now)) else {
        return String::new();
    };
    let Some(then) = clock::local(at) else {
        return String::new();
    };
    if then.same_day(&today) {
        clock_time(&then)
    } else if is_yesterday(&then, now) {
        "Yesterday".to_string()
    } else if now.saturating_sub(at) < WEEKDAY_WITHIN_DAYS * DAY_SECS {
        then.weekday_short().to_string()
    } else if then.year == today.year {
        then.short_date()
    } else {
        format!("{} {}", then.short_date(), then.year)
    }
}

fn day_label(day: &LocalTime, today: &LocalTime, now: u64) -> String {
    if day.same_day(today) {
        "Today".to_string()
    } else if is_yesterday(day, now) {
        "Yesterday".to_string()
    } else if day.year == today.year {
        day.date_label()
    } else {
        format!("{} {}", day.date_label(), day.year)
    }
}

fn is_yesterday(day: &LocalTime, now: u64) -> bool {
    clock::local(now.saturating_sub(DAY_SECS)).is_some_and(|y| y.same_day(day))
}

fn clock_time(t: &LocalTime) -> String {
    format!("{:02}:{:02}", t.hour, t.minute)
}

/// Slack's `ts` ("1729332000.123456") as whole seconds.
fn ts_secs(ts: &str) -> Option<u64> {
    ts.split('.').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON_19_OCT_2026_UTC: u64 = 1_792_411_200;

    fn at(hour: u32, minute: u32, days_ago: u64) -> u64 {
        let day = NOON_19_OCT_2026_UTC - days_ago * DAY_SECS;
        clock::same_day_at(day, hour, minute).unwrap()
    }

    fn message(user: &str, secs: u64) -> Message {
        Message {
            ts: format!("{secs}.000100"),
            user: Some(user.into()),
            text: "hi".into(),
            thread_ts: None,
            reply_count: 0,
            files: Vec::new(),
            images: Vec::new(),
            attachment: None,
            bot_name: None,
            forwarded: Vec::new(),
        }
    }

    fn shape(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Day(label) => format!("day {label}"),
                Entry::Run {
                    author,
                    time,
                    messages,
                } => format!("{author:?} {time} x{}", messages.len()),
            })
            .collect()
    }

    #[test]
    fn quick_messages_by_one_author_share_a_header() {
        let now = at(18, 0, 0);
        let messages = [
            message("U1", at(9, 0, 0)),
            message("U1", at(9, 2, 0)),
            message("U2", at(9, 3, 0)),
            message("U1", at(9, 4, 0)),
            message("U1", at(9, 30, 0)),
        ];
        assert_eq!(
            shape(&timeline(&messages, now)),
            [
                "day Today",
                "User(\"U1\") 09:00 x2",
                "User(\"U2\") 09:03 x1",
                "User(\"U1\") 09:04 x1",
                "User(\"U1\") 09:30 x1",
            ]
        );
    }

    #[test]
    fn each_day_opens_with_a_divider_and_a_fresh_header() {
        let now = at(18, 0, 0);
        let messages = [message("U1", at(23, 58, 1)), message("U1", at(0, 1, 0))];
        assert_eq!(
            shape(&timeline(&messages, now)),
            [
                "day Yesterday",
                "User(\"U1\") 23:58 x1",
                "day Today",
                "User(\"U1\") 00:01 x1",
            ]
        );
    }

    #[test]
    fn a_thread_sets_its_parent_apart_from_the_replies() {
        let now = at(18, 0, 0);
        let messages = [
            message("U1", at(9, 0, 0)),
            message("U1", at(9, 1, 0)),
            message("U2", at(9, 2, 0)),
        ];
        assert_eq!(
            shape(&thread(&messages, now)),
            [
                "day Today",
                "User(\"U1\") 09:00 x1",
                "day 2 replies",
                "User(\"U1\") 09:01 x1",
                "User(\"U2\") 09:02 x1",
            ]
        );
    }

    #[test]
    fn replies_on_a_later_day_keep_their_day() {
        let now = at(18, 0, 0);
        let messages = [message("U1", at(9, 0, 1)), message("U2", at(9, 0, 0))];
        assert_eq!(
            shape(&thread(&messages, now)),
            [
                "day Yesterday",
                "User(\"U1\") 09:00 x1",
                "day 1 reply",
                "day Today",
                "User(\"U2\") 09:00 x1",
            ]
        );
    }

    #[test]
    fn a_thread_without_replies_is_just_its_parent() {
        let now = at(18, 0, 0);
        let parent = [message("U1", at(9, 0, 0))];
        assert_eq!(
            shape(&thread(&parent, now)),
            ["day Today", "User(\"U1\") 09:00 x1"]
        );
        assert!(thread(&[], now).is_empty());
    }

    #[test]
    fn reply_counts_read_naturally() {
        assert_eq!(replies_label(1), "1 reply");
        assert_eq!(replies_label(3), "3 replies");
    }

    #[test]
    fn bots_are_named_by_their_profile() {
        let now = at(18, 0, 0);
        let bot = Message {
            user: None,
            bot_name: Some("CI".into()),
            ..message("", at(9, 0, 0))
        };
        let entries = timeline(std::slice::from_ref(&bot), now);
        assert!(matches!(
            entries[1],
            Entry::Run {
                author: Author::Bot("CI"),
                ..
            }
        ));
    }

    #[test]
    fn the_wanted_message_falls_back_to_its_thread_when_not_loaded() {
        let messages = [message("U1", 100), message("U1", 200)];
        let wanted = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let reply_in_thread = wanted(&["300.000100", "100.000100"]);
        assert_eq!(
            first_loaded(&messages, &reply_in_thread),
            Some("100.000100")
        );
        let loaded = wanted(&["200.000100", "100.000100"]);
        assert_eq!(first_loaded(&messages, &loaded), Some("200.000100"));
        assert_eq!(first_loaded(&messages, &wanted(&["999.0"])), None);
        assert_eq!(first_loaded(&messages, &[]), None);
    }

    #[test]
    fn list_labels_shrink_with_age() {
        let now = at(18, 0, 0);
        let label = |secs: u64| list_label(&format!("{secs}.0001"), now);
        assert_eq!(label(at(14, 32, 0)), "14:32");
        assert_eq!(label(at(14, 32, 1)), "Yesterday");
        assert_eq!(label(at(14, 32, 3)), "Fri");
        assert_eq!(label(at(14, 32, 30)), "19 Sep");
        assert_eq!(label(at(14, 32, 400)), "14 Sep 2025");
        assert_eq!(list_label("", now), "");
    }
}
