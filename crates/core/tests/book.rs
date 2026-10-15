//! Booking a slot: what is offered, and the guard against booking it twice.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use lyric_cal_core::db::{BookRequest, Db, Error, EventInput, EventTypeInput, WeeklyHours};

const DHAKA: Tz = chrono_tz::Asia::Dhaka;

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

/// Mondays 09:00–12:00 in Dhaka (03:00–06:00 UTC); a 30 minute call, no notice needed.
fn setup(db: &Db) {
    db.set_weekly_hours(&[WeeklyHours { weekday: 1, start_minute: 540, end_minute: 720 }]).unwrap();
    db.create_event_type(&EventTypeInput {
        slug: "chat".into(),
        title: "Chat".into(),
        description: String::new(),
        duration_min: 30,
        buffer_before_min: 0,
        buffer_after_min: 0,
        min_notice_min: 0,
        max_days_ahead: 30,
        color: Some("#a5e0ca".into()),
        active: true,
    })
    .unwrap();
}

fn now() -> DateTime<Utc> {
    at("2026-10-15T00:00:00Z")
}

fn monday(db: &Db) -> Vec<String> {
    let t = db.event_type_by_slug("chat").unwrap().unwrap();
    db.slots_for(&t, DHAKA, DHAKA, now(), at("2026-10-19T00:00:00Z"), at("2026-10-20T00:00:00Z"))
        .unwrap()
        .iter()
        .map(|s| s.with_timezone(&DHAKA).format("%H:%M").to_string())
        .collect()
}

fn request(start: &str) -> BookRequest {
    BookRequest {
        slug: "chat".into(),
        start: start.into(),
        guest_name: "Ana Ruiz".into(),
        guest_email: "ana@example.com".into(),
        guest_note: "About the calendar.".into(),
        guest_tz: "Europe/Madrid".into(),
    }
}

#[test]
fn timed_events_take_their_slots_but_all_day_ones_do_not() {
    let db = Db::open_in_memory().unwrap();
    setup(&db);
    assert_eq!(monday(&db), ["09:00", "09:30", "10:00", "10:30", "11:00", "11:30"]);
    let event = |start: &str, end: &str, all_day| EventInput {
        title: "Busy".into(),
        notes: String::new(),
        start: start.into(),
        end: end.into(),
        all_day,
        tz: "Asia/Dhaka".into(),
        rrule: None,
        color: None,
        reminder_minutes: None,
    };
    db.create_event(&event("2026-10-19T04:00:00Z", "2026-10-19T04:30:00Z", false)).unwrap();
    db.create_event(&event("2026-10-19", "2026-10-20", true)).unwrap();
    assert_eq!(monday(&db), ["09:00", "09:30", "10:30", "11:00", "11:30"]);
}

#[test]
fn booking_fills_the_grid_and_the_slot() {
    let db = Db::open_in_memory().unwrap();
    setup(&db);
    let b = db.book(&request("2026-10-19T03:30:00Z"), DHAKA, now()).unwrap();
    assert_eq!((b.start.as_str(), b.end.as_str()), ("2026-10-19T03:30:00Z", "2026-10-19T04:00:00Z"));
    assert_eq!((b.type_title.as_str(), b.status.as_str()), ("Chat", "confirmed"));

    let grid = db.occurrences_between("2026-10-19T00:00:00Z", "2026-10-20T00:00:00Z").unwrap();
    assert_eq!(grid.len(), 1);
    assert_eq!(grid[0].title, "Chat with Ana Ruiz");
    assert_eq!(grid[0].booking_id, Some(b.id));
    assert!(grid[0].notes.contains("ana@example.com") && grid[0].notes.contains("About the calendar."));
    assert_eq!(monday(&db), ["09:00", "10:00", "10:30", "11:00", "11:30"]);

    assert!(matches!(db.book(&request("2026-10-19T03:30:00Z"), DHAKA, now()), Err(Error::Taken)));
    db.cancel_booking(b.id).unwrap();
    db.book(&request("2026-10-19T03:30:00Z"), DHAKA, now()).unwrap();
}

#[test]
fn only_offered_slots_can_be_booked() {
    let db = Db::open_in_memory().unwrap();
    setup(&db);
    for off in ["2026-10-19T03:10:00Z", "2026-10-19T06:00:00Z", "2026-10-20T03:00:00Z", "2026-10-12T03:00:00Z"] {
        assert!(matches!(db.book(&request(off), DHAKA, now()), Err(Error::Taken)), "{off}");
    }
    let mut r = request("2026-10-19T03:00:00Z");
    r.slug = "nope".into();
    assert!(matches!(db.book(&r, DHAKA, now()), Err(Error::NotFound(_))));

    let t = db.event_type_by_slug("chat").unwrap().unwrap();
    let mut off = t.input.clone();
    off.active = false;
    db.update_event_type(t.id, &off).unwrap();
    assert!(matches!(db.book(&request("2026-10-19T03:00:00Z"), DHAKA, now()), Err(Error::NotFound(_))));
}

#[test]
fn guests_must_say_who_they_are() {
    let db = Db::open_in_memory().unwrap();
    setup(&db);
    type Spoil = fn(&mut BookRequest);
    let cases: [(&str, Spoil); 4] = [
        ("name", |r| r.guest_name = "  ".into()),
        ("email", |r| r.guest_email = "ana-at-example".into()),
        ("zone", |r| r.guest_tz = "Mars/Olympus".into()),
        ("start", |r| r.start = "monday".into()),
    ];
    for (what, spoil) in cases {
        let mut r = request("2026-10-19T03:00:00Z");
        spoil(&mut r);
        assert!(matches!(db.book(&r, DHAKA, now()), Err(Error::Invalid(_))), "{what}");
    }
    assert!(db.bookings_from("2026-10-01T00:00:00Z").unwrap().is_empty());
}

#[test]
fn two_guests_racing_for_one_slot_get_one_booking() {
    let dir = std::env::temp_dir().join(format!("lyric-cal-race-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("cal.db");
    setup(&Db::open(&path).unwrap());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let (path, barrier) = (path.clone(), barrier.clone());
            std::thread::spawn(move || {
                let db = Db::open(&path).unwrap();
                barrier.wait();
                db.book(&request("2026-10-19T03:00:00Z"), DHAKA, now()).is_ok()
            })
        })
        .collect();
    let won = handles.into_iter().map(|h| h.join().unwrap());
    assert_eq!(won.filter(|&ok| ok).count(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_booking_keeps_its_own_gap_after_it() {
    let db = Db::open_in_memory().unwrap();
    setup(&db);
    let t = db.event_type_by_slug("chat").unwrap().unwrap();
    let mut gap = t.input.clone();
    gap.buffer_after_min = 10;
    db.update_event_type(t.id, &gap).unwrap();
    db.book(&request("2026-10-19T03:00:00Z"), DHAKA, now()).unwrap();
    // 09:00–09:30 is booked and keeps 09:30–09:40 clear, so 09:30 is gone and
    // 10:00 (which needs 10:30–10:40 clear) is fine.
    assert_eq!(monday(&db), ["10:00", "10:30", "11:00", "11:30"]);
}
