//! Window chrome: menu bar, tab strip, status bar, welcome and lock screens.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Key, Layout, Margin, Pos2, Rect, RichText, Sense,
    Stroke, ViewportCommand, pos2, vec2,
};

use super::{Body, Dialog, Kind, PaddApp};
use crate::settings::{TEXT_SIZE_DEFAULT, ThemeChoice};
use crate::theme::Palette;

const TAB_H: f32 = 34.0;

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Eye,
    EyeOff,
    Copy,
    Lock,
    Close,
    Plus,
    Search,
    Up,
    Down,
}

/// Draw a vector icon centred in `rect`: identical on every OS, no icon font needed.
pub(super) fn paint_icon(p: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = Stroke::new(1.4, color);
    match icon {
        Icon::Eye | Icon::EyeOff => {
            p.add(egui::Shape::ellipse_stroke(c, vec2(7.0, 4.3), s));
            p.circle_filled(c, 2.1, color);
            if matches!(icon, Icon::EyeOff) {
                p.line_segment([c + vec2(-7.0, 5.0), c + vec2(7.0, -5.0)], s);
            }
        }
        Icon::Copy => {
            let back = Rect::from_min_size(c + vec2(-5.5, -5.5), vec2(7.0, 8.5));
            let front = back.translate(vec2(3.0, 3.0));
            p.rect_stroke(back, 1.5, s, egui::StrokeKind::Inside);
            p.rect_filled(front.expand(1.0), 2.0, Color32::TRANSPARENT);
            p.rect_stroke(front, 1.5, s, egui::StrokeKind::Inside);
        }
        Icon::Lock => {
            let body = Rect::from_center_size(c + vec2(0.0, 1.8), vec2(9.0, 7.0));
            p.rect_filled(body, 1.5, color);
            let r = 3.0;
            let top = body.top();
            let mut pts = vec![pos2(c.x - r, top)];
            for k in 0..=12 {
                let a = std::f32::consts::PI * (1.0 - k as f32 / 12.0);
                pts.push(pos2(c.x + r * a.cos(), top - 2.5 - r * a.sin()));
            }
            pts.push(pos2(c.x + r, top));
            p.add(egui::Shape::line(pts, Stroke::new(1.5, color)));
        }
        Icon::Close => {
            let d = 3.8;
            p.line_segment([c + vec2(-d, -d), c + vec2(d, d)], s);
            p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], s);
        }
        Icon::Plus => {
            let d = 5.0;
            p.line_segment([c + vec2(-d, 0.0), c + vec2(d, 0.0)], s);
            p.line_segment([c + vec2(0.0, -d), c + vec2(0.0, d)], s);
        }
        Icon::Search => {
            p.circle_stroke(c + vec2(-1.5, -1.5), 4.2, s);
            p.line_segment([c + vec2(1.6, 1.6), c + vec2(5.0, 5.0)], Stroke::new(1.8, color));
        }
        Icon::Up | Icon::Down => {
            let k = if matches!(icon, Icon::Up) { -1.0 } else { 1.0 };
            p.line_segment([c + vec2(-4.0, -2.0 * k), c + vec2(0.0, 2.0 * k)], s);
            p.line_segment([c + vec2(0.0, 2.0 * k), c + vec2(4.0, -2.0 * k)], s);
        }
    }
}

/// A small icon button with a soft hover background.
pub(super) fn icon_button(ui: &mut egui::Ui, rect: Rect, id: egui::Id, icon: Icon, tip: &str) -> bool {
    let pal = Palette::of(ui);
    let resp = ui
        .interact(rect, id, Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, pal.hover);
    }
    let color = if resp.hovered() { pal.strong } else { pal.weak };
    paint_icon(ui.painter(), rect, icon, color);
    resp.clicked()
}

/// The call-to-action button: accent fill.
pub(super) fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let pal = Palette::of(ui);
    ui.add(
        egui::Button::new(RichText::new(text).color(pal.on_accent).strong())
            .fill(pal.accent)
            .corner_radius(7.0)
            .min_size(vec2(96.0, 30.0)),
    )
}

