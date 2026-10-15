//! Repeating events: turns stored events, their RFC 5545 rules and their
//! one-off exceptions into the concrete occurrences a view shows.
//!
//! A timed series repeats at the same wall-clock time in the event's `tz`, so
//! a 09:00 meeting stays at 09:00 across a DST change. An all-day series steps
//! by date. Each occurrence is named by its `original_start`, the start the
//! rule gives it, which is what an exception refers to even after a move.

use crate::db::{Error, Event, EventInput, Exception, Result};
use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use rrule::{Frequency, RRule, RRuleSet, Unvalidated};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Occurrence {
    pub event_id: i64,
    pub title: String,
    pub notes: String,
    /// Stored format: UTC RFC3339, or `YYYY-MM-DD` with an exclusive end.
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub color: Option<String>,
    pub reminder_minutes: Option<i64>,
    pub booking_id: Option<i64>,
    /// Set for a series occurrence; names it when editing or cancelling just this one.
    pub original_start: Option<String>,
}

fn bad_rule(rule: &str, why: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("bad repeat rule {rule}: {why}"))
}

fn zone(e: &EventInput) -> Result<Tz> {
    e.tz.parse().map_err(|_| Error::Invalid(format!("unknown time zone: {}", e.tz)))
}

fn day(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| Error::Invalid(format!("not a date: {s}")))
}

/// The instant a stored value stands for. All-day dates start at midnight in
/// the event's zone.
fn instant(s: &str, all_day: bool, tz: Tz) -> Result<DateTime<Utc>> {
    if !all_day {
        return s.parse().map_err(|_| Error::Invalid(format!("not a date and time: {s}")));
    }
    let midnight = day(s)?.and_hms_opt(0, 0, 0).expect("midnight exists");
    // A zone that skips midnight (a DST jump) starts the day at the first hour it has.
    tz.from_local_datetime(&midnight)
        .earliest()
        .or_else(|| tz.from_local_datetime(&(midnight + Duration::hours(1))).earliest())
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(|| Error::Invalid(format!("{s} has no midnight in {tz}")))
}

fn stamp(t: DateTime<rrule::Tz>, all_day: bool) -> String {
    if all_day {
        t.date_naive().format("%Y-%m-%d").to_string()
    } else {
        t.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Secs, true)
    }
}

/// An all-day series runs in UTC on dates alone; a timed one in its own zone.
fn rule_start(e: &EventInput) -> Result<DateTime<rrule::Tz>> {
    if e.all_day {
        let midnight = day(&e.start)?.and_hms_opt(0, 0, 0).expect("midnight exists");
        return Ok(rrule::Tz::UTC.from_utc_datetime(&midnight));
    }
    let tz = rrule::Tz::from(zone(e)?);
    Ok(instant(&e.start, false, Tz::UTC)?.with_timezone(&tz))
}

/// A date-only `UNTIL` (what RFC 5545 asks of an all-day series) means through
/// the end of that day. The rule engine wants it in UTC, so rewrite it.
fn until_in_utc(e: &EventInput, rule: &str) -> Result<String> {
    let parts = rule.split(';').map(|part| match part.strip_prefix("UNTIL=") {
        Some(v) if v.len() == 8 => {
            let date = NaiveDate::parse_from_str(v, "%Y%m%d").map_err(|err| bad_rule(rule, err))?;
            let next = (date + Duration::days(1)).format("%Y-%m-%d").to_string();
            let tz = if e.all_day { Tz::UTC } else { zone(e)? };
            let last = instant(&next, true, tz)? - Duration::seconds(1);
            Ok(last.format("UNTIL=%Y%m%dT%H%M%SZ").to_string())
        }
        _ => Ok(part.to_string()),
    });
    Ok(parts.collect::<Result<Vec<_>>>()?.join(";"))
}

fn rule_set(e: &EventInput, rule: &str) -> Result<RRuleSet> {
    let parsed: RRule<Unvalidated> = until_in_utc(e, rule)?.parse().map_err(|err| bad_rule(rule, err))?;
    if matches!(parsed.get_freq(), Frequency::Hourly | Frequency::Minutely | Frequency::Secondly) {
        return Err(bad_rule(rule, "repeat at most daily"));
    }
    if parsed.get_count() == Some(0) {
        return Err(bad_rule(rule, "COUNT must be at least 1"));
    }
    parsed.build(rule_start(e)?).map_err(|err| bad_rule(rule, err))
}

