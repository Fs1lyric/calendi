//! Booking slots: weekly hours and date overrides become date ranges, busy
//! time is cut out of them, and the rest is split into bookable start times.
//!
//! Ported from Cal.diy's `packages/features/schedules/lib/date-ranges.ts` and
//! `slots.ts` (MIT, see THIRD_PARTY_LICENSES). `tests/caldiy_*.rs` are their
//! test files ported case for case. Dropped on the way: travel schedules,
//! out-of-office and multi-host `intersect`, which a one-person calendar does
//! not have.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// Open on these weekdays (0 = Sunday) between two minutes of the day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingHours {
    pub days: Vec<u32>,
    pub start_minute: u32,
    pub end_minute: u32,
}

/// One date's hours replacing the weekly hours. Start equal to end closes the day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateOverride {
    pub date: NaiveDate,
    pub start_minute: u32,
    pub end_minute: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Weekly(WorkingHours),
    Override(DateOverride),
}

/// Hours can only be set up to 23:59; that means "until midnight".
const LAST_MINUTE: u32 = 23 * 60 + 59;

/// A wall-clock minute on a date in `tz`, as an instant.
fn wall(tz: Tz, date: NaiveDate, minute: u32) -> DateTime<Utc> {
    if minute >= LAST_MINUTE {
        return wall(tz, date.succ_opt().expect("date in range"), 0);
    }
    let naive = date.and_hms_opt(minute / 60, minute % 60, 0).expect("minute of the day");
    resolve(tz, naive)
}

fn resolve(tz: Tz, naive: NaiveDateTime) -> DateTime<Utc> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t.with_timezone(&Utc),
        // Skipped by a spring-forward change: the clocks read an hour later.
        LocalResult::None => resolve(tz, naive + Duration::hours(1)),
    }
}

