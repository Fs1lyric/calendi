//! The event editor's data: a form a person fills in, turned into what the
//! store saves. Kept apart from the widgets so the rules are testable.

use crate::grid;
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use chrono_tz::Tz;
use lyric_cal_core::db::{Event, EventInput, Exception};
use lyric_cal_core::recur::Occurrence;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeat {
    Never,
    Daily,
    Weekdays,
    Weekly,
    Monthly,
    Yearly,
    /// A rule the presets don't cover, kept as written.
    Custom(String),
}

impl Repeat {
    pub const PRESETS: [Repeat; 6] =
        [Repeat::Never, Repeat::Daily, Repeat::Weekdays, Repeat::Weekly, Repeat::Monthly, Repeat::Yearly];

    pub fn label(&self) -> &str {
        match self {
            Repeat::Never => "Does not repeat",
            Repeat::Daily => "Every day",
            Repeat::Weekdays => "Every weekday",
            Repeat::Weekly => "Every week",
            Repeat::Monthly => "Every month",
            Repeat::Yearly => "Every year",
            Repeat::Custom(rule) => rule,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ends {
    Never,
    On(NaiveDate),
    After(u32),
}

/// Which part of a series an edit or delete applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    One,
    All,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    /// The stored event being edited; `None` for a new one.
    pub event_id: Option<i64>,
    /// The occurrence opened, when it belongs to a series.
    pub original_start: Option<String>,
    /// Where the opened occurrence started, which a whole-series move is measured from.
    pub opened_start: Option<NaiveDateTime>,
    pub title: String,
    pub notes: String,
    pub all_day: bool,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub repeat: Repeat,
    pub ends: Ends,
    pub color: Option<String>,
    pub reminder_minutes: Option<i64>,
    pub tz: Tz,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Save {
    Create(EventInput),
    Update(i64, EventInput),
    Exception(Exception),
}

fn preset_rule(repeat: &Repeat) -> Option<&'static str> {
    Some(match repeat {
        Repeat::Daily => "FREQ=DAILY",
        Repeat::Weekdays => "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
        Repeat::Weekly => "FREQ=WEEKLY",
        Repeat::Monthly => "FREQ=MONTHLY",
        Repeat::Yearly => "FREQ=YEARLY",
        Repeat::Never | Repeat::Custom(_) => return None,
    })
}

/// `HH:MM`, 24-hour.
pub fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

/// Splits a stored rule into a preset and how it ends.
pub fn split_rule(rule: Option<&str>, tz: Tz) -> (Repeat, Ends) {
    let Some(rule) = rule else { return (Repeat::Never, Ends::Never) };
    let mut ends = Ends::Never;
    let mut rest = Vec::new();
    for part in rule.split(';') {
        if let Some(n) = part.strip_prefix("COUNT=").and_then(|n| n.parse().ok()) {
            ends = Ends::After(n);
        } else if let Some(v) = part.strip_prefix("UNTIL=") {
            let day = NaiveDate::parse_from_str(v, "%Y%m%d").ok().or_else(|| {
                let at = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%SZ").ok()?;
                Some(at.and_utc().with_timezone(&tz).date_naive())
            });
            match day {
                Some(day) => ends = Ends::On(day),
                None => rest.push(part),
            }
        } else {
            rest.push(part);
        }
    }
    let base = rest.join(";");
    let repeat = Repeat::PRESETS[1..]
        .iter()
        .find(|p| preset_rule(p) == Some(base.as_str()))
        .cloned()
        .unwrap_or(Repeat::Custom(base));
    (repeat, ends)
}

/// The stored rule for a preset and an end, or `None` for no repeat.
pub fn join_rule(repeat: &Repeat, ends: Ends) -> Option<String> {
    let base = match repeat {
        Repeat::Custom(rule) => rule.as_str(),
        preset => preset_rule(preset)?,
    };
    Some(match ends {
        Ends::Never => base.to_string(),
        Ends::On(day) => format!("{base};UNTIL={}", day.format("%Y%m%d")),
        Ends::After(n) => format!("{base};COUNT={n}"),
    })
}

impl Form {
    /// A new event over `[start, end)` local time.
    pub fn new(start: NaiveDateTime, end: NaiveDateTime, all_day: bool, tz: Tz) -> Self {
        Form {
            event_id: None,
            original_start: None,
            opened_start: None,
            title: String::new(),
            notes: String::new(),
            all_day,
            start,
            end,
            repeat: Repeat::Never,
            ends: Ends::Never,
            color: None,
            reminder_minutes: Some(10),
            tz,
        }
    }

    /// Opens an occurrence of a stored event.
    pub fn open(o: &Occurrence, event: &Event, tz: Tz) -> Self {
        let (start, end) = if o.all_day {
            let day = |s: &str| s.parse::<NaiveDate>().unwrap_or_default().and_time(NaiveTime::MIN);
            (day(&o.start), day(&o.end) - Duration::days(1))
        } else {
            (grid::local(&o.start, tz), grid::local(&o.end, tz))
        };
        let (repeat, ends) = split_rule(event.rrule.as_deref(), tz);
        Form {
            event_id: Some(event.id),
            original_start: o.original_start.clone(),
            opened_start: Some(start),
            title: o.title.clone(),
            notes: o.notes.clone(),
            all_day: o.all_day,
            start,
            end,
            repeat,
            ends,
            color: event.color.clone(),
            reminder_minutes: event.reminder_minutes,
            tz,
        }
    }

    /// Stored start and end for a local span; an all-day span's end is its last day.
    fn stored_span(&self, start: NaiveDateTime, end: NaiveDateTime) -> Result<(String, String), String> {
        if self.all_day {
            let (a, b) = (start.date(), end.date());
            if b < a {
                return Err("the last day can't be before the first".into());
            }
            let day = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
            return Ok((day(a), day(b + Duration::days(1))));
        }
        if end <= start {
            return Err("the event has to end after it starts".into());
        }
        Ok((grid::stamp(start, self.tz), grid::stamp(end, self.tz)))
    }

    pub fn is_series(&self) -> bool {
        self.original_start.is_some()
    }

    /// What to save. `series` is the stored event when editing an existing one.
    pub fn save(&self, scope: Scope, series: Option<&Event>) -> Result<Save, String> {
        let (start, end) = self.stored_span(self.start, self.end)?;
        if self.is_series() && scope == Scope::One {
            return Ok(Save::Exception(Exception {
                event_id: self.event_id.expect("an occurrence belongs to an event"),
                original_start: self.original_start.clone().expect("checked by is_series"),
                cancelled: false,
                title: Some(self.title.trim().to_string()),
                notes: Some(self.notes.clone()),
                start: Some(start),
                end: Some(end),
            }));
        }
        let mut input = EventInput {
            title: self.title.trim().to_string(),
            notes: self.notes.clone(),
            start,
            end,
            all_day: self.all_day,
            tz: self.tz.name().to_string(),
            rrule: join_rule(&self.repeat, self.ends),
            color: self.color.clone(),
            reminder_minutes: self.reminder_minutes,
        };
        let (Some(id), Some(event)) = (self.event_id, series) else { return Ok(Save::Create(input)) };
        input.tz = event.tz.clone();
        if self.is_series() {
            // Move the series by as much as the opened occurrence moved.
            let shift = self.start - self.opened_start.unwrap_or(self.start);
            let first = if event.all_day {
                event.start.parse::<NaiveDate>().unwrap_or_default().and_time(NaiveTime::MIN)
            } else {
                grid::local(&event.start, self.tz)
            };
            let first = first + shift;
            (input.start, input.end) = self.stored_span(first, first + (self.end - self.start))?;
        }
        Ok(Save::Update(id, input))
    }

    /// The exception that removes just the opened occurrence.
    pub fn cancel_one(&self) -> Option<Exception> {
        Some(Exception {
            event_id: self.event_id?,
            original_start: self.original_start.clone()?,
            cancelled: true,
            title: None,
            notes: None,
            start: None,
            end: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DHAKA: Tz = chrono_tz::Asia::Dhaka;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn stored(id: i64, input: EventInput) -> Event {
        Event { id, input, booking_id: None, created_at: String::new(), updated_at: String::new() }
    }

    fn series() -> Event {
        stored(
            7,
            EventInput {
                title: "Standup".into(),
                notes: "daily".into(),
                start: "2026-10-19T03:00:00Z".into(), // 09:00 Dhaka
                end: "2026-10-19T03:15:00Z".into(),
                all_day: false,
                tz: "Asia/Dhaka".into(),
                rrule: Some("FREQ=DAILY;COUNT=10".into()),
                color: Some("#a5e0ca".into()),
                reminder_minutes: Some(5),
            },
        )
    }

    fn occurrence_of(e: &Event, start: &str, end: &str) -> Occurrence {
        Occurrence {
            event_id: e.id,
            title: e.title.clone(),
            notes: e.notes.clone(),
            start: start.into(),
            end: end.into(),
            all_day: e.all_day,
            color: e.color.clone(),
            reminder_minutes: e.reminder_minutes,
            booking_id: None,
            original_start: e.rrule.as_ref().map(|_| start.to_string()),
        }
    }

    #[test]
    fn times_parse_as_24_hour() {
        assert_eq!(parse_time("09:30"), NaiveTime::from_hms_opt(9, 30, 0));
        assert_eq!(parse_time(" 7:05 "), NaiveTime::from_hms_opt(7, 5, 0));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("noon"), None);
    }

    #[test]
    fn rules_split_into_presets_and_ends() {
        assert_eq!(split_rule(None, DHAKA), (Repeat::Never, Ends::Never));
        assert_eq!(split_rule(Some("FREQ=WEEKLY"), DHAKA), (Repeat::Weekly, Ends::Never));
        assert_eq!(
            split_rule(Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;COUNT=4"), DHAKA),
            (Repeat::Weekdays, Ends::After(4))
        );
        assert_eq!(split_rule(Some("FREQ=DAILY;UNTIL=20261031"), DHAKA), (Repeat::Daily, Ends::On(d("2026-10-31"))));
        // 31 Oct 17:59:59Z is still the 31st in Dhaka.
        assert_eq!(
            split_rule(Some("FREQ=MONTHLY;UNTIL=20261031T175959Z"), DHAKA),
            (Repeat::Monthly, Ends::On(d("2026-10-31")))
        );
        assert_eq!(
            split_rule(Some("FREQ=WEEKLY;INTERVAL=2"), DHAKA),
            (Repeat::Custom("FREQ=WEEKLY;INTERVAL=2".into()), Ends::Never)
        );
    }

    #[test]
    fn rules_join_back() {
        assert_eq!(join_rule(&Repeat::Never, Ends::After(3)), None);
        assert_eq!(join_rule(&Repeat::Yearly, Ends::Never).as_deref(), Some("FREQ=YEARLY"));
        assert_eq!(
            join_rule(&Repeat::Weekdays, Ends::On(d("2026-12-31"))).as_deref(),
            Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;UNTIL=20261231")
        );
        assert_eq!(join_rule(&Repeat::Daily, Ends::After(5)).as_deref(), Some("FREQ=DAILY;COUNT=5"));
        let custom = Repeat::Custom("FREQ=WEEKLY;INTERVAL=2".into());
        assert_eq!(join_rule(&custom, Ends::After(2)).as_deref(), Some("FREQ=WEEKLY;INTERVAL=2;COUNT=2"));
    }

    #[test]
    fn a_new_timed_event_saves_in_utc() {
        let mut f = Form::new(t("2026-10-19 09:00"), t("2026-10-19 10:00"), false, DHAKA);
        f.title = "  Dentist ".into();
        let Save::Create(e) = f.save(Scope::All, None).unwrap() else { panic!() };
        assert_eq!((e.start.as_str(), e.end.as_str()), ("2026-10-19T03:00:00Z", "2026-10-19T04:00:00Z"));
        assert_eq!(e.title, "Dentist");
        assert_eq!(e.tz, "Asia/Dhaka");
        assert_eq!(e.rrule, None);
    }

    #[test]
    fn a_new_all_day_event_saves_dates_with_an_exclusive_end() {
        // The form shows the last day; the store wants the day after it.
        let f = Form::new(t("2026-10-19 00:00"), t("2026-10-20 00:00"), true, DHAKA);
        let Save::Create(e) = f.save(Scope::All, None).unwrap() else { panic!() };
        assert_eq!((e.start.as_str(), e.end.as_str()), ("2026-10-19", "2026-10-21"));
        assert!(e.all_day);
    }

    #[test]
    fn the_end_must_come_after_the_start() {
        let f = Form::new(t("2026-10-19 10:00"), t("2026-10-19 10:00"), false, DHAKA);
        assert!(f.save(Scope::All, None).is_err());
        let f = Form::new(t("2026-10-20 00:00"), t("2026-10-19 00:00"), true, DHAKA);
        assert!(f.save(Scope::All, None).is_err());
    }

    #[test]
    fn opening_an_occurrence_shows_it_in_local_time() {
        let e = series();
        let o = occurrence_of(&e, "2026-10-21T03:00:00Z", "2026-10-21T03:15:00Z");
        let f = Form::open(&o, &e, DHAKA);
        assert_eq!((f.start, f.end), (t("2026-10-21 09:00"), t("2026-10-21 09:15")));
        assert_eq!((f.repeat.clone(), f.ends), (Repeat::Daily, Ends::After(10)));
        assert!(f.is_series());
        assert_eq!(f.color.as_deref(), Some("#a5e0ca"));
    }

    #[test]
    fn editing_one_occurrence_writes_an_exception() {
        let e = series();
        let o = occurrence_of(&e, "2026-10-21T03:00:00Z", "2026-10-21T03:15:00Z");
        let mut f = Form::open(&o, &e, DHAKA);
        f.title = "Standup (late)".into();
        f.start = t("2026-10-21 11:00");
        f.end = t("2026-10-21 11:30");
        let Save::Exception(x) = f.save(Scope::One, Some(&e)).unwrap() else { panic!() };
        assert_eq!(x.event_id, 7);
        assert_eq!(x.original_start, "2026-10-21T03:00:00Z");
        assert_eq!(x.title.as_deref(), Some("Standup (late)"));
        assert_eq!(x.start.as_deref(), Some("2026-10-21T05:00:00Z"));
        assert_eq!(x.end.as_deref(), Some("2026-10-21T05:30:00Z"));
        assert!(!x.cancelled);

        assert_eq!(
            f.cancel_one().map(|x| (x.original_start, x.cancelled)),
            Some(("2026-10-21T03:00:00Z".into(), true))
        );
    }

    #[test]
    fn moving_the_whole_series_shifts_it_by_the_same_amount() {
        let e = series();
        // Opened on the 21st and moved to 10:00, half an hour long.
        let o = occurrence_of(&e, "2026-10-21T03:00:00Z", "2026-10-21T03:15:00Z");
        let mut f = Form::open(&o, &e, DHAKA);
        f.start = t("2026-10-21 10:00");
        f.end = t("2026-10-21 10:30");
        let Save::Update(id, input) = f.save(Scope::All, Some(&e)).unwrap() else { panic!() };
        assert_eq!(id, 7);
        // The series still starts on the 19th, an hour later.
        assert_eq!((input.start.as_str(), input.end.as_str()), ("2026-10-19T04:00:00Z", "2026-10-19T04:30:00Z"));
        assert_eq!(input.rrule.as_deref(), Some("FREQ=DAILY;COUNT=10"));
        assert_eq!(input.reminder_minutes, Some(5));
    }

    #[test]
    fn a_single_event_updates_in_place() {
        let mut e = series();
        e.input.rrule = None;
        let o = occurrence_of(&e, &e.start.clone(), &e.end.clone());
        let mut f = Form::open(&o, &e, DHAKA);
        assert!(!f.is_series());
        f.repeat = Repeat::Weekly;
        let Save::Update(_, input) = f.save(Scope::One, Some(&e)).unwrap() else { panic!() };
        assert_eq!(input.rrule.as_deref(), Some("FREQ=WEEKLY"));
        assert_eq!(f.cancel_one(), None);
    }
}
