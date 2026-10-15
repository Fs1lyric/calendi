//! Booking setup data: the weekly hours and date overrides forms, and the
//! small text helpers the booking screen edits them with.

use chrono::NaiveDate;
use lyric_cal_core::db::{Override, WeeklyHours};

/// Weekdays Monday first, as the grid shows them; the number is the store's (0 = Sunday).
pub const WEEKDAYS: [(u32, &str); 7] =
    [(1, "Mon"), (2, "Tue"), (3, "Wed"), (4, "Thu"), (5, "Fri"), (6, "Sat"), (0, "Sun")];

/// `9`, `9:30`, `09:30` or `24:00` as minutes after midnight.
pub fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':').unwrap_or((s.trim(), "0"));
    let digits = |t: &str| !t.is_empty() && t.len() <= 2 && t.bytes().all(|b| b.is_ascii_digit());
    if !digits(h) || !digits(m) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    let minutes = h * 60 + m;
    (m < 60 && minutes <= 24 * 60).then_some(minutes)
}

pub fn fmt_hhmm(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// A title as a link name: lowercase letters, digits and single dashes.
pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if (c.is_whitespace() || c == '-' || c == '_') && !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').chars().take(64).collect()
}

/// One open stretch of a day, as typed.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub from: String,
    pub to: String,
}

impl Span {
    pub fn new(from: u32, to: u32) -> Self {
        Span { from: fmt_hhmm(from), to: fmt_hhmm(to) }
    }

    /// The span in minutes; errors start with `what`, the day it belongs to.
    fn read(&self, what: &str) -> Result<(u32, u32), String> {
        let time = |t: &str| parse_hhmm(t).ok_or_else(|| format!("{what}: {:?} is not a time like 17:30", t.trim()));
        let (from, to) = (time(&self.from)?, time(&self.to)?);
        if from >= to {
            return Err(format!("{what}: {} is not before {}", fmt_hhmm(from), fmt_hhmm(to)));
        }
        Ok((from, to))
    }
}

/// Weekly hours by store weekday (index 0 = Sunday). An empty day is closed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hours {
    pub days: [Vec<Span>; 7],
}

impl Hours {
    pub fn from_store(hours: &[WeeklyHours]) -> Self {
        let mut out = Hours::default();
        for h in hours {
            out.days[h.weekday as usize % 7].push(Span::new(h.start_minute, h.end_minute));
        }
        out
    }

    pub fn to_store(&self) -> Result<Vec<WeeklyHours>, String> {
        let mut out = Vec::new();
        for (weekday, name) in WEEKDAYS {
            for span in &self.days[weekday as usize] {
                let (start_minute, end_minute) = span.read(name)?;
                out.push(WeeklyHours { weekday, start_minute, end_minute });
            }
        }
        out.sort_by_key(|h| (h.weekday, h.start_minute));
        Ok(out)
    }
}

/// A date with different hours, or none at all.
#[derive(Debug, Clone, PartialEq)]
pub struct DayOff {
    pub date: NaiveDate,
    pub off: bool,
    pub span: Span,
}

pub fn overrides_from_store(list: &[Override]) -> Vec<DayOff> {
    list.iter()
        .filter_map(|o| {
            let date = NaiveDate::parse_from_str(&o.date, "%Y-%m-%d").ok()?;
            let off = o.start_minute >= o.end_minute;
            let span = if off { Span::new(9 * 60, 17 * 60) } else { Span::new(o.start_minute, o.end_minute) };
            Some(DayOff { date, off, span })
        })
        .collect()
}

pub fn overrides_to_store(list: &[DayOff]) -> Result<Vec<Override>, String> {
    let mut out = Vec::new();
    for d in list {
        let (start_minute, end_minute) =
            if d.off { (0, 0) } else { d.span.read(&d.date.format("%-d %b").to_string())? };
        out.push(Override { date: d.date.format("%Y-%m-%d").to_string(), start_minute, end_minute });
    }
    out.sort_by(|a, b| (&a.date, a.start_minute).cmp(&(&b.date, b.start_minute)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_parse_loosely_and_print_padded() {
        assert_eq!(parse_hhmm("9"), Some(540));
        assert_eq!(parse_hhmm(" 9:30 "), Some(570));
        assert_eq!(parse_hhmm("09:05"), Some(545));
        assert_eq!(parse_hhmm("24:00"), Some(1440));
        for bad in ["", "24:01", "9:60", "nine", "9:5x", "-1"] {
            assert_eq!(parse_hhmm(bad), None, "{bad:?}");
        }
        assert_eq!(fmt_hhmm(545), "09:05");
        assert_eq!(fmt_hhmm(1440), "24:00");
    }

    #[test]
    fn titles_become_link_names() {
        assert_eq!(slugify("Intro call"), "intro-call");
        assert_eq!(slugify("  30 min — Quick chat!! "), "30-min-quick-chat");
        assert_eq!(slugify("Café"), "caf");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn hours_round_trip_by_weekday() {
        let store = vec![
            WeeklyHours { weekday: 1, start_minute: 540, end_minute: 720 },
            WeeklyHours { weekday: 1, start_minute: 780, end_minute: 1020 },
            WeeklyHours { weekday: 5, start_minute: 600, end_minute: 900 },
        ];
        let hours = Hours::from_store(&store);
        assert_eq!(hours.days[1], vec![Span::new(540, 720), Span::new(780, 1020)]);
        assert!(hours.days[0].is_empty());
        assert_eq!(hours.to_store().unwrap(), store);
    }

    #[test]
    fn bad_hours_name_the_day() {
        let mut hours = Hours::default();
        hours.days[3].push(Span { from: "17:00".into(), to: "09:00".into() });
        assert_eq!(hours.to_store().unwrap_err(), "Wed: 17:00 is not before 09:00");
        hours.days[3][0] = Span { from: "9".into(), to: "late".into() };
        assert_eq!(hours.to_store().unwrap_err(), "Wed: \"late\" is not a time like 17:30");
    }

    #[test]
    fn a_day_off_is_an_empty_override() {
        let date = NaiveDate::from_ymd_opt(2026, 12, 25).unwrap();
        let days = vec![
            DayOff { date, off: true, span: Span::new(540, 1020) },
            DayOff { date: date.succ_opt().unwrap(), off: false, span: Span::new(600, 720) },
        ];
        let store = overrides_to_store(&days).unwrap();
        assert_eq!(
            store,
            vec![
                Override { date: "2026-12-25".into(), start_minute: 0, end_minute: 0 },
                Override { date: "2026-12-26".into(), start_minute: 600, end_minute: 720 },
            ]
        );
        let back = overrides_from_store(&store);
        assert!(back[0].off);
        assert_eq!(back[1], days[1]);
    }
}
