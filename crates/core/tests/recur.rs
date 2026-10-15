use chrono::{DateTime, Utc};
use lyric_cal_core::db::{Db, Event, EventInput, Exception};
use lyric_cal_core::recur::{expand, Occurrence};

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn input(start: &str, end: &str, tz: &str, rrule: Option<&str>) -> EventInput {
    EventInput {
        title: "Standup".into(),
        notes: String::new(),
        start: start.into(),
        end: end.into(),
        all_day: !start.contains('T'),
        tz: tz.into(),
        rrule: rrule.map(Into::into),
        color: None,
        reminder_minutes: None,
    }
}

fn event(id: i64, input: EventInput) -> Event {
    Event { id, input, booking_id: None, created_at: String::new(), updated_at: String::new() }
}

fn starts(occ: &[Occurrence]) -> Vec<&str> {
    occ.iter().map(|o| o.start.as_str()).collect()
}

fn exception(event_id: i64, original_start: &str) -> Exception {
    Exception {
        event_id,
        original_start: original_start.into(),
        cancelled: false,
        title: None,
        notes: None,
        start: None,
        end: None,
    }
}

#[test]
fn a_plain_event_passes_through_when_it_overlaps() {
    let e = event(1, input("2026-10-19T09:00:00Z", "2026-10-19T10:00:00Z", "UTC", None));
    let hit = expand(std::slice::from_ref(&e), &[], at("2026-10-19T09:30:00Z"), at("2026-10-20T00:00:00Z")).unwrap();
    assert_eq!(starts(&hit), ["2026-10-19T09:00:00Z"]);
    assert_eq!(hit[0].original_start, None);
    let miss = expand(&[e], &[], at("2026-10-19T10:00:00Z"), at("2026-10-20T00:00:00Z")).unwrap();
    assert!(miss.is_empty());
}

