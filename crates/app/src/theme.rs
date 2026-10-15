//! Cal.diy's dark palette: near-black greys with white as the brand colour.

use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

pub const BG: Color32 = Color32::from_rgb(0x10, 0x10, 0x10);
pub const SURFACE: Color32 = Color32::from_rgb(0x17, 0x17, 0x17);
pub const SURFACE_2: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
pub const LINE: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x2b);
pub const INK: Color32 = Color32::from_rgb(0xf5, 0xf5, 0xf5);
pub const INK_2: Color32 = Color32::from_rgb(0xd4, 0xd4, 0xd4);
pub const INK_3: Color32 = Color32::from_rgb(0x8f, 0x8f, 0x8f);
pub const ACCENT: Color32 = Color32::from_rgb(0xf5, 0xf5, 0xf5);
pub const PINK: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71);
pub const DANGER: Color32 = PINK;

/// Event colours, stored on the event as `#rrggbb`.
pub const SWATCHES: [(&str, &str); 7] = [
    ("Lilac", "#f4c3e8"),
    ("Pink", "#f48aab"),
    ("Mint", "#a5e0ca"),
    ("Cream", "#f5dfb4"),
    ("Blue", "#89b3f9"),
    ("Teal", "#96dfcd"),
    ("Slate", "#b2bad1"),
];

pub fn hex(s: Option<&str>) -> Color32 {
    s.and_then(|s| Color32::from_hex(s).ok()).unwrap_or(Color32::from_rgb(0x89, 0xb3, 0xf9))
}

/// A colour faded into the background, for event fills.
pub fn soft(c: Color32) -> Color32 {
    let mix = |a: u8, b: u8| ((a as u16 * 30 + b as u16 * 70) / 100) as u8;
    Color32::from_rgb(mix(c.r(), BG.r()), mix(c.g(), BG.g()), mix(c.b(), BG.b()))
}

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = SURFACE_2;
    v.faint_bg_color = SURFACE;
    v.override_text_color = Some(INK);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = soft(ACCENT);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    let r = CornerRadius::same(6);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, SURFACE),
        (&mut v.widgets.inactive, SURFACE_2),
        (&mut v.widgets.hovered, LINE),
        (&mut v.widgets.active, soft(ACCENT)),
        (&mut v.widgets.open, SURFACE_2),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, INK_2);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
    });
}
