//! Desktop reminders: every 30 s, notify for reminders due since the last check.

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use lyric_cal_core::db::Reminder;

/// After a restart, reminders missed for longer than this are dropped, not replayed.
const CATCH_UP: i64 = 10;

/// Where this check starts: the saved last check, but no more than [`CATCH_UP`] minutes back.
pub fn window_start(saved: Option<&str>, now: DateTime<Utc>) -> DateTime<Utc> {
    let floor = now - Duration::minutes(CATCH_UP);
    let saved = saved.and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|t| t.with_timezone(&Utc));
    saved.unwrap_or(floor).clamp(floor, now)
}

/// The notification's summary and body, like ("Dentist", "in 10 min · 15:00").
pub fn text(r: &Reminder, now: DateTime<Utc>, tz: Tz) -> (String, String) {
    let local = r.start.with_timezone(&tz);
    if r.all_day {
        return (r.title.clone(), format!("all day {}", local.format("%a %-d %b")));
    }
    let minutes = (r.start - now).num_minutes();
    let soon = match minutes {
        ..=0 => "now".to_string(),
        1..=89 => format!("in {minutes} min"),
        _ => format!("in {} h", (minutes + 30) / 60),
    };
    (r.title.clone(), format!("{soon} · {}", local.format("%H:%M")))
}

/// Runs forever: checks every 30 s and notifies through `notify-send`.
pub async fn run(state: std::sync::Arc<crate::State>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tick.tick().await;
        let now = (state.now)();
        let due = {
            let db = state.db.lock().unwrap();
            let after = window_start(db.setting("reminders_checked").ok().flatten().as_deref(), now);
            let due = db.due_reminders(after, now, state.tz);
            let _ = db.set_setting("reminders_checked", &now.to_rfc3339());
            due
        };
        match due {
            Ok(list) => {
                for r in list {
                    let (summary, body) = text(&r, now, state.tz);
                    let sent = tokio::process::Command::new("notify-send")
                        .args(["--app-name=Calendi", "--icon=calendi", "--urgency=critical", &summary, &body])
                        .status()
                        .await;
                    if let Err(e) = sent {
                        eprintln!("lyric-cal-booker: notify-send failed: {e}");
                    }
                }
            }
            Err(e) => eprintln!("lyric-cal-booker: reminders: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn checks_resume_where_they_left_off_but_not_from_long_ago() {
        let now = at("2026-10-19T09:00:00Z");
        assert_eq!(window_start(Some("2026-10-19T08:59:30Z"), now), at("2026-10-19T08:59:30Z"));
        assert_eq!(window_start(Some("2026-10-18T09:00:00Z"), now), at("2026-10-19T08:50:00Z"));
        assert_eq!(window_start(None, now), at("2026-10-19T08:50:00Z"));
        assert_eq!(window_start(Some("junk"), now), at("2026-10-19T08:50:00Z"));
        assert_eq!(window_start(Some("2026-10-19T09:05:00Z"), now), now, "clock went back");
    }

    #[test]
    fn text_says_how_soon() {
        let tz = chrono_tz::Asia::Dhaka;
        let now = at("2026-10-19T08:50:00Z");
        let r = |start: &str, all_day| Reminder { title: "Dentist".into(), start: at(start), all_day };
        assert_eq!(text(&r("2026-10-19T09:00:00Z", false), now, tz), ("Dentist".into(), "in 10 min · 15:00".into()));
        assert_eq!(text(&r("2026-10-19T08:50:00Z", false), now, tz), ("Dentist".into(), "now · 14:50".into()));
        assert_eq!(text(&r("2026-10-19T10:50:00Z", false), now, tz), ("Dentist".into(), "in 2 h · 16:50".into()));
        assert_eq!(text(&r("2026-10-20T18:00:00Z", true), now, tz), ("Dentist".into(), "all day Wed 21 Oct".into()));
        assert_eq!(text(&r("2026-10-19T09:00:00Z", false), now, Tz::UTC).1, "in 10 min · 09:00");
    }
}
