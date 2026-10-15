//! Cal.diy's `slots.test.ts`, ported case for case (MIT, see THIRD_PARTY_LICENSES).
//!
//! Left out: the out-of-office cases (no OOO in Lyric Cal) and the performance
//! case. The env-variable case passes the interval directly.

use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use lyric_cal_core::slots::{get_slots, DateRange, SlotOptions};

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// A wall-clock time in a zone, like `dayjs.tz("2023-07-13T07:30:00", tz)`.
fn tz_at(s: &str, tz: Tz) -> DateTime<Utc> {
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").unwrap();
    tz.from_local_datetime(&naive).single().unwrap().with_timezone(&Utc)
}

fn range(start: DateTime<Utc>, end: DateTime<Utc>) -> DateRange {
    DateRange { start, end }
}

/// The mocked clock the TS suite sets in `beforeAll`.
fn mock_now() -> DateTime<Utc> {
    utc("2021-06-20T11:59:59Z")
}

fn start_of_day(t: DateTime<Utc>) -> DateTime<Utc> {
    t.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc()
}

/// dayjs `endOf("day")`: 23:59:59.999.
fn end_of_day(t: DateTime<Utc>) -> DateTime<Utc> {
    start_of_day(t) + Duration::days(1) - Duration::milliseconds(1)
}

fn next_day() -> DateTime<Utc> {
    start_of_day(mock_now()) + Duration::days(1)
}

fn opts(frequency: i64, event_length: i64) -> SlotOptions {
    SlotOptions {
        now: mock_now(),
        time_zone: Tz::UTC,
        frequency,
        event_length,
        minimum_booking_notice: 0,
        offset_start: 0,
        show_optimized_slots: false,
        interval: None,
    }
}

fn formatted(slots: &[DateTime<Tz>]) -> Vec<String> {
    slots.iter().map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, false)).collect()
}

fn next_day_ranges() -> Vec<DateRange> {
    vec![range(next_day(), end_of_day(next_day()))]
}

#[test]
fn can_fit_24_hourly_slots_for_an_empty_day() {
    assert_eq!(get_slots(&next_day_ranges(), &opts(60, 60)).len(), 24);
}

#[test]
fn can_fit_24_hourly_slots_for_an_empty_day_with_interval_not_event_length() {
    assert_eq!(get_slots(&next_day_ranges(), &opts(60, 30)).len(), 24);
}

#[test]
fn only_shows_future_booking_slots_on_the_same_day() {
    // The mock date is 1s to midday, so 12 slots should be open given 0 booking notice.
    let today = start_of_day(mock_now());
    assert_eq!(get_slots(&[range(today, end_of_day(today))], &opts(60, 60)).len(), 12);
}

#[test]
fn adds_minimum_booking_notice_correctly() {
    let o = SlotOptions { minimum_booking_notice: 1500, ..opts(60, 60) };
    assert_eq!(get_slots(&next_day_ranges(), &o).len(), 11);
}

#[test]
fn shows_72_twenty_minute_slots_in_a_full_day() {
    assert_eq!(get_slots(&next_day_ranges(), &opts(20, 20)).len(), 72);
}

#[test]
fn can_create_multiple_time_slot_groups_when_multiple_date_ranges_are_given() {
    let d = next_day();
    let ranges = [
        range(d + Duration::hours(11), d + Duration::hours(12)),
        range(d + Duration::hours(14), d + Duration::hours(15)),
    ];
    assert_eq!(get_slots(&ranges, &opts(20, 20)).len(), 6);
}

#[test]
fn can_merge_multiple_time_slot_groups_when_date_ranges_overlap() {
    let d = next_day();
    let ranges = [
        range(d + Duration::hours(11), d + Duration::hours(12)),
        range(d + Duration::minutes(11 * 60 + 20), d + Duration::minutes(12 * 60 + 40)),
    ];
    assert_eq!(get_slots(&ranges, &opts(20, 20)).len(), 5);
}

#[test]
fn finds_correct_slots_when_two_unequal_date_ranges_are_given() {
    let d = next_day();
    let ranges = [
        range(d + Duration::hours(11), d + Duration::hours(13)),
        range(d + Duration::minutes(11 * 60 + 15), d + Duration::hours(13)),
    ];
    assert_eq!(get_slots(&ranges, &opts(45, 45)).len(), 2);
}

