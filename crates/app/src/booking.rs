//! The booking screen: event types guests can book, the hours they can book
//! them in, and the bookings made so far.

use crate::editor::{accent_button, date_button};
use crate::setup::{self, DayOff, Hours, Span, WEEKDAYS};
use crate::theme;
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use eframe::egui::{self, CornerRadius, Id, RichText};
use lyric_cal_core::db::{Booking, Db, EventType, EventTypeInput};

/// An event type open in the editor: its id when it already exists.
struct TypeEdit {
    id: Option<i64>,
    input: EventTypeInput,
    /// The link name follows the title until it is typed in by hand.
    slug_typed: bool,
    error: Option<String>,
}

pub struct Screen {
    types: Vec<EventType>,
    hours: Hours,
    days_off: Vec<DayOff>,
    bookings: Vec<Booking>,
    edit: Option<TypeEdit>,
    hours_note: Option<Result<String, String>>,
    loaded: bool,
}

/// What the calendar needs to know after a frame of the booking screen.
pub enum Changed {
    Nothing,
    /// A booking was cancelled, so its event left the grid.
    Calendar,
}

impl Screen {
    pub fn new() -> Self {
        Screen {
            types: Vec::new(),
            hours: Hours::default(),
            days_off: Vec::new(),
            bookings: Vec::new(),
            edit: None,
            hours_note: None,
            loaded: false,
        }
    }

    /// Reads everything again when the screen is opened.
    pub fn reload(&mut self) {
        self.loaded = false;
    }

    fn load(&mut self, db: &Db) -> Result<(), String> {
        let e = |e: lyric_cal_core::db::Error| e.to_string();
        self.types = db.event_types().map_err(e)?;
        self.hours = Hours::from_store(&db.weekly_hours().map_err(e)?);
        self.days_off = setup::overrides_from_store(&db.overrides().map_err(e)?);
        self.bookings = db.bookings_from(&Utc::now().to_rfc3339()).map_err(e)?;
        self.hours_note = None;
        self.loaded = true;
        Ok(())
    }

