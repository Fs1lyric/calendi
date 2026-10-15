//! The month grid and the week/day time grid. They only paint and report what
//! the person did; the app decides what that means for the store.

use crate::grid::{self, View};
use crate::theme;
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use chrono_tz::Tz;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use lyric_cal_core::recur::Occurrence;

pub const HOUR_PX: f32 = 48.0;
const GUTTER: f32 = 56.0;
const SNAP: i64 = 15;

pub enum Action {
    /// Open the occurrence at this index for editing.
    Open(usize),
    New {
        start: NaiveDateTime,
        end: NaiveDateTime,
        all_day: bool,
    },
    /// Drop the occurrence at this index so it starts at `start`.
    Move {
        index: usize,
        start: NaiveDateTime,
    },
    Goto(NaiveDate),
}

pub struct Scene<'a> {
    pub occ: &'a [Occurrence],
    pub tz: Tz,
    pub now: NaiveDateTime,
}

impl Scene<'_> {
    fn local_start(&self, o: &Occurrence) -> NaiveDateTime {
        if o.all_day {
            o.start.parse::<NaiveDate>().unwrap_or_default().and_time(NaiveTime::MIN)
        } else {
            grid::local(&o.start, self.tz)
        }
    }
}

fn at(day: NaiveDate, hour: u32) -> NaiveDateTime {
    day.and_hms_opt(hour, 0, 0).expect("valid hour")
}

fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// A rounded event chip with a coloured edge and clipped text. `notes` go under
/// the lines, wrapped to the chip's width.
fn chip(ui: &Ui, rect: Rect, color: Color32, lines: &[(String, f32, Color32)], notes: Option<&str>) {
    let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    p.rect_filled(rect, CornerRadius::same(4), theme::soft(color));
    p.rect_filled(Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())), CornerRadius::same(2), color);
    let mut y = rect.top() + 2.0;
    for (text, size, c) in lines {
        p.text(Pos2::new(rect.left() + 7.0, y), Align2::LEFT_TOP, text, font(*size), *c);
        y += size + 3.0;
    }
    if let Some(notes) = notes {
        let galley = p.layout(notes.to_owned(), font(11.0), theme::INK_2, (rect.width() - 10.0).max(1.0));
        p.galley(Pos2::new(rect.left() + 7.0, y), galley, theme::INK_2);
    }
}

/// The smallest time-grid block with room for a line of notes under its title
/// and time.
const NOTES_MIN_H: f32 = 44.0;

/// The notes to show on hover, if there are any.
fn hover_notes(notes: &str) -> Option<&str> {
    Some(notes.trim()).filter(|n| !n.is_empty())
}

/// The notes to print inside a time-grid block `height` pixels tall. Shorter
/// blocks leave them to the hover tooltip.
fn block_notes(notes: &str, height: f32) -> Option<&str> {
    hover_notes(notes).filter(|_| height >= NOTES_MIN_H)
}

/// Adds the event's notes to a chip's response as its hover tooltip.
fn with_notes(resp: egui::Response, notes: &str) -> egui::Response {
    match hover_notes(notes) {
        Some(n) => resp.on_hover_text(n),
        None => resp,
    }
}

// ---------- month ----------