#[test]
fn finds_correct_slots_when_two_unequal_date_ranges_are_given_inverse() {
    let d = next_day();
    let ranges = [
        range(d + Duration::minutes(11 * 60 + 15), d + Duration::hours(13)),
        range(d + Duration::hours(11), d + Duration::hours(13)),
    ];
    assert_eq!(get_slots(&ranges, &opts(45, 45)).len(), 2);
}

#[test]
fn finds_correct_slots_over_the_span_of_multiple_days() {
    let d = next_day();
    let e = d + Duration::days(1);
    let ranges = [
        range(d + Duration::minutes(11 * 60 + 30), d + Duration::hours(14)),
        range(d + Duration::minutes(9 * 60 + 15), d + Duration::hours(11)),
        range(e + Duration::minutes(11 * 60 + 30), e + Duration::hours(14)),
        range(e + Duration::minutes(9 * 60 + 15), e + Duration::hours(11)),
    ];
    // each day availability should go 10:00, 12:00, 13:00
    assert_eq!(get_slots(&ranges, &opts(60, 60)).len(), 6);
}

#[test]
fn shows_71_twenty_minute_slots_when_hours_do_not_end_on_the_hour() {
    let ranges = [range(utc("2021-06-21T00:00:00Z"), utc("2021-06-21T23:45:00Z"))];
    assert_eq!(get_slots(&ranges, &opts(20, 20)).len(), 71);
}

#[test]
fn the_final_slot_of_the_day_is_included() {
    let o = SlotOptions { time_zone: chrono_tz::Europe::Brussels, minimum_booking_notice: 120, ..opts(15, 15) };
    let ranges = [
        range(utc("2023-07-13T07:00:00Z"), utc("2023-07-13T15:00:00Z")),
        range(utc("2023-07-13T18:30:00Z"), utc("2023-07-13T20:59:59Z")),
    ];
    let slots = get_slots(&ranges, &o);
    assert_eq!(formatted(&slots).last().unwrap(), "2023-07-13T22:45:00+02:00");
}

#[test]
fn slots_for_half_hour_timezones() {
    let tz = chrono_tz::Asia::Kolkata;
    let o = SlotOptions { time_zone: tz, ..opts(60, 60) };
    let ranges = [range(tz_at("2023-07-13T07:30:00", tz), tz_at("2023-07-13T09:30:00", tz))];
    assert_eq!(formatted(&get_slots(&ranges, &o)), ["2023-07-13T08:00:00+05:30"]);
}

#[test]
fn slots_for_5_minute_events() {
    let tz = chrono_tz::Europe::London;
    let o = SlotOptions { time_zone: tz, ..opts(5, 5) };
    let ranges = [
        // fits 1 slot
        range(tz_at("2023-07-13T07:00:00", tz), tz_at("2023-07-13T07:05:00", tz)),
        // fits 4 slots
        range(tz_at("2023-07-13T07:10:00", tz), tz_at("2023-07-13T07:30:00", tz)),
    ];
    assert_eq!(get_slots(&ranges, &o).len(), 5);
}

#[test]
fn slots_for_an_event_length_not_divisible_by_5() {
    let ranges = [range(utc("2023-07-13T07:22:00Z"), utc("2023-07-13T08:00:00Z"))];
    // 07:22, 07:30, 07:38, 07:46
    assert_eq!(get_slots(&ranges, &opts(8, 8)).len(), 4);
}

#[test]
fn a_10_minute_schedule_interval() {
    let o = SlotOptions { interval: Some(10), ..opts(10, 10) };
    let ranges = [range(utc("2023-07-13T00:10:00Z"), utc("2023-07-13T02:00:00Z"))];
    assert_eq!(get_slots(&ranges, &o).len(), 11);
}

// ---------- showOptimizedSlots ----------

fn la(optimized: bool, length: i64) -> SlotOptions {
    SlotOptions { time_zone: chrono_tz::America::Los_Angeles, show_optimized_slots: optimized, ..opts(length, length) }
}