pub(super) fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let pal = Palette::of(ui);
    ui.add(
        egui::Button::new(text)
            .fill(pal.hover)
            .corner_radius(7.0)
            .min_size(vec2(96.0, 30.0)),
    )
}

/// A full-width row for the welcome screen: label left, hint right.
fn action_row(ui: &mut egui::Ui, label: &str, hint: &str, width: f32) -> egui::Response {
    let pal = Palette::of(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 7.0, pal.hover);
    }
    let y = rect.center().y;
    ui.painter().text(
        pos2(rect.left() + 12.0, y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.5),
        if resp.hovered() { pal.strong } else { pal.text },
    );
    ui.painter().text(
        pos2(rect.right() - 12.0, y),
        Align2::RIGHT_CENTER,
        hint,
        FontId::proportional(12.5),
        pal.weak,
    );
    resp
}

fn shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        key.replace("Ctrl+", "⌘").replace("Shift+", "⇧")
    } else {
        key.to_owned()
    }
}

impl PaddApp {
    /// Lay out the whole window.
    pub(super) fn chrome(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui);
        let bar = egui::Frame::NONE.fill(pal.panel);

        egui::Panel::top("padd-menu")
            .frame(bar.inner_margin(Margin {
                left: 8,
                right: 8,
                top: 4,
                bottom: 0,
            }))
            .show_separator_line(false)
            .show(ui, |ui| self.menu_bar(ui));
        if !self.tabs.is_empty() {
            egui::Panel::top("padd-tabs")
                .frame(bar.inner_margin(Margin {
                    left: 6,
                    right: 6,
                    top: 4,
                    bottom: 0,
                }))
                .show_separator_line(false)
                .exact_size(TAB_H + 4.0)
                .show(ui, |ui| self.tab_bar(ui));
        }
        egui::Panel::bottom("padd-status")
            .frame(
                bar.inner_margin(Margin::symmetric(12, 3))
                    .stroke(Stroke::new(1.0, pal.border)),
            )
            .show_separator_line(false)
            .exact_size(24.0)
            .show(ui, |ui| self.status_bar(ui));

