//! Which reminders are due between two checks.

use chrono::{DateTime, Utc};
use lyric_cal_core::db::{Db, EventInput};

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn event(title: &str, start: &str, end: &str, all_day: bool, rrule: Option<&str>, remind: Option<i64>) -> EventInput {
    EventInput {
        title: title.into(),
        notes: String::new(),
        start: start.into(),
        end: end.into(),
        all_day,
        tz: "Asia/Dhaka".into(),
        rrule: rrule.map(Into::into),
        color: None,
        reminder_minutes: remind,
    }
}

fn due(db: &Db, after: &str, upto: &str) -> Vec<String> {
    db.due_reminders(at(after), at(upto), chrono_tz::Asia::Dhaka).unwrap().into_iter().map(|r| r.title).collect()
}

#[test]
fn a_reminder_fires_once_in_the_check_that_covers_it() {
    let db = Db::open_in_memory().unwrap();
    db.create_event(&event("Dentist", "2026-10-19T09:00:00Z", "2026-10-19T09:30:00Z", false, None, Some(10))).unwrap();
    db.create_event(&event("Quiet", "2026-10-19T09:00:00Z", "2026-10-19T09:30:00Z", false, None, None)).unwrap();
    assert!(due(&db, "2026-10-19T08:40:00Z", "2026-10-19T08:49:59Z").is_empty());
    assert_eq!(due(&db, "2026-10-19T08:49:30Z", "2026-10-19T08:50:00Z"), ["Dentist"]);
    assert!(due(&db, "2026-10-19T08:50:00Z", "2026-10-19T08:50:30Z").is_empty(), "not twice");
}

#[test]
fn series_all_day_and_long_reminders() {
    let db = Db::open_in_memory().unwrap();
    db.create_event(&event(
        "Standup",
        "2026-10-19T03:00:00Z",
        "2026-10-19T03:15:00Z",
        false,
        Some("FREQ=DAILY"),
        Some(5),
    ))
    .unwrap();
    // All day on the 21st in Dhaka starts 2026-10-20T18:00Z; a day ahead is the 19th 18:00Z.
    db.create_event(&event("Birthday", "2026-10-21", "2026-10-22", true, None, Some(24 * 60))).unwrap();
    assert_eq!(due(&db, "2026-10-21T02:50:00Z", "2026-10-21T02:56:00Z"), ["Standup"]);
    assert_eq!(due(&db, "2026-10-19T17:59:00Z", "2026-10-19T18:00:00Z"), ["Birthday"]);
}