/// Checks a (cleaned) event's rule against its start, the way `expand` will run it.
pub fn check_rule(e: &EventInput) -> Result<()> {
    match &e.rrule {
        Some(rule) => rule_set(e, rule).map(drop),
        None => Ok(()),
    }
}

/// Starts the series gives inside `[from, to]`, both in stored format.
fn rule_starts(e: &EventInput, rule: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<String>> {
    let set = rule_set(e, rule)?;
    let tz = set.get_dt_start().timezone();
    let found = set.after(from.with_timezone(&tz)).before(to.with_timezone(&tz)).all(u16::MAX);
    Ok(found.dates.into_iter().map(|t| stamp(t, e.all_day)).collect())
}

/// Whether `original` (stored format) is one of the series' own starts.
pub fn is_occurrence(e: &EventInput, original: &str) -> Result<bool> {
    let Some(rule) = &e.rrule else { return Ok(false) };
    let at = instant(original, e.all_day, Tz::UTC)?;
    Ok(rule_starts(e, rule, at, at)?.iter().any(|s| s == original))
}

/// How long one occurrence lasts: whole days for all-day, else exact time.
fn length(e: &EventInput, tz: Tz) -> Result<Duration> {
    Ok(instant(&e.end, e.all_day, tz)? - instant(&e.start, e.all_day, tz)?)
}

/// The end of an occurrence starting at `start` (stored format).
fn end_after(e: &EventInput, tz: Tz, start: &str) -> Result<String> {
    let length = length(e, tz)?;
    Ok(if e.all_day {
        (day(start)? + Duration::days(length.num_days())).format("%Y-%m-%d").to_string()
    } else {
        (instant(start, false, tz)? + length).to_rfc3339_opts(SecondsFormat::Secs, true)
    })
}

struct Draft {
    sort_key: DateTime<Utc>,
    occ: Occurrence,
}

/// Every occurrence that overlaps `[from, to)`, sorted by start. `events` is
/// what `Db::events_between` returns; `exceptions` are those of its series.
pub fn expand(
    events: &[Event],
    exceptions: &[Exception],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<Occurrence>> {
    let by_occurrence: HashMap<(i64, &str), &Exception> =
        exceptions.iter().map(|x| ((x.event_id, x.original_start.as_str()), x)).collect();
    let mut out = Vec::new();
    let mut push = |e: &Event, start: &str, end: &str, original: Option<&str>, x: Option<&Exception>| -> Result<()> {
        let tz = zone(e)?;
        let (s, t) = (instant(start, e.all_day, tz)?, instant(end, e.all_day, tz)?);
        if s < to && t > from {
            out.push(Draft {
                sort_key: s,
                occ: Occurrence {
                    event_id: e.id,
                    title: x.and_then(|x| x.title.clone()).unwrap_or_else(|| e.title.clone()),
                    notes: x.and_then(|x| x.notes.clone()).unwrap_or_else(|| e.notes.clone()),
                    start: start.into(),
                    end: end.into(),
                    all_day: e.all_day,
                    color: e.color.clone(),
                    reminder_minutes: e.reminder_minutes,
                    booking_id: e.booking_id,
                    original_start: original.map(Into::into),
                },
            });
        }
        Ok(())
    };

    for e in events {
        let Some(rule) = &e.rrule else {
            push(e, &e.start, &e.end, None, None)?;
            continue;
        };
        let tz = zone(e)?;
        let length = length(e, tz)?;
        // A day of slack either side covers all-day dates, which the rule runs in UTC.
        let starts = rule_starts(e, rule, from - length - Duration::days(1), to + Duration::days(1))?;
        for start in starts {
            if by_occurrence.contains_key(&(e.id, start.as_str())) {
                continue; // placed below, wherever it was moved to
            }
            let end = end_after(e, tz, &start)?;
            push(e, &start, &end, Some(&start), None)?;
        }
    }

    let series: HashMap<i64, &Event> = events.iter().filter(|e| e.rrule.is_some()).map(|e| (e.id, e)).collect();
    for x in exceptions.iter().filter(|x| !x.cancelled) {
        let Some(e) = series.get(&x.event_id) else { continue };
        let (start, end) = match (&x.start, &x.end) {
            (Some(s), Some(t)) => (s.clone(), t.clone()),
            _ => (x.original_start.clone(), end_after(e, zone(e)?, &x.original_start)?),
        };
        push(e, &start, &end, Some(&x.original_start), Some(x))?;
    }

    out.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.occ.event_id.cmp(&b.occ.event_id)));
    Ok(out.into_iter().map(|d| d.occ).collect())
}
