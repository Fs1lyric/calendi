//! The one SQLite file both binaries share (`~/.local/share/lyric-cal/cal.db`).
//!
//! Timed values are stored as UTC RFC3339 (`2026-10-19T09:00:00Z`) so they
//! sort and compare as text; all-day events store `YYYY-MM-DD` with an
//! exclusive end. `tz` keeps the zone an event was made in, which recurrence
//! needs to stay on the same wall-clock time across DST.

use crate::recur::{self, Occurrence};
use crate::slots::{self, Availability, BookingRules, DateOverride, DateRange, WorkingHours};
use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, Utc};
use chrono_tz::Tz;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::ops::Deref;
use std::path::Path;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    /// The time asked for is not (or no longer) offered.
    #[error("that time is no longer free")]
    Taken,
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}

/// Each entry moves the schema one version forward. Never edit a shipped one.
const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE event_types (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    duration_min INTEGER NOT NULL CHECK (duration_min > 0),
    buffer_before_min INTEGER NOT NULL DEFAULT 0,
    buffer_after_min INTEGER NOT NULL DEFAULT 0,
    min_notice_min INTEGER NOT NULL DEFAULT 240,
    max_days_ahead INTEGER NOT NULL DEFAULT 60,
    color TEXT,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE bookings (
    id INTEGER PRIMARY KEY,
    event_type_id INTEGER NOT NULL REFERENCES event_types(id),
    start TEXT NOT NULL,
    end TEXT NOT NULL,
    guest_name TEXT NOT NULL,
    guest_email TEXT NOT NULL,
    guest_note TEXT NOT NULL DEFAULT '',
    guest_tz TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'confirmed' CHECK (status IN ('confirmed', 'cancelled')),
    created_at TEXT NOT NULL
);
CREATE INDEX bookings_start ON bookings(start);

CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    start TEXT NOT NULL,
    end TEXT NOT NULL,
    all_day INTEGER NOT NULL DEFAULT 0,
    tz TEXT NOT NULL,
    rrule TEXT,
    color TEXT,
    reminder_minutes INTEGER,
    booking_id INTEGER REFERENCES bookings(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX events_start ON events(start);

-- One occurrence of a series moved, edited or cancelled.
CREATE TABLE event_exceptions (
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    original_start TEXT NOT NULL,
    cancelled INTEGER NOT NULL DEFAULT 0,
    title TEXT,
    notes TEXT,
    start TEXT,
    end TEXT,
    PRIMARY KEY (event_id, original_start)
);

CREATE TABLE availability (
    id INTEGER PRIMARY KEY,
    weekday INTEGER NOT NULL CHECK (weekday BETWEEN 0 AND 6),
    start_minute INTEGER NOT NULL,
    end_minute INTEGER NOT NULL
);

CREATE TABLE date_overrides (
    id INTEGER PRIMARY KEY,
    date TEXT NOT NULL,
    start_minute INTEGER NOT NULL,
    end_minute INTEGER NOT NULL
);
"#];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventInput {
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub all_day: bool,
    pub tz: String,
    pub rrule: Option<String>,
    pub color: Option<String>,
    pub reminder_minutes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    #[serde(flatten)]
    pub input: EventInput,
    pub booking_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

impl Deref for Event {
    type Target = EventInput;
    fn deref(&self) -> &EventInput {
        &self.input
    }
}

/// One occurrence of a series cancelled, or with its own title, notes or time.
/// `None` fields keep the series' value; `start` and `end` move together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exception {
    pub event_id: i64,
    /// The start the rule gives this occurrence, in stored format.
    pub original_start: String,
    #[serde(default)]
    pub cancelled: bool,
    pub title: Option<String>,
    pub notes: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTypeInput {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub duration_min: i64,
    pub buffer_before_min: i64,
    pub buffer_after_min: i64,
    pub min_notice_min: i64,
    pub max_days_ahead: i64,
    pub color: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventType {
    pub id: i64,
    #[serde(flatten)]
    pub input: EventTypeInput,
}

impl Deref for EventType {
    type Target = EventTypeInput;
    fn deref(&self) -> &EventTypeInput {
        &self.input
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeeklyHours {
    /// 0 = Sunday.
    pub weekday: u32,
    pub start_minute: u32,
    pub end_minute: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Override {
    /// `YYYY-MM-DD` in the organizer's zone.
    pub date: String,
    pub start_minute: u32,
    pub end_minute: u32,
}

/// A guest's booking, with its event type's title for listing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Booking {
    pub id: i64,
    pub event_type_id: i64,
    pub type_title: String,
    pub start: String,
    pub end: String,
    pub guest_name: String,
    pub guest_email: String,
    pub guest_note: String,
    pub guest_tz: String,
    /// `confirmed` or `cancelled`.
    pub status: String,
    pub created_at: String,
}

/// What a guest sends to book a slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookRequest {
    pub slug: String,
    /// RFC3339; must be a start time the booking page offered.
    pub start: String,
    pub guest_name: String,
    pub guest_email: String,
    #[serde(default)]
    pub guest_note: String,
    /// The zone the guest picked the slot in; slots are aligned in it.
    pub guest_tz: String,
}

fn clean_request(r: &BookRequest) -> Result<(BookRequest, DateTime<Utc>, Tz)> {
    let mut r = r.clone();
    r.guest_name = r.guest_name.trim().to_string();
    r.guest_email = r.guest_email.trim().to_string();
    r.guest_note = r.guest_note.trim().to_string();
    if r.guest_name.is_empty() || r.guest_name.chars().count() > 100 {
        return invalid("give your name");
    }
    let email_ok = match r.guest_email.split_once('@') {
        Some((user, domain)) => {
            !user.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !r.guest_email.chars().any(char::is_whitespace)
                && r.guest_email.len() <= 200
        }
        None => false,
    };
    if !email_ok {
        return invalid("give an email address like name@example.com");
    }
    if r.guest_note.chars().count() > 2000 {
        return invalid("keep the note under 2000 characters");
    }
    let tz: Tz = r.guest_tz.parse().or_else(|_| invalid(format!("unknown time zone {}", r.guest_tz)))?;
    let start = DateTime::parse_from_rfc3339(&r.start)
        .map(|t| t.with_timezone(&Utc))
        .or_else(|_| invalid(format!("not a time: {}", r.start)))?;
    Ok((r, start, tz))
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// A reminder to show now.
#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    pub title: String,
    pub start: DateTime<Utc>,
    pub all_day: bool,
}

/// The furthest ahead a reminder can be set (the editor offers up to a week).
const MAX_REMINDER: i64 = 7 * 24 * 60;

pub fn now_stamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Any RFC3339 instant, normalized to the stored UTC form.
pub fn normalize_instant(s: &str) -> Result<String> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Secs, true))
        .or_else(|_| invalid(format!("not a date and time: {s}")))
}

fn parse_day(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").or_else(|_| invalid(format!("not a date: {s}")))
}

fn clean_event(input: &EventInput) -> Result<EventInput> {
    let mut e = input.clone();
    e.title = e.title.trim().to_string();
    if e.tz.parse::<Tz>().is_err() {
        return invalid(format!("unknown time zone: {}", e.tz));
    }
    if e.all_day {
        if parse_day(&e.end)? <= parse_day(&e.start)? {
            return invalid("an all-day event must end after it starts");
        }
    } else {
        e.start = normalize_instant(&e.start)?;
        e.end = normalize_instant(&e.end)?;
        if e.end <= e.start {
            return invalid("an event must end after it starts");
        }
    }
    e.rrule = e.rrule.map(|r| r.trim().trim_start_matches("RRULE:").to_string()).filter(|r| !r.is_empty());
    crate::recur::check_rule(&e)?;
    if e.reminder_minutes.is_some_and(|m| m < 0) {
        return invalid("a reminder can't be after the event starts");
    }
    if e.reminder_minutes.is_some_and(|m| m > MAX_REMINDER) {
        return invalid("a reminder can be at most a week ahead");
    }
    Ok(e)
}

fn clean_event_type(input: &EventTypeInput) -> Result<EventTypeInput> {
    let mut t = input.clone();
    t.slug = t.slug.trim().to_string();
    t.title = t.title.trim().to_string();
    let slug_ok = !t.slug.is_empty()
        && t.slug.len() <= 64
        && t.slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !slug_ok {
        return invalid("the link name can only use lowercase letters, digits and dashes");
    }
    if t.title.is_empty() {
        return invalid("give the event type a title");
    }
    if !(5..=24 * 60).contains(&t.duration_min) {
        return invalid("a booking lasts between 5 minutes and a day");
    }
    let minutes = [t.buffer_before_min, t.buffer_after_min, t.min_notice_min];
    if minutes.iter().any(|&m| m < 0) || !(1..=365).contains(&t.max_days_ahead) {
        return invalid("buffers and notice can't be negative, and booking ahead is 1 to 365 days");
    }
    Ok(t)
}

fn check_minutes(start: u32, end: u32) -> Result<()> {
    if end > 24 * 60 || start > end {
        return invalid("hours must start before they end, within the day");
    }
    Ok(())
}

pub struct Db {
    conn: Connection,
}

const EVENT_COLS: &str =
    "id, title, notes, start, end, all_day, tz, rrule, color, reminder_minutes, booking_id, created_at, updated_at";
const TYPE_COLS: &str = "id, slug, title, description, duration_min, buffer_before_min, buffer_after_min, \
                         min_notice_min, max_days_ahead, color, active";

fn event_row(r: &Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get(0)?,
        input: EventInput {
            title: r.get(1)?,
            notes: r.get(2)?,
            start: r.get(3)?,
            end: r.get(4)?,
            all_day: r.get(5)?,
            tz: r.get(6)?,
            rrule: r.get(7)?,
            color: r.get(8)?,
            reminder_minutes: r.get(9)?,
        },
        booking_id: r.get(10)?,
        created_at: r.get(11)?,
        updated_at: r.get(12)?,
    })
}

