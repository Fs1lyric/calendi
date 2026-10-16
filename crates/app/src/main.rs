//! Calendi, the desktop calendar.

mod booking;
mod editor;
mod form;
mod grid;
mod setup;
mod theme;
mod views;

use chrono::{Duration, NaiveDate, NaiveDateTime, Timelike, Utc};
use chrono_tz::Tz;
use editor::{accent_button, Editor, Outcome};
use eframe::egui::{self, Key, RichText};
use form::{Form, Save, Scope};
use grid::View;
use lyric_cal_core::db::Db;
use lyric_cal_core::recur::Occurrence;
use views::{Action, Scene};

struct App {
    db: Db,
    tz: Tz,
    view: View,
    anchor: NaiveDate,
    occ: Vec<Occurrence>,
    stale: bool,
    loaded_at: std::time::Instant,
    scroll_to: Option<f32>,
    editor: Option<Editor>,
    /// The booking screen replaces the calendar while it is open.
    booking: Option<booking::Screen>,
    error: Option<String>,
}

impl App {
    fn new(db: Db, tz: Tz) -> Self {
        App {
            db,
            tz,
            view: View::Week,
            anchor: Utc::now().with_timezone(&tz).date_naive(),
            occ: Vec::new(),
            stale: true,
            loaded_at: std::time::Instant::now(),
            scroll_to: Some(8.0 * views::HOUR_PX),
            editor: None,
            booking: None,
            error: None,
        }
    }

    fn now(&self) -> NaiveDateTime {
        Utc::now().with_timezone(&self.tz).naive_local()
    }

    /// Reloads after a change, and every 30 s so bookings made by the booker show up.
    fn reload(&mut self) {
        if !self.stale && self.loaded_at.elapsed().as_secs() < 30 {
            return;
        }
        let (from, to) = grid::window(&grid::days(self.view, self.anchor), self.tz);
        match self.db.occurrences_between(&from.to_rfc3339(), &to.to_rfc3339()) {
            Ok(occ) => self.occ = occ,
            Err(e) => self.error = Some(e.to_string()),
        }
        self.stale = false;
        self.loaded_at = std::time::Instant::now();
    }

    fn go(&mut self, view: View, anchor: NaiveDate) {
        self.booking = None;
        if view != self.view && view != View::Month {
            self.scroll_to = Some(8.0 * views::HOUR_PX);
        }
        (self.view, self.anchor, self.stale) = (view, anchor, true);
    }

    fn open(&mut self, index: usize) {
        let o = &self.occ[index];
        match self.db.event(o.event_id) {
            Ok(Some(e)) => self.editor = Some(Editor::new(Form::open(o, &e, self.tz), Some(e))),
            Ok(None) => self.stale = true,
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn apply(&self, save: &Save) -> lyric_cal_core::db::Result<()> {
        match save {
            Save::Create(e) => self.db.create_event(e).map(drop),
            Save::Update(id, e) => self.db.update_event(*id, e),
            Save::Exception(x) => self.db.set_exception(x),
        }
    }

    /// A drag drop: a single event moves, a series moves just this occurrence.
    fn drop_at(&mut self, index: usize, start: NaiveDateTime) {
        let o = &self.occ[index];
        let Ok(Some(e)) = self.db.event(o.event_id) else { return };
        let mut f = Form::open(o, &e, self.tz);
        let length = f.end - f.start;
        (f.start, f.end) = (start, start + length);
        let result = f.save(Scope::One, Some(&e)).map_err(|m| m.to_string());
        match result.and_then(|s| self.apply(&s).map_err(|e| e.to_string())) {
            Ok(()) => self.stale = true,
            Err(e) => self.error = Some(e),
        }
    }

    /// Navigation and the period on the left, view switch and "New" on the
    /// right. Narrow windows put the two halves on separate rows.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let narrow = ui.available_width() < 760.0;
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            if !narrow {
                ui.label(RichText::new("Calendi").strong().color(theme::ACCENT).size(16.0));
                ui.add_space(12.0);
            }
            if self.booking.is_some() {
                ui.label(RichText::new("Booking").size(if narrow { 15.0 } else { 18.0 }));
            } else if ui.button("Today").clicked() {
                self.go(self.view, self.now().date());
            }
            if self.booking.is_none() {
                if ui.button("‹").clicked() {
                    self.go(self.view, grid::step(self.view, self.anchor, -1));
                }
                if ui.button("›").clicked() {
                    self.go(self.view, grid::step(self.view, self.anchor, 1));
                }
                let title = RichText::new(grid::title(self.view, self.anchor)).size(if narrow { 15.0 } else { 18.0 });
                ui.add(egui::Label::new(title).truncate());
            }
            if !narrow {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.switcher(ui));
            }
        });
        if narrow {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.switcher(ui));
            });
        }
    }

    /// Right to left: "New", Booking, then Month / Week / Day.
    fn switcher(&mut self, ui: &mut egui::Ui) {
        if accent_button(ui, "+ New").clicked() {
            self.new_event();
        }
        ui.add_space(8.0);
        if ui.selectable_label(self.booking.is_some(), "Booking").clicked() {
            self.open_booking();
        }
        ui.add_space(8.0);
        for (v, name) in [(View::Day, "Day"), (View::Week, "Week"), (View::Month, "Month")] {
            if ui.selectable_label(self.view == v && self.booking.is_none(), name).clicked() {
                self.go(v, self.anchor);
            }
        }
    }

    fn open_booking(&mut self) {
        self.booking.get_or_insert_with(booking::Screen::new).reload();
    }

    fn new_event(&mut self) {
        // Today: the next full hour. Another day: nine o'clock.
        let now = self.now();
        let start = if self.anchor == now.date() {
            now.date().and_hms_opt(now.hour(), 0, 0).unwrap() + Duration::hours(1)
        } else {
            self.anchor.and_hms_opt(9, 0, 0).unwrap()
        };
        self.editor = Some(Editor::new(Form::new(start, start + Duration::hours(1), false, self.tz), None));
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if self.editor.is_some() || ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let pressed = |k| ctx.input(|i| i.key_pressed(k));
        if pressed(Key::T) {
            self.go(self.view, self.now().date());
        }
        if pressed(Key::M) {
            self.go(View::Month, self.anchor);
        }
        if pressed(Key::W) {
            self.go(View::Week, self.anchor);
        }
        if pressed(Key::D) {
            self.go(View::Day, self.anchor);
        }
        if pressed(Key::N) {
            self.new_event();
        }
        if pressed(Key::B) {
            self.open_booking();
        }
        if pressed(Key::ArrowLeft) || pressed(Key::J) {
            self.go(self.view, grid::step(self.view, self.anchor, -1));
        }
        if pressed(Key::ArrowRight) || pressed(Key::K) {
            self.go(self.view, grid::step(self.view, self.anchor, 1));
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.keys(&ctx);
        self.reload();
        ctx.request_repaint_after(std::time::Duration::from_secs(30));

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(8.0);
            self.top_bar(ui);
            if let Some(e) = self.error.clone() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(e).color(theme::DANGER));
                    if ui.small_button("Dismiss").clicked() {
                        self.error = None;
                    }
                });
            }
            ui.add_space(6.0);
        });
        if let Some(screen) = self.booking.as_mut() {
            let result = egui::CentralPanel::default().show(ui, |ui| screen.show(ui, &self.db, self.tz)).inner;
            match result {
                Ok(booking::Changed::Calendar) => self.stale = true,
                Ok(booking::Changed::Nothing) => {}
                Err(e) => self.error = Some(e),
            }
        } else {
            self.calendar(ui);
        }
        self.editor_frame(&ctx);
    }
}