pub fn month(ui: &mut Ui, s: &Scene, anchor: NaiveDate) -> Option<Action> {
    let mut action = None;
    let days = grid::days(View::Month, anchor);
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let head = 26.0;
    let (w, h) = (full.width() / 7.0, (full.height() - head) / 6.0);
    let painter = ui.painter().clone();

    for (i, name) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].iter().enumerate() {
        let x = full.left() + w * (i as f32 + 0.5);
        painter.text(Pos2::new(x, full.top() + head / 2.0), Align2::CENTER_CENTER, name, font(12.0), theme::INK_3);
    }

    for (i, &day) in days.iter().enumerate() {
        let (row, col) = (i / 7, i % 7);
        let cell = Rect::from_min_size(
            Pos2::new(full.left() + w * col as f32, full.top() + head + h * row as f32),
            egui::vec2(w, h),
        );
        let r = ui.interact(cell, Id::new(("cell", day)), Sense::click());
        let fill = if r.hovered() { theme::SURFACE } else { theme::BG };
        painter.rect_filled(cell, 0.0, fill);
        painter.rect_stroke(cell, 0.0, Stroke::new(0.5, theme::LINE), StrokeKind::Inside);
        if r.clicked() {
            action = Some(Action::New { start: at(day, 9), end: at(day, 10), all_day: false });
        }

        let today = day == s.now.date();
        let in_month = day.month() == anchor.month();
        let num_center = Pos2::new(cell.left() + 16.0, cell.top() + 14.0);
        let num_rect = Rect::from_center_size(num_center, egui::vec2(24.0, 22.0));
        if today {
            painter.circle_filled(num_center, 11.0, theme::ACCENT);
        }
        let num_color = if today {
            theme::BG
        } else if in_month {
            theme::INK
        } else {
            theme::INK_3
        };
        painter.text(num_center, Align2::CENTER_CENTER, day.day(), font(13.0), num_color);
        if ui.interact(num_rect, Id::new(("num", day)), Sense::click()).on_hover_text("Open this day").clicked() {
            action = Some(Action::Goto(day));
        }

        let mut items: Vec<usize> = (0..s.occ.len())
            .filter(|&k| {
                let o = &s.occ[k];
                if o.all_day {
                    grid::covers(o, day)
                } else {
                    grid::span_on(o, day, s.tz).is_some()
                }
            })
            .collect();
        items.sort_by_key(|&k| (!s.occ[k].all_day, s.local_start(&s.occ[k])));

        let line = 19.0;
        let room = ((cell.height() - 30.0) / line).floor().max(1.0) as usize;
        let hidden = items.len().saturating_sub(room);
        for (n, &k) in items.iter().take(room).enumerate() {
            let o = &s.occ[k];
            let mut r = Rect::from_min_size(
                Pos2::new(cell.left() + 4.0, cell.top() + 28.0 + line * n as f32),
                egui::vec2(cell.width() - 8.0, line - 2.0),
            );
            // The last line makes room for "+n" when some don't fit.
            if hidden > 0 && n + 1 == room {
                let more = Rect::from_min_max(Pos2::new(r.right() - 30.0, r.top()), r.max);
                r.max.x -= 32.0;
                painter.text(more.center(), Align2::CENTER_CENTER, format!("+{hidden}"), font(12.0), theme::INK_2);
                let resp = ui.interact(more, Id::new(("more", day)), Sense::click());
                if resp.on_hover_text(format!("{hidden} more")).clicked() {
                    action = Some(Action::Goto(day));
                }
            }
            let color = theme::hex(o.color.as_deref());
            let text = if o.all_day {
                o.title.clone()
            } else {
                format!("{} {}", grid::local(&o.start, s.tz).format("%H:%M"), o.title)
            };
            chip(ui, r, color, &[(text, 12.0, theme::INK)], None);
            if with_notes(ui.interact(r, Id::new(("mchip", k, day)), Sense::click()), &o.notes).clicked() {
                action = Some(Action::Open(k));
            }
        }
    }
    action
}

// ---------- week and day ----------

/// A drag in progress, kept in egui's memory between frames.
#[derive(Clone, Copy)]
enum Drag {
    Create { day: usize, from: i64 },
    Move { index: usize, grab: i64 },
}