#[test]
fn weekly_series_keeps_its_wall_clock_time_across_dst() {
    // 09:00 in New York: EDT (UTC-4) until 1 Nov 2026, EST (UTC-5) after.
    let e = event(1, input("2026-10-19T13:00:00Z", "2026-10-19T13:30:00Z", "America/New_York", Some("FREQ=WEEKLY")));
    let occ = expand(&[e], &[], at("2026-10-19T00:00:00Z"), at("2026-11-10T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occ),
        ["2026-10-19T13:00:00Z", "2026-10-26T13:00:00Z", "2026-11-02T14:00:00Z", "2026-11-09T14:00:00Z"]
    );
    assert_eq!(occ[2].end, "2026-11-02T14:30:00Z");
    assert_eq!(occ[2].original_start.as_deref(), Some("2026-11-02T14:00:00Z"));
}

#[test]
fn byday_follows_the_events_zone_not_utc() {
    // 05:00 Tuesday in Dhaka is still 23:00 Monday in UTC.
    let e = event(1, input("2026-10-19T23:00:00Z", "2026-10-19T23:30:00Z", "Asia/Dhaka", Some("FREQ=WEEKLY;BYDAY=TU")));
    let occ = expand(&[e], &[], at("2026-10-19T00:00:00Z"), at("2026-10-28T00:00:00Z")).unwrap();
    assert_eq!(starts(&occ), ["2026-10-19T23:00:00Z", "2026-10-26T23:00:00Z"]);
}

#[test]
fn count_and_until_end_a_series() {
    let counted = event(1, input("2026-10-19T09:00:00Z", "2026-10-19T09:15:00Z", "UTC", Some("FREQ=DAILY;COUNT=3")));
    let until = event(
        2,
        input("2026-10-19T10:00:00Z", "2026-10-19T10:15:00Z", "UTC", Some("FREQ=DAILY;UNTIL=20261020T235959Z")),
    );
    let occ = expand(&[counted, until], &[], at("2026-10-01T00:00:00Z"), at("2026-11-01T00:00:00Z")).unwrap();
    assert_eq!(
        starts(&occ),
        [
            "2026-10-19T09:00:00Z",
            "2026-10-19T10:00:00Z",
            "2026-10-20T09:00:00Z",
            "2026-10-20T10:00:00Z",
            "2026-10-21T09:00:00Z"
        ]
    );
}

#[test]
fn an_occurrence_running_into_the_window_is_included() {
    let e = event(1, input("2026-10-18T22:00:00Z", "2026-10-19T02:00:00Z", "UTC", Some("FREQ=DAILY")));
    let occ = expand(&[e], &[], at("2026-10-19T00:00:00Z"), at("2026-10-19T12:00:00Z")).unwrap();
    assert_eq!(starts(&occ), ["2026-10-18T22:00:00Z"]);
}

#[test]
fn a_cancelled_exception_removes_one_occurrence() {
    let e = event(1, input("2026-10-19T09:00:00Z", "2026-10-19T09:15:00Z", "UTC", Some("FREQ=DAILY")));
    let mut x = exception(1, "2026-10-20T09:00:00Z");
    x.cancelled = true;
    let occ = expand(&[e], &[x], at("2026-10-19T00:00:00Z"), at("2026-10-22T00:00:00Z")).unwrap();
    assert_eq!(starts(&occ), ["2026-10-19T09:00:00Z", "2026-10-21T09:00:00Z"]);
}

#[test]
fn an_override_edits_and_moves_one_occurrence() {
    let e = event(1, input("2026-10-19T09:00:00Z", "2026-10-19T09:15:00Z", "UTC", Some("FREQ=DAILY")));
    let mut renamed = exception(1, "2026-10-19T09:00:00Z");
    renamed.title = Some("Standup (short)".into());
    let mut moved = exception(1, "2026-10-20T09:00:00Z");
    moved.start = Some("2026-10-20T15:00:00Z".into());
    moved.end = Some("2026-10-20T16:00:00Z".into());
    let occ = expand(&[e], &[renamed, moved], at("2026-10-19T00:00:00Z"), at("2026-10-21T00:00:00Z")).unwrap();
    assert_eq!(starts(&occ), ["2026-10-19T09:00:00Z", "2026-10-20T15:00:00Z"]);
    assert_eq!(occ[0].title, "Standup (short)");
    assert_eq!(occ[1].title, "Standup");
    assert_eq!(occ[1].end, "2026-10-20T16:00:00Z");
    assert_eq!(occ[1].original_start.as_deref(), Some("2026-10-20T09:00:00Z"));
}

#[test]
fn moved_occurrences_follow_their_new_time_in_and_out_of_the_window() {
    let e = event(1, input("2026-10-19T09:00:00Z", "2026-10-19T09:15:00Z", "UTC", Some("FREQ=DAILY")));
    // The 25th moved into the window, the 20th moved out of it.
    let mut into = exception(1, "2026-10-25T09:00:00Z");
    into.start = Some("2026-10-20T12:00:00Z".into());
    into.end = Some("2026-10-20T12:15:00Z".into());
    let mut out = exception(1, "2026-10-20T09:00:00Z");
    out.start = Some("2026-10-30T09:00:00Z".into());
    out.end = Some("2026-10-30T09:15:00Z".into());
    let occ = expand(&[e], &[into, out], at("2026-10-20T00:00:00Z"), at("2026-10-21T00:00:00Z")).unwrap();
    assert_eq!(starts(&occ), ["2026-10-20T12:00:00Z"]);
    assert_eq!(occ[0].original_start.as_deref(), Some("2026-10-25T09:00:00Z"));
}

#[test]
fn all_day_series_step_by_date() {
    let e = event(1, input("2026-10-30", "2026-11-01", "America/New_York", Some("FREQ=WEEKLY;COUNT=3")));
    let occ = expand(&[e], &[], at("2026-11-01T00:00:00Z"), at("2026-12-01T00:00:00Z")).unwrap();
    // The first one (30-31 Oct, New York) still overlaps 1 Nov 00:00Z.
    assert_eq!(starts(&occ), ["2026-10-30", "2026-11-06", "2026-11-13"]);
    assert_eq!(occ[1].end, "2026-11-08");
    assert!(occ.iter().all(|o| o.all_day));
}

#[test]
fn results_are_sorted_by_start() {
    let a = event(1, input("2026-10-19T12:00:00Z", "2026-10-19T13:00:00Z", "UTC", Some("FREQ=DAILY")));
    let b = event(2, input("2026-10-19T09:00:00Z", "2026-10-19T10:00:00Z", "UTC", None));
    let occ = expand(&[a, b], &[], at("2026-10-19T00:00:00Z"), at("2026-10-21T00:00:00Z")).unwrap();
    assert_eq!(starts(&occ), ["2026-10-19T09:00:00Z", "2026-10-19T12:00:00Z", "2026-10-20T12:00:00Z"]);
}

// ---------- through the store ----------

#[test]
fn the_store_rejects_rules_the_expander_cannot_run() {
    let db = Db::open_in_memory().unwrap();
    for bad in ["FREQ=SOMETIMES", "FREQ=DAILY;COUNT=0", "BYDAY=MO", "FREQ=DAILY;UNTIL=20200101T000000Z"] {
        let e = input("2026-10-19T09:00:00Z", "2026-10-19T09:15:00Z", "UTC", Some(bad));
        assert!(db.create_event(&e).is_err(), "{bad} was accepted");
    }
    let ok = input("2026-10-19T09:00:00Z", "2026-10-19T09:15:00Z", "UTC", Some("RRULE:FREQ=WEEKLY;BYDAY=MO,WE"));
    let id = db.create_event(&ok).unwrap();
    assert_eq!(db.event(id).unwrap().unwrap().rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE"));
}

#[test]
fn exceptions_need_a_series_and_round_trip() {
    let db = Db::open_in_memory().unwrap();
    let single = db.create_event(&input("2026-10-19T09:00:00Z", "2026-10-19T09:15:00Z", "UTC", None)).unwrap();
    assert!(db.set_exception(&exception(single, "2026-10-19T09:00:00Z")).is_err());

    let id =
        db.create_event(&input("2026-10-19T09:00:00Z", "2026-10-19T09:15:00Z", "UTC", Some("FREQ=DAILY"))).unwrap();
    assert!(db.set_exception(&exception(id, "2026-10-20T09:30:00Z")).is_err(), "not an occurrence");
    let mut x = exception(id, "2026-10-20T15:00:00+06:00");
    x.start = Some("2026-10-20T12:00:00Z".into());
    assert!(db.set_exception(&x).is_err(), "a moved occurrence needs both start and end");
    x.end = Some("2026-10-20T11:00:00Z".into());
    assert!(db.set_exception(&x).is_err(), "end before start");
    x.end = Some("2026-10-20T12:15:00Z".into());
    db.set_exception(&x).unwrap();
    x.title = Some("Moved".into());
    db.set_exception(&x).unwrap(); // upsert

    let stored = db.exceptions_for(&[id]).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].original_start, "2026-10-20T09:00:00Z");
    assert_eq!(stored[0].title.as_deref(), Some("Moved"));

    db.delete_exception(id, "2026-10-20T09:00:00Z").unwrap();
    assert!(db.exceptions_for(&[id]).unwrap().is_empty());
}