impl App {
    fn calendar(&mut self, ui: &mut egui::Ui) {
        let action = egui::CentralPanel::default()
            .show(ui, |ui| {
                let scene = Scene { occ: &self.occ, tz: self.tz, now: self.now() };
                match self.view {
                    View::Month => views::month(ui, &scene, self.anchor),
                    v => views::time_grid(ui, &grid::days(v, self.anchor), &scene, self.scroll_to.take()),
                }
            })
            .inner;

        match action {
            Some(Action::Open(k)) => self.open(k),
            Some(Action::New { start, end, all_day }) => {
                self.editor = Some(Editor::new(Form::new(start, end, all_day, self.tz), None));
            }
            Some(Action::Move { index, start }) => self.drop_at(index, start),
            Some(Action::Goto(day)) => self.go(View::Day, day),
            None => {}
        }
    }

    fn editor_frame(&mut self, ctx: &egui::Context) {
        let Some(ed) = self.editor.as_mut() else { return };
        let outcome = ed.show(ctx);
        let result = match &outcome {
            None => return,
            Some(Outcome::Close) => Ok(()),
            Some(Outcome::Save(s)) => self.apply(s),
            Some(Outcome::DeleteEvent(id)) => self.db.delete_event(*id),
            Some(Outcome::DeleteOne(x)) => self.db.set_exception(x),
        };
        match result {
            Ok(()) => {
                self.editor = None;
                self.stale = true;
            }
            Err(e) => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.error = Some(e.to_string());
                }
            }
        }
    }
}

const USAGE: &str = "usage: calendi [--view month|week|day] [--booking] [--new]";

/// Vsync stays off: on Wayland a vsynced swap waits for a frame callback, and Hyprland
/// sends none to a window on a hidden workspace, so the 30s repaint would block the
/// main thread there until the compositor reports Calendi as not responding.
fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Calendi")
            .with_app_id("calendi")
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([360.0, 320.0]),
        glow_options: eframe::egui_glow::GlowConfiguration { vsync: false, ..Default::default() },
        ..Default::default()
    }
}

fn main() -> eframe::Result {
    let mut view = View::Week;
    let mut quick_add = false;
    let mut booking = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--view" => {
                view = match args.next().as_deref() {
                    Some("month") => View::Month,
                    Some("week") => View::Week,
                    Some("day") => View::Day,
                    _ => {
                        eprintln!("{USAGE}");
                        std::process::exit(2);
                    }
                }
            }
            "--new" => quick_add = true,
            "--booking" => booking = true,
            _ => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let tz: Tz = iana_time_zone::get_timezone().ok().and_then(|z| z.parse().ok()).unwrap_or(Tz::UTC);
    let db = match Db::open(&Db::default_path()) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("calendi: can't open the calendar: {e}");
            std::process::exit(1);
        }
    };
    eframe::run_native(
        "Calendi",
        native_options(),
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            let mut app = App::new(db, tz);
            app.view = view;
            if booking {
                app.open_booking();
            }
            if quick_add {
                app.new_event();
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn swaps_never_wait_on_the_compositor() {
        assert!(!super::native_options().glow_options.vsync);
    }
}