fn type_row(r: &Row) -> rusqlite::Result<EventType> {
    Ok(EventType {
        id: r.get(0)?,
        input: EventTypeInput {
            slug: r.get(1)?,
            title: r.get(2)?,
            description: r.get(3)?,
            duration_min: r.get(4)?,
            buffer_before_min: r.get(5)?,
            buffer_after_min: r.get(6)?,
            min_notice_min: r.get(7)?,
            max_days_ahead: r.get(8)?,
            color: r.get(9)?,
            active: r.get(10)?,
        },
    })
}

impl Db {
    pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

    /// Open (creating if needed) the shared file. WAL lets the app and the
    /// booker read and write it at the same time.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::Invalid(format!("can't create {}: {e}", dir.display())))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    /// `~/.local/share/lyric-cal/cal.db`, or `$LYRIC_CAL_DB` when set.
    pub fn default_path() -> std::path::PathBuf {
        if let Some(p) = std::env::var_os("LYRIC_CAL_DB") {
            return p.into();
        }
        let data = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
        data.join("lyric-cal/cal.db")
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(Db { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    pub fn journal_mode(&self) -> Result<String> {
        Ok(self.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?)
    }

    // ---------- events ----------

    pub fn create_event(&self, input: &EventInput) -> Result<i64> {
        let e = clean_event(input)?;
        let now = now_stamp();
        self.conn.execute(
            "INSERT INTO events (title, notes, start, end, all_day, tz, rrule, color, reminder_minutes, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![e.title, e.notes, e.start, e.end, e.all_day, e.tz, e.rrule, e.color, e.reminder_minutes, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Moving a series or changing its rule drops its exceptions, since they
    /// name occurrences the new schedule may not have.
    pub fn update_event(&self, id: i64, input: &EventInput) -> Result<()> {
        let e = clean_event(input)?;
        let Some(old) = self.event(id)? else { return invalid(format!("no event {id}")) };
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE events SET title = ?2, notes = ?3, start = ?4, end = ?5, all_day = ?6, tz = ?7, rrule = ?8,
                    color = ?9, reminder_minutes = ?10, updated_at = ?11
             WHERE id = ?1",
            params![
                id,
                e.title,
                e.notes,
                e.start,
                e.end,
                e.all_day,
                e.tz,
                e.rrule,
                e.color,
                e.reminder_minutes,
                now_stamp()
            ],
        )?;
        let rescheduled = (&old.start, old.all_day, &old.tz, &old.rrule) != (&e.start, e.all_day, &e.tz, &e.rrule);
        if rescheduled {
            tx.execute("DELETE FROM event_exceptions WHERE event_id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_event(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM events WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn event(&self, id: i64) -> Result<Option<Event>> {
        Ok(self
            .conn
            .query_row(&format!("SELECT {EVENT_COLS} FROM events WHERE id = ?1"), [id], event_row)
            .optional()?)
    }

    /// Events that overlap `[from, to)`, plus every series that has started by
    /// `to` (the caller expands those). Both bounds are stored-format text.
    pub fn events_between(&self, from: &str, to: &str) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {EVENT_COLS} FROM events
             WHERE start < ?2 AND (rrule IS NOT NULL OR end > ?1)
             ORDER BY start, id"
        ))?;
        let rows = stmt.query_map([from, to], event_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every occurrence overlapping `[from, to)` (RFC3339 instants), series
    /// expanded and exceptions applied.
    pub fn occurrences_between(&self, from: &str, to: &str) -> Result<Vec<Occurrence>> {
        let (from, to) = (normalize_instant(from)?, normalize_instant(to)?);
        let events = self.events_between(&from, &to)?;
        let series: Vec<i64> = events.iter().filter(|e| e.rrule.is_some()).map(|e| e.id).collect();
        let exceptions = self.exceptions_for(&series)?;
        let instant = |s: &str| s.parse::<DateTime<Utc>>().expect("normalized above");
        recur::expand(&events, &exceptions, instant(&from), instant(&to))
    }

    // ---------- exceptions ----------

    /// Adds or replaces the exception for one occurrence of a series.
    pub fn set_exception(&self, x: &Exception) -> Result<()> {
        let Some(e) = self.event(x.event_id)? else { return invalid(format!("no event {}", x.event_id)) };
        if e.rrule.is_none() {
            return invalid("only a repeating event has occurrences to change");
        }
        let stored = |s: &str| if e.all_day { parse_day(s).map(|_| s.to_string()) } else { normalize_instant(s) };
        let original = stored(&x.original_start)?;
        if !recur::is_occurrence(&e, &original)? {
            return invalid(format!("{original} is not an occurrence of this event"));
        }
        let (start, end) = match (&x.start, &x.end) {
            (None, None) => (None, None),
            (Some(s), Some(t)) => {
                let (s, t) = (stored(s)?, stored(t)?);
                if t <= s {
                    return invalid("an event must end after it starts");
                }
                (Some(s), Some(t))
            }
            _ => return invalid("a moved occurrence needs both a start and an end"),
        };
        self.conn.execute(
            "INSERT OR REPLACE INTO event_exceptions (event_id, original_start, cancelled, title, notes, start, end)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![x.event_id, original, x.cancelled, x.title.as_deref().map(str::trim), x.notes, start, end],
        )?;
        Ok(())
    }

    pub fn delete_exception(&self, event_id: i64, original_start: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM event_exceptions WHERE event_id = ?1 AND original_start = ?2",
            params![event_id, original_start],
        )?;
        Ok(())
    }

    pub fn exceptions_for(&self, event_ids: &[i64]) -> Result<Vec<Exception>> {
        if event_ids.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; event_ids.len()].join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT event_id, original_start, cancelled, title, notes, start, end FROM event_exceptions
             WHERE event_id IN ({marks}) ORDER BY event_id, original_start"
        ))?;
        let rows = stmt.query_map(params_from_iter(event_ids), |r| {
            Ok(Exception {
                event_id: r.get(0)?,
                original_start: r.get(1)?,
                cancelled: r.get(2)?,
                title: r.get(3)?,
                notes: r.get(4)?,
                start: r.get(5)?,
                end: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---------- event types ----------

    pub fn create_event_type(&self, input: &EventTypeInput) -> Result<i64> {
        let t = clean_event_type(input)?;
        self.conn
            .execute(
                "INSERT INTO event_types (slug, title, description, duration_min, buffer_before_min, buffer_after_min,
                                          min_notice_min, max_days_ahead, color, active)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    t.slug,
                    t.title,
                    t.description,
                    t.duration_min,
                    t.buffer_before_min,
                    t.buffer_after_min,
                    t.min_notice_min,
                    t.max_days_ahead,
                    t.color,
                    t.active
                ],
            )
            .map_err(|e| slug_taken(e, &t.slug))?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_event_type(&self, id: i64, input: &EventTypeInput) -> Result<()> {
        let t = clean_event_type(input)?;
        self.conn
            .execute(
                "UPDATE event_types SET slug = ?2, title = ?3, description = ?4, duration_min = ?5,
                        buffer_before_min = ?6, buffer_after_min = ?7, min_notice_min = ?8, max_days_ahead = ?9,
                        color = ?10, active = ?11
                 WHERE id = ?1",
                params![
                    id,
                    t.slug,
                    t.title,
                    t.description,
                    t.duration_min,
                    t.buffer_before_min,
                    t.buffer_after_min,
                    t.min_notice_min,
                    t.max_days_ahead,
                    t.color,
                    t.active
                ],
            )
            .map_err(|e| slug_taken(e, &t.slug))?;
        Ok(())
    }

    /// Booked event types are deactivated instead, so their bookings keep a home.
    pub fn delete_event_type(&self, id: i64) -> Result<()> {
        let booked: bool =
            self.conn
                .query_row("SELECT EXISTS (SELECT 1 FROM bookings WHERE event_type_id = ?1)", [id], |r| r.get(0))?;
        if booked {
            self.conn.execute("UPDATE event_types SET active = 0 WHERE id = ?1", [id])?;
        } else {
            self.conn.execute("DELETE FROM event_types WHERE id = ?1", [id])?;
        }
        Ok(())
    }

    pub fn event_types(&self) -> Result<Vec<EventType>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {TYPE_COLS} FROM event_types ORDER BY title, id"))?;
        let rows = stmt.query_map([], type_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn event_type_by_slug(&self, slug: &str) -> Result<Option<EventType>> {
        Ok(self
            .conn
            .query_row(&format!("SELECT {TYPE_COLS} FROM event_types WHERE slug = ?1"), [slug], type_row)
            .optional()?)
    }

    // ---------- availability ----------

    pub fn set_weekly_hours(&self, hours: &[WeeklyHours]) -> Result<()> {
        for h in hours {
            if h.weekday > 6 {
                return invalid(format!("no weekday {}", h.weekday));
            }
            check_minutes(h.start_minute, h.end_minute)?;
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM availability", [])?;
        for h in hours {
            tx.execute(
                "INSERT INTO availability (weekday, start_minute, end_minute) VALUES (?1, ?2, ?3)",
                params![h.weekday, h.start_minute, h.end_minute],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn weekly_hours(&self) -> Result<Vec<WeeklyHours>> {
        let mut stmt = self
            .conn
            .prepare("SELECT weekday, start_minute, end_minute FROM availability ORDER BY weekday, start_minute")?;
        let rows = stmt.query_map([], |r| {
            Ok(WeeklyHours { weekday: r.get(0)?, start_minute: r.get(1)?, end_minute: r.get(2)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_overrides(&self, overrides: &[Override]) -> Result<()> {
        for o in overrides {
            parse_day(&o.date)?;
            check_minutes(o.start_minute, o.end_minute)?;
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM date_overrides", [])?;
        for o in overrides {
            tx.execute(
                "INSERT INTO date_overrides (date, start_minute, end_minute) VALUES (?1, ?2, ?3)",
                params![o.date, o.start_minute, o.end_minute],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn overrides(&self) -> Result<Vec<Override>> {
        let mut stmt = self
            .conn
            .prepare("SELECT date, start_minute, end_minute FROM date_overrides ORDER BY date, start_minute")?;
        let rows =
            stmt.query_map([], |r| Ok(Override { date: r.get(0)?, start_minute: r.get(1)?, end_minute: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Weekly hours and overrides in the slot engine's terms.
    pub fn availability(&self) -> Result<Vec<Availability>> {
        let mut out: Vec<Availability> = self
            .weekly_hours()?
            .into_iter()
            .map(|h| {
                Availability::Weekly(WorkingHours {
                    days: vec![h.weekday],
                    start_minute: h.start_minute,
                    end_minute: h.end_minute,
                })
            })
            .collect();
        for o in self.overrides()? {
            out.push(Availability::Override(DateOverride {
                date: parse_day(&o.date)?,
                start_minute: o.start_minute,
                end_minute: o.end_minute,
            }));
        }
        Ok(out)
    }

    // ---------- bookings ----------

    /// Bookings that end after `from`, cancelled ones included, soonest first.
    pub fn bookings_from(&self, from: &str) -> Result<Vec<Booking>> {
        let from = normalize_instant(from)?;
        let mut stmt = self.conn.prepare(
            "SELECT b.id, b.event_type_id, t.title, b.start, b.end, b.guest_name, b.guest_email, b.guest_note,
                    b.guest_tz, b.status, b.created_at
             FROM bookings b JOIN event_types t ON t.id = b.event_type_id
             WHERE b.end > ?1 ORDER BY b.start, b.id",
        )?;
        let rows = stmt.query_map([from], |r| {
            Ok(Booking {
                id: r.get(0)?,
                event_type_id: r.get(1)?,
                type_title: r.get(2)?,
                start: r.get(3)?,
                end: r.get(4)?,
                guest_name: r.get(5)?,
                guest_email: r.get(6)?,
                guest_note: r.get(7)?,
                guest_tz: r.get(8)?,
                status: r.get(9)?,
                created_at: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Marks a booking cancelled and removes its event from the grid, which
    /// frees the slot for the next guest.
    pub fn cancel_booking(&self, id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("UPDATE bookings SET status = 'cancelled' WHERE id = ?1", [id])?;
        tx.execute("DELETE FROM events WHERE booking_id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Timed calendar time between `from` and `to` that guests can't book.
    /// All-day events count as free, as in Google Calendar. A booking also
    /// keeps its own type's gaps before and after it, as in Cal.com.
    pub fn busy_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<DateRange>> {
        let occ = self.occurrences_between(&stamp(from), &stamp(to))?;
        let mut gaps = self.conn.prepare(
            "SELECT t.buffer_before_min, t.buffer_after_min
             FROM bookings b JOIN event_types t ON t.id = b.event_type_id WHERE b.id = ?1",
        )?;
        let instant = |s: &str| DateTime::parse_from_rfc3339(s).map(|t| t.with_timezone(&Utc)).ok();
        let mut out = Vec::new();
        for o in occ.iter().filter(|o| !o.all_day) {
            let (Some(start), Some(end)) = (instant(&o.start), instant(&o.end)) else { continue };
            let (before, after): (i64, i64) = match o.booking_id {
                Some(id) => gaps.query_row([id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?.unwrap_or((0, 0)),
                None => (0, 0),
            };
            out.push(DateRange { start: start - Duration::minutes(before), end: end + Duration::minutes(after) });
        }
        Ok(out)
    }

    /// Start times of `t` a guest in `guest_tz` can book between `from` and `to`.
    pub fn slots_for(
        &self,
        t: &EventType,
        organizer_tz: Tz,
        guest_tz: Tz,
        now: DateTime<Utc>,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<DateTime<Utc>>> {
        let rules = BookingRules {
            duration_min: t.duration_min,
            buffer_before_min: t.buffer_before_min,
            buffer_after_min: t.buffer_after_min,
            min_notice_min: t.min_notice_min,
            max_days_ahead: t.max_days_ahead,
        };
        // Busy blocks just outside the window still reach in through buffers: this
        // type's, and up to 4 h of another booking's own (the editor's limit).
        let pad = Duration::minutes(t.buffer_before_min.max(t.buffer_after_min) + t.duration_min) + Duration::hours(4);
        let busy = self.busy_between(from - pad, to + pad)?;
        Ok(slots::bookable_slots(&self.availability()?, organizer_tz, guest_tz, &rules, &busy, now, from, to))
    }

    /// Books a slot for a guest. Inside one write-locked transaction it checks
    /// that the slot is still offered, then adds the booking and its event, so
    /// two guests racing for one slot get one booking and one [`Error::Taken`].
    pub fn book(&self, req: &BookRequest, organizer_tz: Tz, now: DateTime<Utc>) -> Result<Booking> {
        let (req, start, guest_tz) = clean_request(req)?;
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, rusqlite::TransactionBehavior::Immediate)?;
        let t = match self.event_type_by_slug(&req.slug)? {
            Some(t) if t.active => t,
            _ => return Err(Error::NotFound(format!("no booking link /{}", req.slug))),
        };
        // Slots line up from the start of open hours, so look at the days around it.
        let offered =
            self.slots_for(&t, organizer_tz, guest_tz, now, start - Duration::days(1), start + Duration::days(1))?;
        if !offered.contains(&start) {
            return Err(Error::Taken);
        }
        let end = start + Duration::minutes(t.duration_min);
        let created_at = now_stamp();
        tx.execute(
            "INSERT INTO bookings (event_type_id, start, end, guest_name, guest_email, guest_note, guest_tz, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![t.id, stamp(start), stamp(end), req.guest_name, req.guest_email, req.guest_note, req.guest_tz, created_at],
        )?;
        let id = tx.last_insert_rowid();
        let mut notes = format!("Booked by {} <{}>", req.guest_name, req.guest_email);
        if !req.guest_note.is_empty() {
            notes = format!("{notes}\n\n{}", req.guest_note);
        }
        let event = self.create_event(&EventInput {
            title: format!("{} with {}", t.title, req.guest_name),
            notes,
            start: stamp(start),
            end: stamp(end),
            all_day: false,
            tz: organizer_tz.name().to_string(),
            rrule: None,
            color: t.color.clone(),
            reminder_minutes: Some(10),
        })?;
        tx.execute("UPDATE events SET booking_id = ?1 WHERE id = ?2", [id, event])?;
        tx.commit()?;
        Ok(Booking {
            id,
            event_type_id: t.id,
            type_title: t.title.clone(),
            start: stamp(start),
            end: stamp(end),
            guest_name: req.guest_name,
            guest_email: req.guest_email,
            guest_note: req.guest_note,
            guest_tz: req.guest_tz,
            status: "confirmed".into(),
            created_at,
        })
    }

    /// Reminders whose time falls in `(after, upto]`. All-day events start at
    /// midnight in `tz`. Checking back to back windows fires each one once.
    pub fn due_reminders(&self, after: DateTime<Utc>, upto: DateTime<Utc>, tz: Tz) -> Result<Vec<Reminder>> {
        let occ = self.occurrences_between(&stamp(after), &stamp(upto + Duration::minutes(MAX_REMINDER + 24 * 60)))?;
        let mut out = Vec::new();
        for o in occ {
            let Some(minutes) = o.reminder_minutes else { continue };
            let start = if o.all_day {
                parse_day(&o.start)?
                    .and_hms_opt(0, 0, 0)
                    .and_then(|t| t.and_local_timezone(tz).earliest())
                    .map(|t| t.with_timezone(&Utc))
            } else {
                DateTime::parse_from_rfc3339(&o.start).ok().map(|t| t.with_timezone(&Utc))
            };
            let Some(start) = start else { continue };
            let fire = start - Duration::minutes(minutes);
            if fire > after && fire <= upto {
                out.push(Reminder { title: o.title, start, all_day: o.all_day });
            }
        }
        Ok(out)
    }

    // ---------- settings ----------

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }
}

fn slug_taken(e: rusqlite::Error, slug: &str) -> Error {
    match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
            Error::Invalid(format!("the link name \"{slug}\" is already used"))
        }
        e => e.into(),
    }
}