pub fn time_grid(ui: &mut Ui, days: &[NaiveDate], s: &Scene, scroll_to: Option<f32>) -> Option<Action> {
    let mut action = None;
    let full = ui.available_rect_before_wrap();
    let col_w = (full.width() - GUTTER) / days.len() as f32;
    let col_x = |i: usize| full.left() + GUTTER + col_w * i as f32;

    // Day headers.
    let head = 46.0;
    let (head_rect, _) = ui.allocate_exact_size(egui::vec2(full.width(), head), Sense::hover());
    for (i, &day) in days.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(col_x(i), head_rect.top()), egui::vec2(col_w, head));
        let today = day == s.now.date();
        let p = ui.painter();
        p.text(
            Pos2::new(r.center().x, r.top() + 10.0),
            Align2::CENTER_CENTER,
            day.format("%a").to_string().to_uppercase(),
            font(11.0),
            if today { theme::ACCENT } else { theme::INK_3 },
        );
        let c = Pos2::new(r.center().x, r.top() + 30.0);
        if today {
            p.circle_filled(c, 13.0, theme::ACCENT);
        }
        p.text(c, Align2::CENTER_CENTER, day.day(), font(17.0), if today { theme::BG } else { theme::INK });
        if days.len() > 1 && ui.interact(r, Id::new(("head", day)), Sense::click()).clicked() {
            action = Some(Action::Goto(day));
        }
    }

    // All-day lane: each all-day occurrence spans its visible days.
    let first = days[0];
    let all_day: Vec<(usize, i64, i64)> = s
        .occ
        .iter()
        .enumerate()
        .filter(|(_, o)| o.all_day)
        .filter_map(|(k, o)| {
            let a = (o.start.parse::<NaiveDate>().ok()? - first).num_days().max(0);
            let b = (o.end.parse::<NaiveDate>().ok()? - first).num_days().min(days.len() as i64);
            (a < b).then_some((k, a, b))
        })
        .collect();
    let lanes = grid::pack(&all_day.iter().map(|&(_, a, b)| (a, b)).collect::<Vec<_>>());
    let lane_count = lanes.iter().map(|p| p.column + 1).max().unwrap_or(0).max(1);
    let lane_h = 22.0;
    let (lane_rect, _) =
        ui.allocate_exact_size(egui::vec2(full.width(), lane_count as f32 * lane_h + 8.0), Sense::hover());
    ui.painter().text(
        Pos2::new(lane_rect.left() + GUTTER - 8.0, lane_rect.top() + 14.0),
        Align2::RIGHT_CENTER,
        "all day",
        font(11.0),
        theme::INK_3,
    );
    for (i, &day) in days.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(col_x(i), lane_rect.top()), egui::vec2(col_w, lane_rect.height()));
        ui.painter().rect_stroke(r, 0.0, Stroke::new(0.5, theme::LINE), StrokeKind::Inside);
        if ui.interact(r, Id::new(("lane", day)), Sense::click()).clicked() {
            action = Some(Action::New { start: at(day, 0), end: at(day, 0), all_day: true });
        }
    }
    for (&(k, a, b), lane) in all_day.iter().zip(&lanes) {
        let r = Rect::from_min_max(
            Pos2::new(col_x(a as usize) + 2.0, lane_rect.top() + 4.0 + lane.column as f32 * lane_h),
            Pos2::new(col_x(b as usize) - 2.0, lane_rect.top() + 2.0 + (lane.column + 1) as f32 * lane_h),
        );
        let o = &s.occ[k];
        chip(ui, r, theme::hex(o.color.as_deref()), &[(o.title.clone(), 12.0, theme::INK)], None);
        if with_notes(ui.interact(r, Id::new(("achip", k)), Sense::click()), &o.notes).clicked() {
            action = Some(Action::Open(k));
        }
    }

    // The hours.
    let mut area = egui::ScrollArea::vertical().id_salt("timegrid").auto_shrink([false, false]);
    if let Some(y) = scroll_to {
        area = area.vertical_scroll_offset(y);
    }
    area.show(ui, |ui| {
        let (body, _) = ui.allocate_exact_size(egui::vec2(full.width(), 24.0 * HOUR_PX), Sense::hover());
        let col = |i: usize| Rect::from_min_size(Pos2::new(col_x(i), body.top()), egui::vec2(col_w, body.height()));
        let p = ui.painter().clone();
        for h in 0..24 {
            let y = body.top() + h as f32 * HOUR_PX;
            p.line_segment(
                [Pos2::new(body.left() + GUTTER, y), Pos2::new(body.right(), y)],
                Stroke::new(0.5, theme::LINE),
            );
            if h > 0 {
                p.text(
                    Pos2::new(body.left() + GUTTER - 8.0, y),
                    Align2::RIGHT_CENTER,
                    format!("{h:02}:00"),
                    font(11.0),
                    theme::INK_3,
                );
            }
        }
        let pointer = ui.ctx().pointer_interact_pos();
        let day_at = |pos: Pos2| ((pos.x - col_x(0)) / col_w).floor().clamp(0.0, days.len() as f32 - 1.0) as usize;
        let minute_at = |pos: Pos2| grid::minute_at(pos.y - body.top(), HOUR_PX, SNAP);
        let drag_id = Id::new("grid-drag");
        let drag: Option<Drag> = ui.data(|d| d.get_temp(drag_id));

        // Empty space: click for an hour, drag to sweep out a span.
        for (i, &day) in days.iter().enumerate() {
            let rect = col(i);
            p.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(0.5, theme::LINE));
            let r = ui.interact(rect, Id::new(("col", day)), Sense::click_and_drag());
            if r.clicked() {
                if let Some(pos) = r.interact_pointer_pos() {
                    let m = grid::minute_at(pos.y - body.top(), HOUR_PX, 30);
                    let start = day.and_time(NaiveTime::MIN) + Duration::minutes(m);
                    action = Some(Action::New { start, end: start + Duration::hours(1), all_day: false });
                }
            }
            if r.drag_started() {
                if let Some(pos) = r.interact_pointer_pos() {
                    ui.data_mut(|d| d.insert_temp(drag_id, Drag::Create { day: i, from: minute_at(pos) }));
                }
            }
        }
        if let (Some(Drag::Create { day, from }), Some(pos)) = (drag, pointer) {
            let to = minute_at(pos);
            let (a, b) = (from.min(to), from.max(to) + SNAP);
            let ghost = Rect::from_min_max(
                Pos2::new(col(day).left() + 2.0, body.top() + a as f32 / 60.0 * HOUR_PX),
                Pos2::new(col(day).right() - 4.0, body.top() + b as f32 / 60.0 * HOUR_PX),
            );
            p.rect_filled(ghost, CornerRadius::same(4), theme::soft(theme::ACCENT));
            p.rect_stroke(ghost, CornerRadius::same(4), Stroke::new(1.0, theme::ACCENT), StrokeKind::Inside);
            let label = format!("{} – {}", hm(a), hm(b));
            p.text(ghost.left_top() + egui::vec2(6.0, 4.0), Align2::LEFT_TOP, label, font(11.0), theme::INK);
            if ui.input(|i| i.pointer.any_released()) {
                let midnight = days[day].and_time(NaiveTime::MIN);
                action = Some(Action::New {
                    start: midnight + Duration::minutes(a),
                    end: midnight + Duration::minutes(b),
                    all_day: false,
                });
                ui.data_mut(|d| d.remove::<Drag>(drag_id));
            }
        }

        // Events, packed side by side where they overlap.
        for (i, &day) in days.iter().enumerate() {
            let todays: Vec<(usize, (i64, i64))> =
                (0..s.occ.len()).filter_map(|k| grid::span_on(&s.occ[k], day, s.tz).map(|sp| (k, sp))).collect();
            let placed = grid::pack(&todays.iter().map(|t| t.1).collect::<Vec<_>>());
            for (&(k, (a, b)), pl) in todays.iter().zip(&placed) {
                let o = &s.occ[k];
                let w = col_w / pl.columns as f32;
                let r = Rect::from_min_size(
                    Pos2::new(col(i).left() + w * pl.column as f32 + 1.0, body.top() + a as f32 / 60.0 * HOUR_PX + 1.0),
                    egui::vec2(w - 4.0, ((b - a) as f32 / 60.0 * HOUR_PX - 2.0).max(16.0)),
                );
                let moving = matches!(drag, Some(Drag::Move { index, .. }) if index == k);
                let color = theme::hex(o.color.as_deref());
                let ink = if moving { theme::INK_3 } else { theme::INK };
                let start = grid::local(&o.start, s.tz);
                let end = grid::local(&o.end, s.tz);
                let mut lines = vec![(o.title.clone(), 13.0, ink)];
                if r.height() > 34.0 {
                    lines.push((format!("{} – {}", start.format("%H:%M"), end.format("%H:%M")), 11.0, theme::INK_2));
                }
                chip(ui, r, color, &lines, block_notes(&o.notes, r.height()));
                let resp = with_notes(ui.interact(r, Id::new(("ev", k, day)), Sense::click_and_drag()), &o.notes);
                if resp.clicked() {
                    action = Some(Action::Open(k));
                }
                if resp.drag_started() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let under = day.and_time(NaiveTime::MIN) + Duration::minutes(minute_at(pos));
                        let grab = (under - start).num_minutes();
                        ui.data_mut(|d| d.insert_temp(drag_id, Drag::Move { index: k, grab }));
                    }
                }
            }
        }
        if let (Some(Drag::Move { index, grab }), Some(pos)) = (drag, pointer) {
            let o = &s.occ[index];
            let length = grid::local(&o.end, s.tz) - grid::local(&o.start, s.tz);
            let d = day_at(pos);
            let raw = days[d].and_time(NaiveTime::MIN) + Duration::minutes(minute_at(pos) - grab);
            let start = snap_time(raw);
            let a = (start - days[d].and_time(NaiveTime::MIN)).num_minutes();
            let top = body.top() + a as f32 / 60.0 * HOUR_PX;
            let ghost = Rect::from_min_size(
                Pos2::new(col(d).left() + 1.0, top),
                egui::vec2(col_w - 4.0, (length.num_minutes() as f32 / 60.0 * HOUR_PX).max(16.0)),
            );
            p.rect_stroke(
                ghost,
                CornerRadius::same(4),
                Stroke::new(1.5, theme::hex(o.color.as_deref())),
                StrokeKind::Inside,
            );
            p.text(
                ghost.left_top() + egui::vec2(7.0, 3.0),
                Align2::LEFT_TOP,
                start.format("%H:%M"),
                font(11.0),
                theme::INK,
            );
            if ui.input(|i| i.pointer.any_released()) {
                if start != grid::local(&o.start, s.tz) {
                    action = Some(Action::Move { index, start });
                }
                ui.data_mut(|d| d.remove::<Drag>(drag_id));
            }
        }

        // Now.
        if let Some(i) = days.iter().position(|&d| d == s.now.date()) {
            let y = body.top() + (s.now.hour() * 60 + s.now.minute()) as f32 / 60.0 * HOUR_PX;
            let rect = col(i);
            p.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.5, theme::PINK));
            p.circle_filled(Pos2::new(rect.left(), y), 4.0, theme::PINK);
        }
    });
    action
}

fn hm(minutes: i64) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn snap_time(t: NaiveDateTime) -> NaiveDateTime {
    let m = (t.hour() * 60 + t.minute()) as i64;
    t.date().and_time(NaiveTime::MIN) + Duration::minutes(m.div_euclid(SNAP) * SNAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_hour_block_shows_its_notes() {
        assert_eq!(block_notes("Ice wash face", HOUR_PX - 2.0), Some("Ice wash face"));
    }

    #[test]
    fn notes_are_trimmed_and_blank_notes_show_nothing() {
        assert_eq!(block_notes("get ready\n", HOUR_PX), Some("get ready"));
        assert_eq!(block_notes(" \n", HOUR_PX), None);
    }

    #[test]
    fn a_half_hour_block_leaves_notes_to_the_tooltip() {
        assert_eq!(block_notes("watch reels", HOUR_PX / 2.0 - 2.0), None);
        assert_eq!(hover_notes("watch reels\n"), Some("watch reels"));
    }
}
