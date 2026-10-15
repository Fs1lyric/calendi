//! The calendar's geometry, kept free of drawing so it can be tested: which
//! days a view shows, the query window, where an occurrence falls in local
//! time, and how overlapping events share a day column.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use lyric_cal_core::recur::Occurrence;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Month,
    Week,
    Day,
}

/// Weeks start on Monday.
pub fn week_start(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

/// The days on screen: six full weeks for a month, seven or one otherwise.
pub fn days(view: View, anchor: NaiveDate) -> Vec<NaiveDate> {
    let (first, count) = match view {
        View::Month => (week_start(anchor.with_day(1).expect("day 1 exists")), 42),
        View::Week => (week_start(anchor), 7),
        View::Day => (anchor, 1),
    };
    first.iter_days().take(count).collect()
}

/// The anchor `by` months, weeks or days away. A month step keeps the day of
/// the month where it can (31 Jan → 28 Feb).
pub fn step(view: View, anchor: NaiveDate, by: i32) -> NaiveDate {
    match view {
        View::Month => {
            let months = chrono::Months::new(by.unsigned_abs());
            if by >= 0 {
                anchor + months
            } else {
                anchor - months
            }
        }
        View::Week => anchor + Duration::weeks(by as i64),
        View::Day => anchor + Duration::days(by as i64),
    }
}

pub fn title(view: View, anchor: NaiveDate) -> String {
    match view {
        View::Month => anchor.format("%B %Y").to_string(),
        View::Day => anchor.format("%A %-d %B %Y").to_string(),
        View::Week => {
            let (a, b) = (week_start(anchor), week_start(anchor) + Duration::days(6));
            let left = if a.year() != b.year() {
                "%-d %b %Y"
            } else if a.month() != b.month() {
                "%-d %b"
            } else {
                "%-d"
            };
            format!("{} – {}", a.format(left), b.format("%-d %b %Y"))
        }
    }
}

/// Local midnight as an instant. A zone that skips midnight starts the day an hour later.
pub fn midnight(day: NaiveDate, tz: Tz) -> DateTime<Utc> {
    let at = |h| tz.from_local_datetime(&day.and_hms_opt(h, 0, 0).expect("valid hour")).earliest();
    at(0).or_else(|| at(1)).expect("a day has a first hour").with_timezone(&Utc)
}

/// From the first day's midnight to the midnight after the last day.
pub fn window(days: &[NaiveDate], tz: Tz) -> (DateTime<Utc>, DateTime<Utc>) {
    let (first, last) = (days[0], days[days.len() - 1]);
    (midnight(first, tz), midnight(last + Duration::days(1), tz))
}

/// A stored timed value (UTC RFC3339) in local wall-clock time.
pub fn local(stamp: &str, tz: Tz) -> NaiveDateTime {
    stamp.parse::<DateTime<Utc>>().map(|t| t.with_timezone(&tz).naive_local()).unwrap_or_default()
}

/// A local wall-clock time as a stored UTC value.
pub fn stamp(t: NaiveDateTime, tz: Tz) -> String {
    // A wall-clock time skipped by DST lands on the first valid one after it.
    let mut t = t;
    loop {
        if let Some(at) = tz.from_local_datetime(&t).earliest() {
            return at.with_timezone(&Utc).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        }
        t += Duration::minutes(15);
    }
}

/// Whether an all-day occurrence covers `day` (its end date is exclusive).
pub fn covers(o: &Occurrence, day: NaiveDate) -> bool {
    let date = |s: &str| s.parse::<NaiveDate>().ok();
    matches!((date(&o.start), date(&o.end)), (Some(a), Some(b)) if a <= day && day < b)
}

/// A timed occurrence's minutes on `day`, clipped to the day, if it touches it.
pub fn span_on(o: &Occurrence, day: NaiveDate, tz: Tz) -> Option<(i64, i64)> {
    if o.all_day {
        return None;
    }
    let base = day.and_hms_opt(0, 0, 0).expect("midnight exists");
    let minutes = |s: &str| (local(s, tz) - base).num_minutes();
    let (a, b) = (minutes(&o.start), minutes(&o.end));
    (a < 24 * 60 && b > 0).then(|| (a.max(0), b.min(24 * 60)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub column: usize,
    pub columns: usize,
}

/// Lays out spans (start, end minutes) side by side: each gets the first free
/// column, and every span in an overlapping cluster shares its column count.
pub fn pack(spans: &[(i64, i64)]) -> Vec<Placed> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| (spans[i].0, -spans[i].1));
    let mut out = vec![Placed { column: 0, columns: 1 }; spans.len()];
    let mut cluster: Vec<usize> = Vec::new();
    let mut column_ends: Vec<i64> = Vec::new();
    let mut cluster_end = i64::MIN;
    let close = |cluster: &mut Vec<usize>, columns: usize, out: &mut Vec<Placed>| {
        for i in cluster.drain(..) {
            out[i].columns = columns;
        }
    };
    for i in order {
        let (start, end) = spans[i];
        if start >= cluster_end {
            close(&mut cluster, column_ends.len(), &mut out);
            column_ends.clear();
        }
        let column = match column_ends.iter().position(|&e| e <= start) {
            Some(c) => c,
            None => {
                column_ends.push(i64::MIN);
                column_ends.len() - 1
            }
        };
        column_ends[column] = end;
        out[i].column = column;
        cluster.push(i);
        cluster_end = cluster_end.max(end);
    }
    close(&mut cluster, column_ends.len(), &mut out);
    out
}

/// The minute under `y` pixels from the top of a day column, snapped down to `snap`.
pub fn minute_at(y: f32, hour_px: f32, snap: i64) -> i64 {
    let minute = (y / hour_px * 60.0).floor() as i64;
    (minute.div_euclid(snap) * snap).clamp(0, 24 * 60 - snap)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn occ(start: &str, end: &str) -> Occurrence {
        Occurrence {
            event_id: 1,
            title: String::new(),
            notes: String::new(),
            start: start.into(),
            end: end.into(),
            all_day: !start.contains('T'),
            color: None,
            reminder_minutes: None,
            booking_id: None,
            original_start: None,
        }
    }

    const DHAKA: Tz = chrono_tz::Asia::Dhaka;

    #[test]
    fn weeks_start_on_monday() {
        assert_eq!(week_start(d("2026-10-15")), d("2026-10-12"));
        assert_eq!(week_start(d("2026-10-12")), d("2026-10-12"));
        assert_eq!(week_start(d("2026-10-18")), d("2026-10-12"));
    }

    #[test]
    fn views_show_the_right_days() {
        let month = days(View::Month, d("2026-10-15"));
        assert_eq!(month.len(), 42);
        assert_eq!(month[0], d("2026-09-28"));
        assert_eq!(month[41], d("2026-11-08"));
        assert_eq!(
            days(View::Week, d("2026-10-15")),
            (12..=18).map(|n| d(&format!("2026-10-{n}"))).collect::<Vec<_>>()
        );
        assert_eq!(days(View::Day, d("2026-10-15")), [d("2026-10-15")]);
    }

    #[test]
    fn stepping_moves_by_the_views_unit() {
        assert_eq!(step(View::Month, d("2026-01-31"), 1), d("2026-02-28"));
        assert_eq!(step(View::Month, d("2026-01-15"), -1), d("2025-12-15"));
        assert_eq!(step(View::Week, d("2026-10-15"), 1), d("2026-10-22"));
        assert_eq!(step(View::Day, d("2026-10-31"), 1), d("2026-11-01"));
    }

    #[test]
    fn titles_name_the_period() {
        assert_eq!(title(View::Month, d("2026-10-15")), "October 2026");
        assert_eq!(title(View::Week, d("2026-10-15")), "12 – 18 Oct 2026");
        assert_eq!(title(View::Week, d("2026-09-30")), "28 Sep – 4 Oct 2026");
        assert_eq!(title(View::Week, d("2026-12-30")), "28 Dec 2026 – 3 Jan 2027");
        assert_eq!(title(View::Day, d("2026-10-15")), "Thursday 15 October 2026");
    }

    #[test]
    fn the_window_runs_local_midnight_to_midnight() {
        let (from, to) = window(&days(View::Week, d("2026-10-15")), DHAKA);
        assert_eq!(from.to_rfc3339(), "2026-10-11T18:00:00+00:00");
        assert_eq!(to.to_rfc3339(), "2026-10-18T18:00:00+00:00");
    }

    #[test]
    fn stamps_convert_both_ways() {
        let t = local("2026-10-15T03:30:00Z", DHAKA);
        assert_eq!(t.to_string(), "2026-10-15 09:30:00");
        assert_eq!(stamp(t, DHAKA), "2026-10-15T03:30:00Z");
    }

    #[test]
    fn all_day_coverage_excludes_the_end_date() {
        let o = occ("2026-10-15", "2026-10-17");
        assert!(!covers(&o, d("2026-10-14")));
        assert!(covers(&o, d("2026-10-15")));
        assert!(covers(&o, d("2026-10-16")));
        assert!(!covers(&o, d("2026-10-17")));
    }

    #[test]
    fn timed_spans_are_clipped_to_each_day() {
        // 22:00 on the 15th to 02:00 on the 16th, Dhaka time.
        let o = occ("2026-10-15T16:00:00Z", "2026-10-15T20:00:00Z");
        assert_eq!(span_on(&o, d("2026-10-15"), DHAKA), Some((22 * 60, 24 * 60)));
        assert_eq!(span_on(&o, d("2026-10-16"), DHAKA), Some((0, 2 * 60)));
        assert_eq!(span_on(&o, d("2026-10-17"), DHAKA), None);
        // Ending exactly at midnight does not touch the next day.
        let o = occ("2026-10-15T17:00:00Z", "2026-10-15T18:00:00Z");
        assert_eq!(span_on(&o, d("2026-10-16"), DHAKA), None);
    }

    #[test]
    fn overlapping_spans_share_the_width() {
        let p = |column, columns| Placed { column, columns };
        // A and B overlap, C fits under A after it ends, D is alone.
        let spans = [(540, 600), (570, 660), (600, 630), (720, 780)];
        assert_eq!(pack(&spans), [p(0, 2), p(1, 2), p(0, 2), p(0, 1)]);
        assert_eq!(pack(&[]), []);
        // Touching end to start is not an overlap.
        assert_eq!(pack(&[(540, 600), (600, 660)]), [p(0, 1), p(0, 1)]);
    }

    #[test]
    fn pixels_map_to_snapped_minutes() {
        assert_eq!(minute_at(0.0, 48.0, 15), 0);
        assert_eq!(minute_at(48.0 * 9.5, 48.0, 15), 570);
        assert_eq!(minute_at(48.0 * 9.4, 48.0, 15), 555);
        assert_eq!(minute_at(-10.0, 48.0, 15), 0);
        assert_eq!(minute_at(48.0 * 30.0, 48.0, 15), 24 * 60 - 15);
    }
}