pub fn process_working_hours(item: &WorkingHours, tz: Tz, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<DateRange> {
    let mut out = Vec::new();
    let mut date = from.with_timezone(&tz).date_naive();
    while wall(tz, date, 0) < to {
        if item.days.contains(&date.weekday().num_days_from_sunday()) {
            let start = wall(tz, date, item.start_minute).max(from);
            let end = wall(tz, date, item.end_minute).min(to);
            if end >= start {
                out.push(DateRange { start, end });
            }
        }
        date = date.succ_opt().expect("date in range");
    }
    out
}

pub fn process_date_override(item: &DateOverride, tz: Tz) -> DateRange {
    DateRange { start: wall(tz, item.date, item.start_minute), end: wall(tz, item.date, item.end_minute) }
}

/// Sort and join ranges that overlap or touch, so hours that run past
/// midnight into the next day's hours become one range.
fn merge(mut ranges: Vec<DateRange>) -> Vec<DateRange> {
    ranges.sort_by_key(|r| r.start);
    let mut out: Vec<DateRange> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// The organizer's open time between `from` and `to`. A date with any
/// override uses only its overrides; overrides are not cut to the window.
pub fn build_date_ranges(
    availability: &[Availability],
    tz: Tz,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<DateRange> {
    let mut weekly = Vec::new();
    let mut overrides = Vec::new();
    let mut overridden = HashSet::new();
    let (first, last) = (from.date_naive() - Duration::days(1), to.date_naive() + Duration::days(1));
    for item in availability {
        match item {
            Availability::Weekly(w) => weekly.extend(process_working_hours(w, tz, from, to)),
            Availability::Override(o) if o.date >= first && o.date <= last => {
                overridden.insert(o.date);
                let r = process_date_override(o, tz);
                if r.start < r.end {
                    overrides.push(r);
                }
            }
            Availability::Override(_) => {}
        }
    }
    let local_day = |r: &DateRange| r.start.with_timezone(&tz).date_naive();
    let mut out: Vec<DateRange> =
        merge(weekly).into_iter().filter(|r| r.start < r.end && !overridden.contains(&local_day(r))).collect();
    out.extend(merge(overrides));
    out.sort_by_key(|r| r.start);
    out
}

pub fn subtract(source: &[DateRange], excluded: &[DateRange]) -> Vec<DateRange> {
    let mut excluded = excluded.to_vec();
    excluded.sort_by_key(|r| r.start);
    let mut out = Vec::new();
    for s in source {
        let mut current = s.start;
        for x in &excluded {
            if x.start >= s.end {
                break;
            }
            if x.end <= current {
                continue;
            }
            if x.start > current {
                out.push(DateRange { start: current, end: x.start });
            }
            current = current.max(x.end);
        }
        if s.end > current {
            out.push(DateRange { start: current, end: s.end });
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct SlotOptions {
    pub now: DateTime<Utc>,
    /// The zone slots are aligned in (the guest's).
    pub time_zone: Tz,
    /// Minutes between slot starts.
    pub frequency: i64,
    pub event_length: i64,
    pub minimum_booking_notice: i64,
    pub offset_start: i64,
    /// Fit the most slots into a range instead of starting them on the hour.
    pub show_optimized_slots: bool,
    /// Cal.diy's `NEXT_PUBLIC_AVAILABILITY_SCHEDULE_INTERVAL`; 1 when unset.
    pub interval: Option<i64>,
}

fn corrected_start(t: DateTime<Tz>, range: &DateRange, interval: i64, optimized: bool) -> DateTime<Tz> {
    let minute = t.minute() as i64;
    if !optimized {
        let up = (minute + interval - 1) / interval * interval;
        return t + Duration::minutes(up - minute);
    }
    // Keep the most slots, but spend leftover minutes moving the first one
    // onto the hour, else the quarter hour, else the next 5 minutes.
    let extra = (range.end - t.with_timezone(&Utc)).num_minutes() % interval;
    for step in [interval, 15, 5] {
        let needed = step - minute % step;
        if extra >= needed {
            return t + Duration::minutes(needed);
        }
    }
    t
}

pub fn get_slots(ranges: &[DateRange], o: &SlotOptions) -> Vec<DateTime<Tz>> {
    let frequency = o.frequency.max(1);
    let event_length = o.event_length.max(1);
    let offset = if o.offset_start > 0 { o.offset_start } else { 0 };
    let step = Duration::minutes(frequency + offset);
    let interval = [60, 30, 20, 15, 10, 5].into_iter().find(|i| frequency % i == 0).unwrap_or(o.interval.unwrap_or(1));

    let mut ordered = ranges.to_vec();
    ordered.sort_by_key(|r| r.start);
    let earliest = o.now + Duration::minutes(o.minimum_booking_notice);

    let mut slots = Vec::new();
    let mut seen = HashSet::new();
    let mut boundaries: BTreeSet<DateTime<Utc>> = BTreeSet::new();
    for range in &ordered {
        let start = if range.start > earliest { range.start } else { earliest };
        let start = start.with_second(0).and_then(|t| t.with_nanosecond(0)).expect("valid time");
        let mut t = start.with_timezone(&o.time_zone);
        if t.minute() as i64 % interval != 0 {
            t = corrected_start(t, range, interval, o.show_optimized_slots);
        }
        t += Duration::minutes(offset);

        // Stay on the grid of the slot this range starts inside of.
        if let Some(&prev) = boundaries.range(..t.with_timezone(&Utc)).next_back() {
            let prev_end = prev + step;
            if prev_end > t.with_timezone(&Utc) {
                t = (if prev >= range.start { prev } else { prev_end }).with_timezone(&o.time_zone);
            }
        }

        while t + Duration::minutes(event_length) - Duration::seconds(1) <= range.end {
            let at = t.with_timezone(&Utc);
            if seen.insert(at) {
                boundaries.insert(at);
                slots.push(t);
            }
            t += step;
        }
    }
    slots
}

/// What an event type asks of the calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingRules {
    pub duration_min: i64,
    pub buffer_before_min: i64,
    pub buffer_after_min: i64,
    pub min_notice_min: i64,
    pub max_days_ahead: i64,
}

/// Start times a guest can book between `from` and `to`, given the organizer's
/// hours and everything already on the calendar.
#[allow(clippy::too_many_arguments)] // one call site, in the booker; a struct would only rename them
pub fn bookable_slots(
    availability: &[Availability],
    organizer_tz: Tz,
    guest_tz: Tz,
    rules: &BookingRules,
    busy: &[DateRange],
    now: DateTime<Utc>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<DateTime<Utc>> {
    let to = to.min(now + Duration::days(rules.max_days_ahead));
    if to <= from {
        return Vec::new();
    }
    let open = build_date_ranges(availability, organizer_tz, from, to);
    // A slot needs its buffers free too, so grow each busy block by them:
    // the after-buffer reaches back before it, the before-buffer past it.
    let padded: Vec<DateRange> = busy
        .iter()
        .map(|b| DateRange {
            start: b.start - Duration::minutes(rules.buffer_after_min),
            end: b.end + Duration::minutes(rules.buffer_before_min),
        })
        .collect();
    let free = subtract(&open, &padded);
    let opts = SlotOptions {
        now,
        time_zone: guest_tz,
        frequency: rules.duration_min,
        event_length: rules.duration_min,
        minimum_booking_notice: rules.min_notice_min,
        offset_start: 0,
        show_optimized_slots: false,
        interval: None,
    };
    get_slots(&free, &opts).into_iter().map(|t| t.with_timezone(&Utc)).filter(|t| *t >= from && *t < to).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn rules() -> BookingRules {
        BookingRules {
            duration_min: 30,
            buffer_before_min: 0,
            buffer_after_min: 0,
            min_notice_min: 0,
            max_days_ahead: 60,
        }
    }

    fn nine_to_noon() -> Vec<Availability> {
        vec![Availability::Weekly(WorkingHours {
            days: vec![1, 2, 3, 4, 5],
            start_minute: 9 * 60,
            end_minute: 12 * 60,
        })]
    }

    fn hhmm(slots: &[DateTime<Utc>]) -> Vec<String> {
        slots.iter().map(|t| t.format("%H:%M").to_string()).collect()
    }

    // 2026-10-19 is a Monday.
    const NOW: &str = "2026-10-15T00:00:00Z";
    const FROM: &str = "2026-10-19T00:00:00Z";
    const TO: &str = "2026-10-20T00:00:00Z";

    #[test]
    fn busy_time_removes_the_slots_it_touches() {
        let busy = [DateRange { start: utc("2026-10-19T10:00:00Z"), end: utc("2026-10-19T10:45:00Z") }];
        let got = bookable_slots(&nine_to_noon(), Tz::UTC, Tz::UTC, &rules(), &busy, utc(NOW), utc(FROM), utc(TO));
        assert_eq!(hhmm(&got), ["09:00", "09:30", "11:00", "11:30"]);
    }

    #[test]
    fn buffers_keep_room_around_existing_events() {
        let busy = [DateRange { start: utc("2026-10-19T10:00:00Z"), end: utc("2026-10-19T10:30:00Z") }];
        let r = BookingRules { buffer_before_min: 15, buffer_after_min: 15, ..rules() };
        let got = bookable_slots(&nine_to_noon(), Tz::UTC, Tz::UTC, &r, &busy, utc(NOW), utc(FROM), utc(TO));
        // 09:30 would end at 10:00, its after-buffer runs into the event; 10:30's before-buffer does too.
        assert_eq!(hhmm(&got), ["09:00", "11:00", "11:30"]);
    }

    #[test]
    fn nothing_past_the_booking_horizon() {
        let r = BookingRules { max_days_ahead: 3, ..rules() };
        assert!(bookable_slots(&nine_to_noon(), Tz::UTC, Tz::UTC, &r, &[], utc(NOW), utc(FROM), utc(TO)).is_empty());
    }

    #[test]
    fn hours_are_read_in_the_organizer_zone() {
        let got = bookable_slots(
            &nine_to_noon(),
            chrono_tz::Asia::Dhaka,
            Tz::UTC,
            &rules(),
            &[],
            utc(NOW),
            utc(FROM),
            utc(TO),
        );
        assert_eq!(hhmm(&got).first().unwrap(), "03:00");
        assert_eq!(got.len(), 6);
    }
}