/// 08:00–10:00 and 10:10–<end> Los Angeles: a 10:00–10:10 busy time cut out.
fn with_busy_gap(end: &str) -> Vec<DateRange> {
    vec![range(utc("2024-09-12T15:00:00Z"), utc("2024-09-12T17:00:00Z")), range(utc("2024-09-12T17:10:00Z"), utc(end))]
}

fn la_times(hhmm: &[&str]) -> Vec<String> {
    hhmm.iter().map(|t| format!("2024-09-12T{t}:00-07:00")).collect()
}

#[test]
fn optimized_respects_start_of_the_hour_for_0930_to_1700() {
    let ranges = [range(utc("2024-09-12T16:30:00Z"), utc("2024-09-13T00:00:00Z"))];
    assert_eq!(
        formatted(&get_slots(&ranges, &la(true, 60))),
        la_times(&["10:00", "11:00", "12:00", "13:00", "14:00", "15:00", "16:00"])
    );
}

#[test]
fn optimized_shows_4_hour_slots_for_0800_to_1230_with_a_busy_gap() {
    assert_eq!(
        formatted(&get_slots(&with_busy_gap("2024-09-12T19:30:00Z"), &la(true, 60))),
        la_times(&["08:00", "09:00", "10:15", "11:15"])
    );
}

#[test]
fn unoptimized_shows_3_hour_slots_for_0800_to_1230_with_a_busy_gap() {
    assert_eq!(
        formatted(&get_slots(&with_busy_gap("2024-09-12T19:30:00Z"), &la(false, 60))),
        la_times(&["08:00", "09:00", "11:00"])
    );
}

#[test]
fn optimized_and_unoptimized_show_3_hour_slots_for_0800_to_1200_with_a_busy_gap() {
    for optimized in [true, false] {
        assert_eq!(
            formatted(&get_slots(&with_busy_gap("2024-09-12T19:00:00Z"), &la(optimized, 60))),
            la_times(&["08:00", "09:00", "11:00"])
        );
    }
}

#[test]
fn optimized_shows_9_half_hour_slots_for_0800_to_1245_with_a_busy_gap() {
    assert_eq!(
        formatted(&get_slots(&with_busy_gap("2024-09-12T19:45:00Z"), &la(true, 30))),
        la_times(&["08:00", "08:30", "09:00", "09:30", "10:15", "10:45", "11:15", "11:45", "12:15"])
    );
}

#[test]
fn unoptimized_shows_8_half_hour_slots_for_0800_to_1245_with_a_busy_gap() {
    assert_eq!(
        formatted(&get_slots(&with_busy_gap("2024-09-12T19:45:00Z"), &la(false, 30))),
        la_times(&["08:00", "08:30", "09:00", "09:30", "10:30", "11:00", "11:30", "12:00"])
    );
}

#[test]
fn optimized_shows_13_twenty_minute_slots_for_0800_to_1230_with_a_busy_gap() {
    let slots = formatted(&get_slots(&with_busy_gap("2024-09-12T19:30:00Z"), &la(true, 20)));
    assert_eq!(slots.len(), 13);
    assert_eq!(slots[..8], la_times(&["08:00", "08:20", "08:40", "09:00", "09:20", "09:40", "10:10", "10:30"]));
}

#[test]
fn unoptimized_shows_12_twenty_minute_slots_for_0800_to_1230_with_a_busy_gap() {
    let slots = formatted(&get_slots(&with_busy_gap("2024-09-12T19:30:00Z"), &la(false, 20)));
    assert_eq!(slots.len(), 12);
    assert_eq!(slots[..8], la_times(&["08:00", "08:20", "08:40", "09:00", "09:20", "09:40", "10:20", "10:40"]));
}

#[test]
fn optimized_shows_8_hour_slots_for_0930_to_1730() {
    let ranges = [range(utc("2024-09-12T16:30:00Z"), utc("2024-09-13T00:30:00Z"))];
    assert_eq!(
        formatted(&get_slots(&ranges, &la(true, 60))),
        la_times(&["09:30", "10:30", "11:30", "12:30", "13:30", "14:30", "15:30", "16:30"])
    );
}