#[test]
fn rescheduling_a_series_drops_its_exceptions_but_renaming_keeps_them() {
    let db = Db::open_in_memory().unwrap();
    let mut e = input("2026-10-19T09:00:00Z", "2026-10-19T09:15:00Z", "UTC", Some("FREQ=DAILY"));
    let id = db.create_event(&e).unwrap();
    let mut x = exception(id, "2026-10-20T09:00:00Z");
    x.cancelled = true;
    db.set_exception(&x).unwrap();

    e.title = "Daily".into();
    db.update_event(id, &e).unwrap();
    assert_eq!(db.exceptions_for(&[id]).unwrap().len(), 1);

    e.start = "2026-10-19T10:00:00Z".into();
    e.end = "2026-10-19T10:15:00Z".into();
    db.update_event(id, &e).unwrap();
    assert!(db.exceptions_for(&[id]).unwrap().is_empty());
}

#[test]
fn occurrences_between_reads_and_expands() {
    let db = Db::open_in_memory().unwrap();
    let id =
        db.create_event(&input("2026-10-19T09:00:00Z", "2026-10-19T09:15:00Z", "UTC", Some("FREQ=DAILY"))).unwrap();
    db.create_event(&input("2026-10-20T11:00:00Z", "2026-10-20T12:00:00Z", "UTC", None)).unwrap();
    let mut x = exception(id, "2026-10-21T09:00:00Z");
    x.cancelled = true;
    db.set_exception(&x).unwrap();

    let occ = db.occurrences_between("2026-10-20T00:00:00Z", "2026-10-23T00:00:00Z").unwrap();
    assert_eq!(starts(&occ), ["2026-10-20T09:00:00Z", "2026-10-20T11:00:00Z", "2026-10-22T09:00:00Z"]);
}

#[test]
fn all_day_series_take_a_date_until_and_date_exceptions() {
    let db = Db::open_in_memory().unwrap();
    let e = input("2026-10-19", "2026-10-20", "Asia/Dhaka", Some("FREQ=DAILY;UNTIL=20261022"));
    let id = db.create_event(&e).unwrap();
    let mut x = exception(id, "2026-10-20");
    x.start = Some("2026-10-25".into());
    x.end = Some("2026-10-26".into());
    db.set_exception(&x).unwrap();
    assert!(db.set_exception(&exception(id, "2026-10-23")).is_err(), "past UNTIL");

    let occ = db.occurrences_between("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z").unwrap();
    assert_eq!(starts(&occ), ["2026-10-19", "2026-10-21", "2026-10-22", "2026-10-25"]);
}