        egui::CentralPanel::no_frame()
            .frame(egui::Frame::NONE.fill(pal.bg))
            .show(ui, |ui| {
                if self.tabs.is_empty() {
                    self.welcome(ui);
                    return;
                }
                let idx = self.active.min(self.tabs.len() - 1);
                self.active = idx;
                match self.tabs[idx].body {
                    Body::Locked { .. } => self.locked_view(ui, idx),
                    Body::Large { .. } => self.large_view(ui, idx),
                    Body::Ready => {
                        if self.find.is_some() {
                            self.find_bar(ui, idx);
                        }
                        self.editor(ui, idx);
                    }
                }
            });
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = Palette::of(ui);
        egui::MenuBar::new().ui(ui, |ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            ui.painter().image(
                self.logo.id(),
                r,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ui.add_space(2.0);
            ui.menu_button("File", |ui| {
                ui.set_min_width(240.0);
                let item = |label: &str, key: &str| egui::Button::new(label).shortcut_text(shortcut(key));
                if ui.add(item("New", "Ctrl+N")).clicked() {
                    self.new_tab(false);
                }
                if ui.button("New secrets file").clicked() {
                    self.new_tab(true);
                }
                if ui.add(item("Open…", "Ctrl+O")).clicked() {
                    self.open_dialog(&ctx);
                }
                ui.add_enabled_ui(!self.settings.recent.is_empty(), |ui| {
                    ui.menu_button("Open recent", |ui| {
                        ui.set_min_width(280.0);
                        let mut open = None;
                        for p in &self.settings.recent {
                            let name = super::file_name(p);
                            let dir = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
                            if ui.add(egui::Button::new(name).shortcut_text(dir)).clicked() {
                                open = Some(p.clone());
                            }
                        }
                        ui.separator();
                        if ui.button("Clear list").clicked() {
                            self.settings.recent.clear();
                        }
                        if let Some(p) = open {
                            self.open_path(&ctx, p);
                        }
                    });
                });
                ui.separator();
                let has = !self.tabs.is_empty();
                if ui.add_enabled(has, item("Save", "Ctrl+S")).clicked() {
                    self.save(&ctx, self.active, false);
                }
                if ui.add_enabled(has, item("Save as…", "Ctrl+Shift+S")).clicked() {
                    self.save(&ctx, self.active, true);
                }
                ui.separator();
                let (enc, ready) = self
                    .tabs
                    .get(self.active)
                    .map_or((false, false), |t| (t.encrypted(), matches!(t.body, Body::Ready)));
                let label = if enc {
                    "Change password…"
                } else {
                    "Encrypt with password…"
                };
                if ui.add_enabled(has && ready, egui::Button::new(label)).clicked() {
                    let t = &self.tabs[self.active];
                    self.dialog = Some(Dialog::SetPassword {
                        tab: t.id,
                        p1: String::new(),
                        p2: String::new(),
                        error: None,
                        delete_plain: false,
                        has_plain: !enc && t.path.is_some(),
                    });
                }
                if ui.add_enabled(enc && ready, item("Lock", "Ctrl+L")).clicked() {
                    self.lock(&ctx, self.active);
                }
                ui.separator();
                if ui.add_enabled(has, item("Close tab", "Ctrl+W")).clicked() {
                    self.close_tab(&ctx, self.active, false);
                }
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                ui.set_min_width(220.0);
                let ready = self
                    .tabs
                    .get(self.active)
                    .is_some_and(|t| matches!(t.body, Body::Ready));
                if ui
                    .add_enabled(
                        ready,
                        egui::Button::new("Find…").shortcut_text(shortcut("Ctrl+F")),
                    )
                    .clicked()
                {
                    self.open_find();
                }
            });
            ui.menu_button("View", |ui| {
                ui.set_min_width(260.0);
                ui.label(RichText::new("Text size").small().color(pal.weak));
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(RichText::new("A−").size(13.0)).min_size(vec2(34.0, 26.0)))
                        .clicked()
                    {
                        self.set_text_size(self.settings.text_size - 1.0);
                    }
                    let mut size = self.settings.text_size;
                    let r = ui.add(
                        egui::Slider::new(
                            &mut size,
                            crate::settings::TEXT_SIZE_MIN..=crate::settings::TEXT_SIZE_MAX,
                        )
                        .step_by(0.5)
                        .show_value(false),
                    );
                    if r.changed() {
                        self.set_text_size(size);
                    }
                    if ui
                        .add(egui::Button::new(RichText::new("A+").size(16.0)).min_size(vec2(34.0, 26.0)))
                        .clicked()
                    {
                        self.set_text_size(self.settings.text_size + 1.0);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} pt", self.settings.text_size)).color(pal.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                self.settings.text_size != TEXT_SIZE_DEFAULT,
                                egui::Button::new("Reset").shortcut_text(shortcut("Ctrl+0")),
                            )
                            .clicked()
                        {
                            self.set_text_size(TEXT_SIZE_DEFAULT);
                        }
                    });
                });
                ui.separator();
                ui.label(RichText::new("Interface size").small().color(pal.weak));
                ui.horizontal(|ui| {
                    for (label, z) in [("90%", 0.9), ("100%", 1.0), ("115%", 1.15), ("130%", 1.3)] {
                        let on = (self.settings.ui_scale - z).abs() < 0.01;
                        if ui.selectable_label(on, label).clicked() {
                            self.settings.ui_scale = z;
                        }
                    }
                });
                ui.separator();
                ui.label(RichText::new("Theme").small().color(pal.weak));
                ui.horizontal(|ui| {
                    for (label, t) in [
                        ("System", ThemeChoice::System),
                        ("Light", ThemeChoice::Light),
                        ("Dark", ThemeChoice::Dark),
                    ] {
                        if ui.selectable_label(self.settings.theme == t, label).clicked() {
                            self.settings.theme = t;
                        }
                    }
                });
                ui.separator();
                ui.checkbox(&mut self.settings.live_preview, "Live Markdown preview")
                    .on_hover_text(shortcut("Ctrl+E"));
                ui.checkbox(&mut self.settings.readable_width, "Comfortable line width");
                ui.checkbox(&mut self.settings.wrap, "Word wrap");
            });
            ui.menu_button("Help", |ui| {
                ui.set_min_width(200.0);
                if ui.button("Keyboard shortcuts").clicked() {
                    self.dialog = Some(Dialog::Shortcuts);
                }
                if ui.button("About Padd").clicked() {
                    self.dialog = Some(Dialog::About);
                }
            });
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = Palette::of(ui);
        let mut close = None;
        let mut activate = None;
        egui::ScrollArea::horizontal()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (i, t) in self.tabs.iter().enumerate() {
                        let active = i == self.active;
                        let enc = t.encrypted();
                        let font = FontId::proportional(13.5);
                        let color = if active { pal.strong } else { pal.weak };
                        let mut job = egui::text::LayoutJob::simple_singleline(t.name(), font, color);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(200.0);
                        let galley = ui.painter().layout_job(job);
                        let lead = if enc { 30.0 } else { 14.0 };
                        let w = (lead + galley.size().x + 34.0).max(96.0);
                        let (rect, resp) = ui.allocate_exact_size(vec2(w, TAB_H), Sense::click());
                        let hovered = resp.hovered();
                        let p = ui.painter();
                        if active {
                            p.rect_filled(
                                rect,
                                CornerRadius {
                                    nw: 8,
                                    ne: 8,
                                    sw: 0,
                                    se: 0,
                                },
                                pal.bg,
                            );
                            // LCARS touch: a short accent bar on the active tab
                            let bar = Rect::from_min_size(
                                rect.left_top() + vec2(10.0, 0.0),
                                vec2(rect.width() - 20.0, 2.0),
                            );
                            p.rect_filled(bar, 1.0, pal.accent);
                        } else if hovered {
                            p.rect_filled(rect.shrink2(vec2(0.0, 4.0)), 7.0, pal.hover);
                        }
                        let mut x = rect.left() + 14.0;
                        if enc {
                            let r = Rect::from_center_size(pos2(x + 6.0, rect.center().y), vec2(12.0, 14.0));
                            paint_icon(p, r, Icon::Lock, if active { pal.accent2 } else { pal.weak });
                            x += 16.0;
                        }
                        p.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);

                        // close button, or the unsaved dot
                        let cr = Rect::from_center_size(
                            pos2(rect.right() - 16.0, rect.center().y),
                            vec2(20.0, 20.0),
                        );
                        let cresp = ui.interact(cr, ui.scope_id().with(("tab-close", t.id)), Sense::click());
                        if cresp.hovered() {
                            ui.painter().rect_filled(cr, 5.0, pal.hover);
                        }
                        if t.dirty && !cresp.hovered() {
                            ui.painter().circle_filled(
                                cr.center(),
                                3.5,
                                if active { pal.accent } else { pal.weak },
                            );
                        } else if active || hovered || cresp.hovered() {
                            paint_icon(
                                ui.painter(),
                                cr,
                                Icon::Close,
                                if cresp.hovered() { pal.strong } else { pal.weak },
                            );
                        }
                        if cresp.clicked() {
                            close = Some(i);
                        } else if resp.clicked() {
                            activate = Some(i);
                        }
                        if resp.middle_clicked() {
                            close = Some(i);
                        }
                        if let Some(path) = &t.path {
                            resp.on_hover_text(path.display().to_string());
                        }
                    }
                    let (r, _) = ui.allocate_exact_size(vec2(30.0, TAB_H), Sense::hover());
                    let r = Rect::from_center_size(r.center(), vec2(24.0, 24.0));
                    if icon_button(
                        ui,
                        r,
                        ui.scope_id().with("tab-new"),
                        Icon::Plus,
                        &shortcut("New tab  Ctrl+N"),
                    ) {
                        activate = Some(usize::MAX);
                    }
                });
            });
        match activate {
            Some(usize::MAX) => self.new_tab(false),
            Some(i) => self.active = i,
            None => {}
        }
        if let Some(i) = close {
            self.close_tab(&ctx, i, false);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui);
        let small = |s: String| RichText::new(s).size(12.0).color(pal.weak);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            if let Some(t) = self.tabs.get(self.active) {
                if matches!(t.body, Body::Ready) {
                    ui.label(small(format!("Ln {}, Col {}", t.cursor.0, t.cursor.1)));
                    if t.kind == Kind::Markdown && t.text.len() < 2_000_000 {
                        let words = t.text.split_whitespace().count();
                        ui.label(small(format!("{words} words")));
                    }
                }
                let kind = match t.kind {
                    Kind::Markdown => "Markdown",
                    Kind::Code => "Code",
                    Kind::Plain => "Text",
                };
                ui.label(small(kind.into()));
                ui.label(small(
                    match t.fmt.encoding {
                        crate::fileio::Encoding::Utf8 => "UTF-8",
                        crate::fileio::Encoding::Utf8Bom => "UTF-8 BOM",
                        crate::fileio::Encoding::Utf16Le => "UTF-16 LE",
                        crate::fileio::Encoding::Utf16Be => "UTF-16 BE",
                    }
                    .into(),
                ));
                ui.label(small(if t.fmt.crlf { "CRLF" } else { "LF" }.into()));
                if t.encrypted() {
                    ui.label(RichText::new("encrypted").size(12.0).color(pal.accent2));
                }
                if t.saving {
                    ui.label(small("saving…".into()));
                }
                if matches!(t.body, Body::Large { .. }) {
                    ui.label(small("read-only (large file)".into()));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(small(format!(
                    "{}%",
                    (self.settings.text_size / TEXT_SIZE_DEFAULT * 100.0).round()
                )))
                .on_hover_text("Text size · Ctrl + / Ctrl − / Ctrl 0");
                if let Some(at) = self.clipboard_clear_at {
                    let left = at.saturating_duration_since(Instant::now()).as_secs() + 1;
                    ui.label(
                        RichText::new(format!("clipboard clears in {left}s"))
                            .size(12.0)
                            .color(pal.accent),
                    );
                }
                if let Some((msg, when, err)) = &self.status
                    && when.elapsed() < Duration::from_secs(6)
                {
                    let color = if *err {
                        ui.visuals().error_fg_color
                    } else {
                        pal.text
                    };
                    ui.label(RichText::new(msg).size(12.0).color(color));
                }
            });
        });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = Palette::of(ui);
        let width = 360.0_f32.min(ui.available_width() - 32.0);
        let mut open = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.16).max(24.0));
                ui.add(egui::Image::new((self.logo.id(), vec2(88.0, 88.0))));
                ui.add_space(10.0);
                ui.label(RichText::new("Padd").size(34.0).color(pal.heading));
                ui.label(RichText::new("Markdown, structured text and secrets").color(pal.weak));
                ui.add_space(26.0);
                ui.allocate_ui(vec2(width, 0.0), |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if action_row(ui, "Open file…", &shortcut("Ctrl+O"), width).clicked() {
                            self.open_dialog(&ctx);
                        }
                        if action_row(ui, "New Markdown file", &shortcut("Ctrl+N"), width).clicked() {
                            self.new_tab(false);
                        }
                        if action_row(ui, "New secrets file", "encrypted", width).clicked() {
                            self.new_tab(true);
                        }
                        if !self.settings.recent.is_empty() {
                            ui.add_space(18.0);
                            ui.label(RichText::new("   RECENT").size(11.0).color(pal.weak));
                            ui.add_space(4.0);
                            for p in self.settings.recent.iter().take(6) {
                                let name = super::file_name(p);
                                let dir = p
                                    .parent()
                                    .map(|d| {
                                        let s = d.display().to_string();
                                        let home = std::env::var("HOME").unwrap_or_default();
                                        if !home.is_empty() && s.starts_with(&home) {
                                            format!("~{}", &s[home.len()..])
                                        } else {
                                            s
                                        }
                                    })
                                    .unwrap_or_default();
                                let dir: String = if dir.chars().count() > 30 {
                                    format!(
                                        "…{}",
                                        dir.chars()
                                            .rev()
                                            .take(29)
                                            .collect::<Vec<_>>()
                                            .into_iter()
                                            .rev()
                                            .collect::<String>()
                                    )
                                } else {
                                    dir
                                };
                                if action_row(ui, &name, &dir, width).clicked() {
                                    open = Some(p.clone());
                                }
                            }
                        }
                    });
                });
                ui.add_space(22.0);
                ui.label(
                    RichText::new("Drop files anywhere to open them")
                        .size(12.0)
                        .color(pal.weak),
                );
            });
        });
        if let Some(p) = open {
            self.open_path(&ctx, p);
        }
    }

    fn locked_view(&mut self, ui: &mut egui::Ui, idx: usize) {
        let ctx = ui.ctx().clone();
        let pal = Palette::of(ui);
        let focus = std::mem::take(&mut self.focus_unlock);
        let mut submit = false;
        let name = self.tabs[idx].name();
        let Body::Locked { input, error, busy } = &mut self.tabs[idx].body else {
            return;
        };
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.22).max(24.0));
            let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
            ui.painter()
                .circle_filled(r.center(), 32.0, pal.accent2.gamma_multiply(0.16));
            paint_big_lock(ui.painter(), r.center(), pal.accent2);
            ui.add_space(14.0);
            ui.label(RichText::new(&name).size(20.0).color(pal.heading));
            ui.label(RichText::new("This file is encrypted. Enter its password to open it.").color(pal.weak));
            ui.add_space(16.0);
            let r = ui.add_enabled(
                !*busy,
                egui::TextEdit::singleline(input)
                    .password(true)
                    .hint_text("Password")
                    .desired_width(280.0)
                    .margin(vec2(10.0, 7.0)),
            );
            if focus {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
            ui.add_space(10.0);
            if *busy {
                ui.horizontal(|ui| {
                    ui.add_space(ui.available_width() / 2.0 - 50.0);
                    ui.spinner();
                    ui.label(RichText::new("Decrypting…").color(pal.weak));
                });
            } else if primary_button(ui, "Unlock").clicked() {
                submit = true;
            }
            if let Some(e) = error {
                ui.add_space(6.0);
                ui.colored_label(ui.visuals().error_fg_color, e.as_str());
            }
        });
        if submit {
            self.unlock(&ctx, idx);
        }
    }
}

/// A large padlock for the lock screen.
fn paint_big_lock(p: &egui::Painter, c: Pos2, color: Color32) {
    let body = Rect::from_center_size(c + vec2(0.0, 5.0), vec2(24.0, 18.0));
    p.rect_filled(body, 4.0, color);
    let r = 7.5;
    let top = body.top();
    let mut pts = vec![pos2(c.x - r, top)];
    for k in 0..=16 {
        let a = std::f32::consts::PI * (1.0 - k as f32 / 16.0);
        pts.push(pos2(c.x + r * a.cos(), top - 6.0 - r * a.sin()));
    }
    pts.push(pos2(c.x + r, top));
    p.add(egui::Shape::line(pts, Stroke::new(3.0, color)));
    p.circle_filled(c + vec2(0.0, 4.0), 2.5, Color32::from_black_alpha(140));
}
