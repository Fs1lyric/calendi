//! Cal.diy's `date-ranges.test.ts`, ported (MIT, see THIRD_PARTY_LICENSES).
//!
//! Left out: travel schedules, out-of-office and `intersect` (team scheduling),
//! none of which Lyric Cal has, plus the two cases skipped upstream.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use lyric_cal_core::slots::{
    build_date_ranges, process_date_override, process_working_hours, subtract, Availability, DateOverride, DateRange,
    WorkingHours,
};

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn day(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn range(a: &str, b: &str) -> DateRange {
    DateRange { start: utc(a), end: utc(b) }
}

fn weekly(days: &[u32], start: u32, end: u32) -> WorkingHours {
    WorkingHours { days: days.to_vec(), start_minute: start, end_minute: end }
}

fn on(date: &str, start: u32, end: u32) -> DateOverride {
    DateOverride { date: day(date), start_minute: start, end_minute: end }
}

const H: u32 = 60;
/// Cal.diy's "end of day" is 23:59.
const EOD: u32 = 23 * H + 59;

// ---------- processWorkingHours ----------

#[test]
fn has_availability_on_the_last_day_of_the_month_dst_ends() {
    let item = weekly(&[0, 1, 2, 3, 4, 5, 6], 8 * H, 17 * H);
    let results = process_working_hours(
        &item,
        chrono_tz::Europe::London,
        utc("2023-10-24T00:00:00Z"),
        utc("2023-11-01T00:00:00Z"),
    );
    let last = results.last().unwrap();
    assert_eq!(last.start.with_timezone(&chrono_tz::Europe::London).format("%d").to_string(), "31");
}

#[test]
fn has_the_correct_working_hours_on_the_date_of_a_dst_change() {
    // Now is 2023-11-05T13:26:14Z, the Sunday New York left DST.
    let item = weekly(&[1, 2, 3, 4, 5], 9 * H, 17 * H);
    let results = process_working_hours(
        &item,
        chrono_tz::America::New_York,
        utc("2023-11-05T13:26:14Z"),
        utc("2023-11-30T23:59:59.999Z"),
    );
    let weekdays: Vec<u32> = (6..=30)
        .filter(|d| !matches!(d % 7, 4 | 5)) // drop Sat 11, 18, 25 and Sun 12, 19, 26
        .collect();
    let want: Vec<DateRange> = weekdays
        .iter()
        .map(|d| range(&format!("2023-11-{d:02}T14:00:00Z"), &format!("2023-11-{d:02}T22:00:00Z")))
        .collect();
    assert_eq!(results, want);
}

#[test]
fn skips_hours_that_end_before_they_start_across_days() {
    let item = weekly(&[1, 2, 3], 8 * H, 7 * H);
    let tz = chrono_tz::America::New_York;
    assert!(process_working_hours(&item, tz, utc("2023-11-07T00:00:00Z"), utc("2023-11-08T00:00:00Z")).is_empty());
}

#[test]
fn skips_hours_that_end_before_they_start_on_the_same_day() {
    let item = weekly(&[1], 8 * H, 7 * H);
    let tz = chrono_tz::America::New_York;
    assert!(process_working_hours(&item, tz, utc("2023-11-07T00:00:00Z"), utc("2023-11-07T23:59:59Z")).is_empty());
}

// ---------- processDateOverride ----------

#[test]
fn processes_a_date_override_in_the_organizer_timezone() {
    let r = process_date_override(&on("2023-06-12", 8 * H, 17 * H), chrono_tz::America::New_York);
    assert_eq!(r, range("2023-06-12T12:00:00Z", "2023-06-12T21:00:00Z"));
}

// ---------- buildDateRanges ----------

fn build(items: Vec<Availability>, tz: Tz, from: &str, to: &str) -> Vec<DateRange> {
    build_date_ranges(&items, tz, utc(from), utc(to))
}

#[test]
fn returns_the_correct_date_ranges() {
    let items = vec![
        Availability::Override(on("2023-06-13", 10 * H, 15 * H)),
        Availability::Weekly(weekly(&[1, 2, 3, 4, 5], 8 * H, 17 * H)),
    ];
    let results = build(items, chrono_tz::America::New_York, "2023-06-13T00:00:00Z", "2023-06-15T00:00:00Z");
    assert_eq!(
        results,
        [range("2023-06-13T14:00:00Z", "2023-06-13T19:00:00Z"), range("2023-06-14T12:00:00Z", "2023-06-14T21:00:00Z")]
    );
}

#[test]
fn handles_day_shifts_correctly() {
    let items = vec![
        Availability::Override(on("2023-08-15", 9 * H, 17 * H)),
        Availability::Override(on("2023-08-15", 19 * H, 21 * H)),
    ];
    // dateFrom/dateTo are 2023-08-15 in Brussels.
    let results = build(items, chrono_tz::Pacific::Honolulu, "2023-08-14T22:00:00Z", "2023-08-15T21:59:59.999Z");
    assert_ne!(results[0].end, utc("2023-08-14T17:00:00-10:00"));
}

#[test]
fn a_zero_length_override_makes_the_whole_day_unavailable() {
    let items = vec![
        Availability::Override(on("2023-06-13", 0, 0)),
        Availability::Weekly(weekly(&[1, 2, 3, 4, 5], 8 * H, 17 * H)),
    ];
    let results = build(items, chrono_tz::Europe::London, "2023-06-13T00:00:00Z", "2023-06-15T00:00:00Z");
    assert_eq!(results[0], range("2023-06-14T07:00:00Z", "2023-06-14T16:00:00Z"));
}

#[test]
fn a_date_override_is_not_cut_to_the_requested_window() {
    let items = vec![Availability::Override(on("2023-06-13", 9 * H, 14 * H))];
    let results = build(items, chrono_tz::Europe::London, "2023-06-13T10:00:00Z", "2023-06-13T10:30:00Z");
    assert_eq!(results[0], range("2023-06-13T08:00:00Z", "2023-06-13T13:00:00Z"));
}

#[test]
fn a_date_override_that_is_already_the_next_day_in_utc() {
    let items = vec![
        Availability::Override(on("2023-06-13", 22 * H, 23 * H)),
        Availability::Weekly(weekly(&[1, 2, 3, 4, 5], 8 * H, 17 * H)),
    ];
    let results = build(items, chrono_tz::America::New_York, "2023-06-13T00:00:00Z", "2023-06-15T00:00:00Z");
    assert_eq!(results[0], range("2023-06-14T02:00:00Z", "2023-06-14T03:00:00Z"));
    assert_eq!(results[1], range("2023-06-14T12:00:00Z", "2023-06-14T21:00:00Z"));
}

#[test]
fn availability_past_midnight_merges_adjacent_ranges() {
    let items = vec![
        Availability::Weekly(weekly(&[1, 2, 3, 4, 5], 23 * H, EOD)),
        Availability::Weekly(weekly(&[2, 3, 4, 5, 6], 0, 30)),
    ];
    let results = build(items, chrono_tz::Europe::London, "2023-06-13T00:00:00Z", "2023-06-15T00:00:00Z");
    assert_eq!(
        results,
        [range("2023-06-13T22:00:00Z", "2023-06-13T23:30:00Z"), range("2023-06-14T22:00:00Z", "2023-06-14T23:30:00Z")]
    );
}

#[test]
fn multi_day_availability_past_midnight_merges_adjacent_ranges() {
    let items = vec![Availability::Weekly(weekly(&[1, 2], 0, EOD))];
    let results = build(items, chrono_tz::Europe::London, "2023-06-11T00:00:00Z", "2023-06-14T00:00:00Z");
    assert_eq!(results, [range("2023-06-11T23:00:00Z", "2023-06-13T23:00:00Z")]);
}

#[test]
fn multi_day_date_overrides_past_midnight_merge_adjacent_ranges() {
    let items =
        vec![Availability::Override(on("2023-06-12", 0, EOD)), Availability::Override(on("2023-06-13", 0, EOD))];
    let results = build(items, chrono_tz::Europe::London, "2023-06-11T00:00:00Z", "2023-06-14T00:00:00Z");
    assert_eq!(results, [range("2023-06-11T23:00:00Z", "2023-06-13T23:00:00Z")]);
}

#[test]
fn keeps_earlier_time_when_overlapping_ranges_share_an_end() {
    let items =
        vec![Availability::Weekly(weekly(&[1], 6 * H, 10 * H)), Availability::Weekly(weekly(&[1], 8 * H, 10 * H))];
    let results = build(items, Tz::UTC, "2023-06-12T00:00:00Z", "2023-06-13T00:00:00Z");
    assert_eq!(results, [range("2023-06-12T06:00:00Z", "2023-06-12T10:00:00Z")]);
}

// ---------- subtract ----------

fn july_workdays() -> Vec<DateRange> {
    [5, 6, 7, 10, 11, 12, 13, 14, 17, 18, 19, 20, 21, 24, 25, 26, 27, 28, 31]
        .iter()
        .map(|d| range(&format!("2023-07-{d:02}T04:00:00Z"), &format!("2023-07-{d:02}T12:00:00Z")))
        .collect()
}

fn expected_after_subtracting() -> Vec<DateRange> {
    let mut want = vec![
        range("2023-07-05T04:15:00Z", "2023-07-05T04:45:00Z"),
        range("2023-07-05T05:00:00Z", "2023-07-05T12:00:00Z"),
    ];
    want.extend(july_workdays().into_iter().skip(1));
    want
}

#[test]
fn subtracts_excluded_ranges_given_in_order() {
    let excluded =
        [range("2023-07-05T04:00:00Z", "2023-07-05T04:15:00Z"), range("2023-07-05T04:45:00Z", "2023-07-05T05:00:00Z")];
    assert_eq!(subtract(&july_workdays(), &excluded), expected_after_subtracting());
}

#[test]
fn subtracts_excluded_ranges_given_out_of_order() {
    let excluded =
        [range("2023-07-05T04:45:00Z", "2023-07-05T05:00:00Z"), range("2023-07-05T04:00:00Z", "2023-07-05T04:15:00Z")];
    assert_eq!(subtract(&july_workdays(), &excluded), expected_after_subtracting());
}

#[test]
fn subtract_leaves_ranges_untouched_by_busy_time() {
    let r = [range("2023-07-05T04:00:00Z", "2023-07-05T12:00:00Z")];
    let busy = [range("2023-07-05T12:00:00Z", "2023-07-05T13:00:00Z")];
    assert_eq!(subtract(&r, &busy), r);
}
