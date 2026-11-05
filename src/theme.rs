//! Padd's look: one palette per mode, applied to every egui widget style.
//!
//! Dark: deep slate with a warm LCARS-amber accent and a lavender secondary.
//! Light: warm paper with the same accents, toned down for contrast.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Vec2,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// Chrome: menu bar, tab strip, status bar.
    pub panel: Color32,
    /// Editor surface (and the active tab).
    pub bg: Color32,
    /// Inputs and wells, a step away from `bg`.
    pub well: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub strong: Color32,
    pub heading: Color32,
    pub weak: Color32,
    pub code: Color32,
    pub code_bg: Color32,
    pub link: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    /// Text drawn on an `accent` fill.
    pub on_accent: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                dark,
                panel: rgb(0x15171C),
                bg: rgb(0x1B1E24),
                well: rgb(0x111318),
                hover: rgb(0x262A32),
                border: rgb(0x2A2E37),
                text: rgb(0xD5D8DF),
                strong: rgb(0xF4F5F7),
                heading: rgb(0xF7F8FA),
                weak: rgb(0x6F7685),
                code: rgb(0xEAC27E),
                code_bg: rgb(0x252930),
                link: rgb(0x7DB9FF),
                accent: rgb(0xF2A65A),
                accent2: rgb(0xA9A7F5),
                on_accent: rgb(0x1A1407),
            }
        } else {
            Self {
                dark,
                panel: rgb(0xEDEDE9),
                bg: rgb(0xFBFBF9),
                well: rgb(0xFFFFFF),
                hover: rgb(0xE2E2DD),
                border: rgb(0xD9D9D3),
                text: rgb(0x2A2E35),
                strong: rgb(0x0F1115),
                heading: rgb(0x14171C),
                weak: rgb(0x8A909B),
                code: rgb(0xA0501A),
                code_bg: rgb(0xF0EFEA),
                link: rgb(0x1F6FD1),
                accent: rgb(0xD5731C),
                accent2: rgb(0x6A65D8),
                on_accent: rgb(0xFFFFFF),
            }
        }
    }

    pub fn of(ui: &egui::Ui) -> Self {
        Self::new(ui.visuals().dark_mode)
    }
}

/// Install Padd's style for both light and dark mode.
pub fn install(ctx: &egui::Context) {
    for theme in [Theme::Dark, Theme::Light] {
        let pal = Palette::new(theme == Theme::Dark);
        ctx.style_mut_of(theme, |s| style(s, &pal));
    }
    ctx.options_mut(|o| o.zoom_with_keyboard = false); // Ctrl +/- sizes the editor text instead
}

fn style(s: &mut egui::Style, pal: &Palette) {
    use FontFamily::{Monospace, Proportional};
    s.text_styles = [
        (TextStyle::Small, FontId::new(11.5, Proportional)),
        (TextStyle::Body, FontId::new(14.0, Proportional)),
        (TextStyle::Button, FontId::new(14.0, Proportional)),
        (TextStyle::Heading, FontId::new(22.0, Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, Monospace)),
    ]
    .into();

    let sp = &mut s.spacing;
    sp.item_spacing = Vec2::new(8.0, 6.0);
    sp.button_padding = Vec2::new(10.0, 4.0);
    sp.menu_margin = Margin::same(8);
    sp.window_margin = Margin::same(18);
    sp.interact_size.y = 24.0;
    sp.scroll = egui::style::ScrollStyle::floating();

    let v = &mut s.visuals;
    v.panel_fill = pal.panel;
    v.window_fill = pal.bg;
    v.extreme_bg_color = pal.well;
    v.text_edit_bg_color = Some(pal.well);
    v.faint_bg_color = pal.hover;
    v.code_bg_color = pal.code_bg;
    v.hyperlink_color = pal.link;
    v.window_stroke = Stroke::new(1.0, pal.border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    let shadow = Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(if pal.dark { 110 } else { 40 }),
    };
    v.window_shadow = shadow;
    v.popup_shadow = Shadow { blur: 18, ..shadow };
    v.selection.bg_fill = pal.accent.gamma_multiply(if pal.dark { 0.32 } else { 0.28 });
    v.selection.stroke = Stroke::new(1.0, pal.accent);
    v.text_cursor.stroke = Stroke::new(2.0, pal.accent);
    v.weak_text_color = Some(pal.weak);
    v.indent_has_left_vline = false;

    let r = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = pal.panel;
    w.noninteractive.weak_bg_fill = pal.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, pal.text);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = pal.hover;
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, pal.text);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = pal.hover;
    w.hovered.weak_bg_fill = pal.hover;
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.5, pal.strong);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = pal.accent.gamma_multiply(0.35);
    w.active.weak_bg_fill = pal.accent.gamma_multiply(0.25);
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(1.5, pal.strong);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open = w.hovered;
}
