use lyric_cal_core::db::{Db, EventInput, EventTypeInput, Override, WeeklyHours};

fn db() -> Db {
    Db::open_in_memory().unwrap()
}

fn meeting() -> EventInput {
    EventInput {
        title: "Dentist".into(),
        notes: String::new(),
        start: "2026-10-19T09:00:00Z".into(),
        end: "2026-10-19T09:30:00Z".into(),
        all_day: false,
        tz: "Asia/Dhaka".into(),
        rrule: None,
        color: None,
        reminder_minutes: Some(10),
    }
}

#[test]
fn migrations_run_once_and_reopen_cleanly() {
    let dir = std::env::temp_dir().join(format!("lyric-cal-test-{}", std::process::id()));
    let path = dir.join("cal.db");
    let _ = std::fs::remove_dir_all(&dir);
    {
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), Db::SCHEMA_VERSION);
        db.create_event(&meeting()).unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert_eq!(db.schema_version().unwrap(), Db::SCHEMA_VERSION);
    assert_eq!(db.events_between("2026-10-19T00:00:00Z", "2026-10-20T00:00:00Z").unwrap().len(), 1);
    assert_eq!(db.journal_mode().unwrap(), "wal");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn events_round_trip() {
    let db = db();
    let id = db.create_event(&meeting()).unwrap();
    let mut e = db.event(id).unwrap().unwrap();
    assert_eq!(e.title, "Dentist");
    assert_eq!(e.reminder_minutes, Some(10));

    e.input.title = "Dentist (moved)".into();
    e.input.start = "2026-10-20T09:00:00Z".into();
    e.input.end = "2026-10-20T09:30:00Z".into();
    db.update_event(id, &e.input).unwrap();
    assert_eq!(db.event(id).unwrap().unwrap().title, "Dentist (moved)");

    db.delete_event(id).unwrap();
    assert!(db.event(id).unwrap().is_none());
}

#[test]
fn range_query_finds_overlaps_and_every_series() {
    let db = db();
    db.create_event(&meeting()).unwrap();
    let mut series = meeting();
    series.title = "Standup".into();
    series.start = "2026-01-05T03:00:00Z".into();
    series.end = "2026-01-05T03:15:00Z".into();
    series.rrule = Some("FREQ=DAILY".into());
    db.create_event(&series).unwrap();
    let mut later = meeting();
    later.start = "2026-11-01T09:00:00Z".into();
    later.end = "2026-11-01T10:00:00Z".into();
    db.create_event(&later).unwrap();

    let titles: Vec<String> = db
        .events_between("2026-10-19T09:15:00Z", "2026-10-20T00:00:00Z")
        .unwrap()
        .into_iter()
        .map(|e| e.input.title)
        .collect();
    assert_eq!(titles, ["Standup", "Dentist"]);
}

#[test]
fn bad_input_is_refused() {
    let db = db();
    let mut e = meeting();
    e.end = "2026-10-19T08:00:00Z".into();
    assert!(db.create_event(&e).is_err());
    let mut e = meeting();
    e.tz = "Mars/Olympus".into();
    assert!(db.create_event(&e).is_err());
    let mut e = meeting();
    e.rrule = Some("FREQ=SOMETIMES".into());
    assert!(db.create_event(&e).is_err());
}

fn intro() -> EventTypeInput {
    EventTypeInput {
        slug: "intro-call".into(),
        title: "Intro call".into(),
        description: "Twenty minutes to say hi.".into(),
        duration_min: 20,
        buffer_before_min: 5,
        buffer_after_min: 10,
        min_notice_min: 240,
        max_days_ahead: 30,
        color: None,
        active: true,
    }
}

#[test]
fn event_types_are_found_by_slug() {
    let db = db();
    let input = intro();
    let id = db.create_event_type(&input).unwrap();
    assert_eq!(db.event_type_by_slug("intro-call").unwrap().unwrap().id, id);
    assert!(db.create_event_type(&input).is_err(), "slugs are unique");

    let mut bad = input.clone();
    bad.slug = "Intro Call!".into();
    assert!(db.create_event_type(&bad).is_err(), "slugs are url-safe");

    let mut off = input.clone();
    off.active = false;
    db.update_event_type(id, &off).unwrap();
    assert!(!db.event_type_by_slug("intro-call").unwrap().unwrap().input.active);
    assert_eq!(db.event_types().unwrap().len(), 1);
    db.delete_event_type(id).unwrap();
    assert!(db.event_types().unwrap().is_empty());
}