    pub fn show(&mut self, ui: &mut egui::Ui, db: &Db, tz: Tz) -> Result<Changed, String> {
        if !self.loaded {
            self.load(db)?;
        }
        let mut changed = Ok(Changed::Nothing);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let width = ui.available_width().min(760.0);
            let pad = ((ui.available_width() - width) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(8.0);
                    card(ui, "Event types", |ui| self.types_list(ui, db));
                    card(ui, "Weekly hours", |ui| self.hours_editor(ui, db, tz));
                    card(ui, "Upcoming bookings", |ui| changed = self.bookings_list(ui, db, tz));
                    ui.add_space(16.0);
                });
            });
        });
        if let Some(edit) = self.edit.as_mut() {
            match type_editor(ui.ctx(), edit) {
                Some(TypeOutcome::Close) => self.edit = None,
                Some(TypeOutcome::Save) => {
                    let result = match edit.id {
                        Some(id) => db.update_event_type(id, &edit.input),
                        None => db.create_event_type(&edit.input).map(drop),
                    };
                    match result {
                        Ok(()) => (self.edit, self.loaded) = (None, false),
                        Err(e) => edit.error = Some(e.to_string()),
                    }
                }
                Some(TypeOutcome::Delete(id)) => {
                    db.delete_event_type(id).map_err(|e| e.to_string())?;
                    (self.edit, self.loaded) = (None, false);
                }
                None => {}
            }
        }
        changed
    }

    fn types_list(&mut self, ui: &mut egui::Ui, db: &Db) {
        if self.types.is_empty() {
            ui.label(RichText::new("Nothing to book yet. Add a type, like a 30 minute call.").color(theme::INK_3));
        }
        let mut toggled = None;
        for t in &self.types {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                let color = if t.active { theme::hex(t.color.as_deref()) } else { theme::INK_3 };
                ui.painter().circle_filled(dot.center(), 5.0, color);
                let title = RichText::new(&t.title).strong().color(if t.active { theme::INK } else { theme::INK_3 });
                ui.label(title);
                ui.label(RichText::new(format!("{} min · /{}", t.duration_min, t.slug)).color(theme::INK_3));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Edit").clicked() {
                        self.edit =
                            Some(TypeEdit { id: Some(t.id), input: t.input.clone(), slug_typed: true, error: None });
                    }
                    let mut on = t.active;
                    let text = if on { "Open" } else { "Off" };
                    if ui.checkbox(&mut on, text).changed() {
                        toggled = Some((t.id, on));
                    }
                });
            });
        }
        if let Some((id, on)) = toggled {
            if let Some(t) = self.types.iter_mut().find(|t| t.id == id) {
                t.input.active = on;
                if db.update_event_type(id, &t.input).is_err() {
                    self.loaded = false;
                }
            }
        }
        ui.add_space(6.0);
        if accent_button(ui, "+ New type").clicked() {
            self.edit = Some(TypeEdit { id: None, input: new_type(), slug_typed: false, error: None });
        }
    }

    fn hours_editor(&mut self, ui: &mut egui::Ui, db: &Db, tz: Tz) {
        ui.label(RichText::new(format!("Times are in {tz}. Guests see them in their own zone.")).color(theme::INK_3));
        ui.add_space(4.0);
        egui::Grid::new("hours").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
            for (weekday, name) in WEEKDAYS {
                let spans = &mut self.hours.days[weekday as usize];
                let mut open = !spans.is_empty();
                if ui.checkbox(&mut open, name).changed() {
                    *spans = if open { vec![Span::new(9 * 60, 17 * 60)] } else { Vec::new() };
                }
                ui.horizontal_wrapped(|ui| {
                    if spans.is_empty() {
                        ui.label(RichText::new("Closed").color(theme::INK_3));
                    }
                    let mut remove = None;
                    for (i, span) in spans.iter_mut().enumerate() {
                        span_fields(ui, span);
                        if ui.small_button("×").on_hover_text("Remove these hours").clicked() {
                            remove = Some(i);
                        }
                        ui.add_space(8.0);
                    }
                    if let Some(i) = remove {
                        spans.remove(i);
                    }
                    if !spans.is_empty() && ui.small_button("+").on_hover_text("Add hours to this day").clicked() {
                        let from = spans.last().and_then(|s| setup::parse_hhmm(&s.to)).unwrap_or(12 * 60);
                        spans.push(Span::new(from.min(23 * 60), (from + 60).min(24 * 60)));
                    }
                });
                ui.end_row();
            }
        });

        ui.add_space(10.0);
        ui.label(RichText::new("Days off and special hours").strong());
        let today = Utc::now().with_timezone(&tz).date_naive();
        let mut remove = None;
        for (i, d) in self.days_off.iter_mut().enumerate() {
            ui.horizontal_wrapped(|ui| {
                date_button(ui, &format!("off-{i}"), &mut d.date);
                ui.checkbox(&mut d.off, "Unavailable");
                if !d.off {
                    span_fields(ui, &mut d.span);
                }
                if ui.small_button("×").on_hover_text("Remove this date").clicked() {
                    remove = Some(i);
                }
                if d.date < today {
                    ui.label(RichText::new("past").color(theme::INK_3));
                }
            });
        }
        if let Some(i) = remove {
            self.days_off.remove(i);
        }
        if ui.button("+ Add a date").clicked() {
            let date = self.days_off.iter().map(|d| d.date).max().map_or(today, |d| d.max(today));
            self.days_off.push(DayOff { date: date + Duration::days(1), off: true, span: Span::new(9 * 60, 17 * 60) });
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if accent_button(ui, "Save hours").clicked() {
                self.hours_note = Some(self.save_hours(db));
            }
            if ui.button("Undo changes").clicked() {
                self.loaded = false;
            }
            match &self.hours_note {
                Some(Ok(m)) => ui.label(RichText::new(m).color(theme::INK_2)),
                Some(Err(e)) => ui.label(RichText::new(e).color(theme::DANGER)),
                None => ui.label(""),
            };
        });
    }

    fn save_hours(&self, db: &Db) -> Result<String, String> {
        let weekly = self.hours.to_store()?;
        let overrides = setup::overrides_to_store(&self.days_off)?;
        db.set_weekly_hours(&weekly).map_err(|e| e.to_string())?;
        db.set_overrides(&overrides).map_err(|e| e.to_string())?;
        Ok("Saved.".into())
    }

    fn bookings_list(&mut self, ui: &mut egui::Ui, db: &Db, tz: Tz) -> Result<Changed, String> {
        if self.bookings.is_empty() {
            ui.label(RichText::new("No bookings coming up.").color(theme::INK_3));
        }
        let mut cancel = None;
        for b in &self.bookings {
            let cancelled = b.status == "cancelled";
            let ink = if cancelled { theme::INK_3 } else { theme::INK };
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(when(&b.start, &b.end, tz)).strong().color(ink));
                ui.label(RichText::new(format!("{} · {}", b.type_title, b.guest_name)).color(ink));
                ui.label(RichText::new(&b.guest_email).color(theme::INK_3));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if cancelled {
                        ui.label(RichText::new("cancelled").color(theme::INK_3));
                    } else if ui.button(RichText::new("Cancel").color(theme::DANGER)).clicked() {
                        cancel = Some(b.id);
                    }
                });
            });
            if !b.guest_note.is_empty() {
                ui.label(RichText::new(format!("“{}”", b.guest_note)).italics().color(theme::INK_2));
            }
            ui.add_space(4.0);
        }
        let Some(id) = cancel else { return Ok(Changed::Nothing) };
        db.cancel_booking(id).map_err(|e| e.to_string())?;
        self.loaded = false;
        Ok(Changed::Calendar)
    }
}

/// A booking's time in the organizer's zone, like "Tue 20 Oct, 15:00–15:30".
fn when(start: &str, end: &str, tz: Tz) -> String {
    let local = |s: &str| DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&tz));
    match (local(start), local(end)) {
        (Some(s), Some(e)) => format!("{}–{}", s.format("%a %-d %b, %H:%M"), e.format("%H:%M")),
        _ => start.to_string(),
    }
}