#[test]
fn unoptimized_shows_7_hour_slots_for_0930_to_1730() {
    let ranges = [range(utc("2024-09-12T16:30:00Z"), utc("2024-09-13T00:30:00Z"))];
    assert_eq!(
        formatted(&get_slots(&ranges, &la(false, 60))),
        la_times(&["10:00", "11:00", "12:00", "13:00", "14:00", "15:00", "16:00"])
    );
}

#[test]
fn both_respect_start_of_the_hour_for_0745_to_1530() {
    let ranges = [range(utc("2024-09-12T14:45:00Z"), utc("2024-09-12T22:30:00Z"))];
    for optimized in [true, false] {
        assert_eq!(
            formatted(&get_slots(&ranges, &la(optimized, 60))),
            la_times(&["08:00", "09:00", "10:00", "11:00", "12:00", "13:00", "14:00"])
        );
    }
}

#[test]
fn optimized_moves_to_the_next_quarter_for_0905_to_1255() {
    let ranges = [range(utc("2024-09-12T16:05:00Z"), utc("2024-09-12T19:55:00Z"))];
    assert_eq!(formatted(&get_slots(&ranges, &la(true, 60))), la_times(&["09:15", "10:15", "11:15"]));
}

#[test]
fn optimized_moves_a_same_day_start_to_the_next_5_minutes() {
    let tz = chrono_tz::America::Los_Angeles;
    let o = SlotOptions { now: utc("2021-06-20T11:20:00Z"), ..la(true, 15) };
    let ranges = [range(tz_at("2021-06-20T11:22:00", tz), tz_at("2021-06-20T12:55:00", tz))];
    let want: Vec<String> = ["11:25", "11:40", "11:55", "12:10", "12:25", "12:40"]
        .iter()
        .map(|t| format!("2021-06-20T{t}:00-07:00"))
        .collect();
    assert_eq!(formatted(&get_slots(&ranges, &o)), want);
}

#[test]
fn optimized_respects_the_hour_for_a_same_day_start_with_seconds() {
    let tz = chrono_tz::America::Los_Angeles;
    let o = SlotOptions { now: utc("2021-06-20T11:20:00Z"), ..la(true, 60) };
    let ranges = [range(tz_at("2021-06-20T11:22:36.234", tz), tz_at("2021-06-20T14:00:00", tz))];
    assert_eq!(formatted(&get_slots(&ranges, &o)), ["2021-06-20T12:00:00-07:00", "2021-06-20T13:00:00-07:00"]);
}

// ---------- 40-minute events ----------

#[test]
fn forty_minute_slots_for_a_full_day() {
    assert_eq!(get_slots(&next_day_ranges(), &opts(40, 40)).len(), 36);
}

#[test]
fn forty_minute_slots_have_the_right_times() {
    let d = next_day();
    let slots = get_slots(&[range(d + Duration::hours(9), d + Duration::hours(11))], &opts(40, 40));
    let times: Vec<String> = slots.iter().map(|t| t.format("%H:%M").to_string()).collect();
    assert_eq!(times, ["09:00", "09:40", "10:20"]);
}

#[test]
fn forty_minute_slots_are_40_minutes_apart() {
    let d = next_day();
    let slots = get_slots(&[range(d + Duration::hours(10), d + Duration::hours(12))], &opts(40, 40));
    assert_eq!(slots.len(), 3);
    for w in slots.windows(2) {
        assert_eq!((w[1] - w[0]).num_minutes(), 40);
    }
}

#[test]
fn forty_minute_slots_on_the_same_day() {
    let today = start_of_day(mock_now());
    assert_eq!(get_slots(&[range(today, end_of_day(today))], &opts(40, 40)).len(), 18);
}

#[test]
fn forty_minute_slots_across_multiple_date_ranges() {
    let d = next_day();
    let ranges = [
        range(d + Duration::hours(9), d + Duration::minutes(10 * 60 + 20)),
        range(d + Duration::hours(14), d + Duration::minutes(15 * 60 + 20)),
    ];
    assert_eq!(get_slots(&ranges, &opts(40, 40)).len(), 4);
}