#[test]
fn weekly_hours_and_overrides_are_replaced_whole() {
    let db = db();
    let week = vec![
        WeeklyHours { weekday: 1, start_minute: 540, end_minute: 1020 },
        WeeklyHours { weekday: 2, start_minute: 540, end_minute: 720 },
        WeeklyHours { weekday: 2, start_minute: 780, end_minute: 1020 },
    ];
    db.set_weekly_hours(&week).unwrap();
    assert_eq!(db.weekly_hours().unwrap(), week);
    db.set_weekly_hours(&week[..1]).unwrap();
    assert_eq!(db.weekly_hours().unwrap(), week[..1]);
    assert!(db.set_weekly_hours(&[WeeklyHours { weekday: 7, start_minute: 0, end_minute: 60 }]).is_err());

    let off = vec![Override { date: "2026-12-25".into(), start_minute: 0, end_minute: 0 }];
    db.set_overrides(&off).unwrap();
    assert_eq!(db.overrides().unwrap(), off);
    assert_eq!(db.availability().unwrap().len(), 2);
}

#[test]
fn settings_have_defaults() {
    let db = db();
    assert_eq!(db.setting("timezone").unwrap(), None);
    db.set_setting("timezone", "Asia/Dhaka").unwrap();
    assert_eq!(db.setting("timezone").unwrap().as_deref(), Some("Asia/Dhaka"));
}

fn booked(db: &Db, type_id: i64, start: &str, end: &str, guest: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO bookings (event_type_id, start, end, guest_name, guest_email, guest_tz, created_at)
             VALUES (?1, ?2, ?3, ?4, 'guest@example.com', 'Europe/Berlin', '2026-10-01T00:00:00Z')",
            rusqlite::params![type_id, start, end, guest],
        )
        .unwrap();
    let id = db.conn().last_insert_rowid();
    let mut grid = meeting();
    (grid.title, grid.start, grid.end) = (format!("Intro with {guest}"), start.into(), end.into());
    let event = db.create_event(&grid).unwrap();
    db.conn().execute("UPDATE events SET booking_id = ?1 WHERE id = ?2", [id, event]).unwrap();
    id
}

#[test]
fn bookings_list_upcoming_with_their_type() {
    let db = db();
    let t = db.create_event_type(&intro()).unwrap();
    booked(&db, t, "2026-10-01T09:00:00Z", "2026-10-01T09:30:00Z", "Past");
    booked(&db, t, "2026-10-22T09:00:00Z", "2026-10-22T09:30:00Z", "Later");
    booked(&db, t, "2026-10-20T09:00:00Z", "2026-10-20T09:30:00Z", "Sooner");
    let list = db.bookings_from("2026-10-15T00:00:00Z").unwrap();
    let names: Vec<_> = list.iter().map(|b| b.guest_name.as_str()).collect();
    assert_eq!(names, ["Sooner", "Later"]);
    assert_eq!(list[0].type_title, "Intro call");
    assert_eq!(list[0].guest_tz, "Europe/Berlin");
    assert_eq!(list[0].status, "confirmed");
}

#[test]
fn cancelling_a_booking_frees_its_slot_in_the_grid() {
    let db = db();
    let t = db.create_event_type(&intro()).unwrap();
    let id = booked(&db, t, "2026-10-20T09:00:00Z", "2026-10-20T09:30:00Z", "Ana");
    let day = ("2026-10-20T00:00:00Z", "2026-10-21T00:00:00Z");
    assert_eq!(db.occurrences_between(day.0, day.1).unwrap().len(), 1);
    db.cancel_booking(id).unwrap();
    assert!(db.occurrences_between(day.0, day.1).unwrap().is_empty());
    assert_eq!(db.bookings_from(day.0).unwrap()[0].status, "cancelled");
}