fn new_type() -> EventTypeInput {
    EventTypeInput {
        slug: String::new(),
        title: String::new(),
        description: String::new(),
        duration_min: 30,
        buffer_before_min: 0,
        buffer_after_min: 0,
        min_notice_min: 240,
        max_days_ahead: 60,
        color: None,
        active: true,
    }
}

fn card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(theme::SURFACE).corner_radius(CornerRadius::same(10)).inner_margin(16.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).size(17.0).color(theme::ACCENT));
        ui.add_space(8.0);
        body(ui);
    });
    ui.add_space(12.0);
}

fn span_fields(ui: &mut egui::Ui, span: &mut Span) {
    fn field(s: &mut String) -> egui::TextEdit<'_> {
        egui::TextEdit::singleline(s).desired_width(46.0).horizontal_align(egui::Align::Center)
    }
    ui.add(field(&mut span.from));
    ui.label("–");
    ui.add(field(&mut span.to));
}

enum TypeOutcome {
    Save,
    Delete(i64),
    Close,
}

fn type_editor(ctx: &egui::Context, edit: &mut TypeEdit) -> Option<TypeOutcome> {
    let mut out = None;
    let modal = egui::Modal::new(Id::new("type-editor"))
        .frame(egui::Frame::window(&ctx.global_style()).inner_margin(20.0))
        .show(ctx, |ui| {
            ui.set_width(420.0);
            let t = &mut edit.input;
            let title = egui::TextEdit::singleline(&mut t.title)
                .hint_text("Name it, like Intro call")
                .font(egui::FontId::proportional(20.0))
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY);
            if ui.add(title).changed() && !edit.slug_typed {
                t.slug = setup::slugify(&t.title);
            }
            ui.add_space(10.0);
            egui::Grid::new("type-grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                fn label(ui: &mut egui::Ui, s: &str) {
                    ui.label(RichText::new(s).color(theme::INK_2));
                }
                label(ui, "Link");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("/").color(theme::INK_3));
                    if ui.add(egui::TextEdit::singleline(&mut t.slug).desired_width(200.0)).changed() {
                        edit.slug_typed = true;
                    }
                });
                ui.end_row();
                label(ui, "Length");
                minutes(ui, &mut t.duration_min, 5..=24 * 60, " min");
                ui.end_row();
                label(ui, "Gap before");
                minutes(ui, &mut t.buffer_before_min, 0..=240, " min");
                ui.end_row();
                label(ui, "Gap after");
                minutes(ui, &mut t.buffer_after_min, 0..=240, " min");
                ui.end_row();
                label(ui, "Notice");
                let mut hours = t.min_notice_min / 60;
                if minutes(ui, &mut hours, 0..=24 * 14, " h ahead").changed() {
                    t.min_notice_min = hours * 60;
                }
                ui.end_row();
                label(ui, "Bookable");
                minutes(ui, &mut t.max_days_ahead, 1..=365, " days out");
                ui.end_row();
                label(ui, "Color");
                ui.horizontal(|ui| {
                    for (name, hex) in theme::SWATCHES {
                        let picked = t.color.as_deref() == Some(hex);
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                        ui.painter().circle_filled(r.center(), if picked { 9.0 } else { 7.0 }, theme::hex(Some(hex)));
                        if resp.on_hover_text(name).clicked() {
                            t.color = Some(hex.to_string());
                        }
                    }
                });
                ui.end_row();
                label(ui, "About");
                ui.add(
                    egui::TextEdit::multiline(&mut t.description)
                        .hint_text("Shown to guests on the booking page")
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                ui.end_row();
            });
            if let Some(e) = &edit.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(theme::DANGER));
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if let Some(id) = edit.id {
                    if ui.button(RichText::new("Delete").color(theme::DANGER)).clicked() {
                        out = Some(TypeOutcome::Delete(id));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if accent_button(ui, "Save").clicked() {
                        out = Some(TypeOutcome::Save);
                    }
                    if ui.button("Cancel").clicked() {
                        out = Some(TypeOutcome::Close);
                    }
                });
            });
        });
    if out.is_none() && modal.should_close() {
        out = Some(TypeOutcome::Close);
    }
    out
}

fn minutes(ui: &mut egui::Ui, value: &mut i64, range: std::ops::RangeInclusive<i64>, suffix: &str) -> egui::Response {
    ui.add(egui::DragValue::new(value).range(range).suffix(suffix).speed(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booking_times_read_in_the_organizers_zone() {
        let tz: Tz = "Asia/Dhaka".parse().unwrap();
        assert_eq!(when("2026-10-20T09:00:00Z", "2026-10-20T09:30:00Z", tz), "Tue 20 Oct, 15:00–15:30");
        assert_eq!(when("garbage", "2026-10-20T09:30:00Z", tz), "garbage");
    }
}
