//! The editor surface, the find bar and the large-file viewer.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, CornerRadius, FontId, Galley, Key, Margin, Rect, RichText, Shape, Stroke,
    StrokeKind, TextBuffer, pos2, vec2,
};
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};

use super::chrome::{Icon, icon_button, paint_icon};
use super::{Body, Kind, LayoutCache, MAX_RICH, PaddApp};
use crate::highlight;
use crate::markdown::{self, SECRET_BUTTONS_W};
use crate::theme::Palette;

/// Max matches highlighted at once (keeps huge files snappy).
const MAX_MATCHES: usize = 5_000;

/// Case-insensitive search; returns char ranges.
fn find_all(text: &str, query: &str) -> Vec<(usize, usize)> {
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let q: Vec<char> = query.chars().map(fold).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let t: Vec<char> = text.chars().map(fold).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + q.len() <= t.len() && out.len() < MAX_MATCHES {
        if t[i..i + q.len()] == q[..] {
            out.push((i, i + q.len()));
            i += q.len();
        } else {
            i += 1;
        }
    }
    out
}

impl PaddApp {
    pub(super) fn find_bar(&mut self, ui: &mut egui::Ui, idx: usize) {
        let pal = Palette::of(ui);
        let ctx = ui.ctx().clone();
        let (tab_id, generation) = (self.tabs[idx].id, self.tabs[idx].edit_gen);
        let Some(find) = &mut self.find else { return };
        let mut close = false;
        egui::Frame::NONE
            .fill(pal.panel)
            .inner_margin(Margin::symmetric(12, 6))
            .stroke(egui::Stroke::new(1.0, pal.border))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 24.0), egui::Sense::hover());
                    paint_icon(ui.painter(), r, Icon::Search, pal.weak);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut find.query)
                            .hint_text("Find in document")
                            .desired_width(260.0)
                            .margin(vec2(8.0, 4.0)),
                    );
                    if std::mem::take(&mut find.focus) {
                        resp.request_focus();
                    }
                    let key = (tab_id, generation, find.query.clone());
                    if find.key != key {
                        let text = &self.tabs[idx].text;
                        find.matches = find_all(text, &find.query);
                        find.current = find.current.min(find.matches.len().saturating_sub(1));
                        if find.key.2 != find.query {
                            find.current = 0;
                            find.jump = !find.matches.is_empty();
                        }
                        find.key = key;
                    }
                    let n = find.matches.len();
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let shift = ui.input(|i| i.modifiers.shift);
                    let mut step = 0isize;
                    if enter {
                        step = if shift { -1 } else { 1 };
                        resp.request_focus();
                    }
                    let label = if find.query.is_empty() {
                        String::new()
                    } else if n == 0 {
                        "No results".into()
                    } else {
                        format!(
                            "{} of {}{}",
                            find.current + 1,
                            n,
                            if n >= MAX_MATCHES { "+" } else { "" }
                        )
                    };
                    ui.label(RichText::new(label).size(12.5).color(pal.weak));
                    let b = |ui: &mut egui::Ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(24.0, 24.0), egui::Sense::hover());
                        r
                    };
                    let r = b(ui);
                    if icon_button(
                        ui,
                        r,
                        ui.scope_id().with("find-prev"),
                        Icon::Up,
                        "Previous  Shift+Enter",
                    ) {
                        step = -1;
                    }
                    let r = b(ui);
                    if icon_button(ui, r, ui.scope_id().with("find-next"), Icon::Down, "Next  Enter") {
                        step = 1;
                    }
                    if step != 0 && n > 0 {
                        find.current = ((find.current as isize + step).rem_euclid(n as isize)) as usize;
                        find.jump = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let r = b(ui);
                        if icon_button(ui, r, ui.scope_id().with("find-close"), Icon::Close, "Close  Esc") {
                            close = true;
                        }
                    });
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                });
            });
        if close {
            self.find = None;
            ctx.memory_mut(|m| m.request_focus(self.tabs[idx].editor_id()));
        }
    }

    pub(super) fn large_view(&mut self, ui: &mut egui::Ui, idx: usize) {
        let pal = Palette::of(ui);
        let tab = &self.tabs[idx];
        let Body::Large { lines } = &tab.body else { return };
        let font = FontId::monospace(self.settings.text_size * 0.9);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        egui::ScrollArea::both()
            .auto_shrink(false)
            .show_rows(ui, row_h, lines.len(), |ui, rows| {
                ui.add_space(4.0);
                for r in rows {
                    let start = lines[r];
                    let end = lines.get(r + 1).map_or(tab.text.len(), |e| e - 1);
                    let line = &tab.text[start..end.max(start)];
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.add_sized(
                            vec2(56.0, row_h),
                            egui::Label::new(
                                RichText::new(format!("{}", r + 1))
                                    .font(font.clone())
                                    .color(pal.weak),
                            ),
                        );
                        ui.add(
                            egui::Label::new(RichText::new(line).font(font.clone()).color(pal.text))
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
                }
            });
    }

    pub(super) fn editor(&mut self, ui: &mut egui::Ui, idx: usize) {
        let ctx = ui.ctx().clone();
        let pal = Palette::of(ui);
        let live = self.settings.live_preview;
        let wrap = self.settings.wrap;
        let size = self.settings.text_size;
        let readable = self.settings.readable_width;

        // Find: matches to highlight, and an optional jump to the current one.
        let mut marks: Vec<(usize, usize)> = Vec::new();
        let mut current = None;
        let mut jump = None;
        if let Some(f) = &mut self.find
            && f.key.0 == self.tabs[idx].id
        {
            marks = f.matches.clone();
            current = f.matches.get(f.current).copied();
            if std::mem::take(&mut f.jump) {
                jump = current;
            }
        }

        let tab = &mut self.tabs[idx];
        let id = tab.editor_id();
        let rich = tab.text.len() <= MAX_RICH;
        if let Some((a, b)) = jump {
            let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
            state.store(&ctx, id);
        }

        // Lines under the cursor show raw Markdown; everything else renders.
        let has_focus = ctx.memory(|m| m.has_focus(id));
        let sel = egui::TextEdit::load_state(&ctx, id)
            .and_then(|s| s.cursor.char_range())
            .map(|r| (r.primary.index.0, r.secondary.index.0));
        let active = match (has_focus, sel) {
            (true, Some((a, b))) if live && tab.kind == Kind::Markdown => {
                Some(markdown::active_lines(&tab.text, a, b))
            }
            _ => None,
        };

        let syntax = match tab.kind {
            Kind::Code if rich => {
                let name = tab.name();
                let first = tab.text.lines().next().unwrap_or("");
                highlight::syntax_for(&name, first)
            }
            _ => None,
        };
        let assets_ready = highlight::assets().is_some();
        let md = live && rich && tab.kind == Kind::Markdown;
        let mono_size = (size * 0.9).round();

        // Comfortable reading column for Markdown: centred, about 48 em wide.
        let avail = ui.available_size();
        let extra_side = if md && readable {
            ((avail.x - size * 48.0) / 2.0 - 32.0).max(0.0)
        } else {
            0.0
        };
        let margin = if md {
            Margin {
                left: 32,
                right: 32,
                top: 26,
                bottom: 40,
            }
        } else {
            Margin {
                left: 18,
                right: 14,
                top: 12,
                bottom: 30,
            }
        };

        let super::Tab {
            text,
            cache,
            hl,
            revealed,
            kind,
            ..
        } = tab;
        let kind = *kind;
        let mut copy: Option<String> = None;
        let mut toggle: Option<usize> = None;
        let scroll = if wrap {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both()
        };
        let edit = scroll.auto_shrink(false).id_salt(id).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.add_space(extra_side);
                ui.vertical(|ui| {
                    ui.set_width(avail.x - 2.0 * extra_side);
                    let out = {
                        let mut layouter =
                            |ui: &egui::Ui, buf: &dyn TextBuffer, wrap_width: f32| -> Arc<Galley> {
                                let text = buf.as_str();
                                let key = {
                                    let mut h = std::collections::hash_map::DefaultHasher::new();
                                    (
                                        text,
                                        &active,
                                        wrap_width.to_bits(),
                                        pal.dark,
                                        md,
                                        assets_ready,
                                        kind as u8,
                                        size.to_bits(),
                                    )
                                        .hash(&mut h);
                                    let mut r: Vec<_> = revealed.iter().collect();
                                    r.sort();
                                    r.hash(&mut h);
                                    h.finish()
                                };
                                if let Some(c) = cache.as_ref().filter(|c| c.key == key) {
                                    return c.galley.clone();
                                }
                                let (mut job, secrets, grids) = if md {
                                    let max_w = if wrap { wrap_width } else { f32::INFINITY };
                                    let s = ui.fonts_mut(|f| {
                                        let mut measure = |s: &str, fmt: &TextFormat| {
                                            let job = LayoutJob::single_section(s.to_owned(), fmt.clone());
                                            f.layout_job(job).size().x
                                        };
                                        markdown::layout(
                                            text,
                                            active.clone(),
                                            revealed,
                                            &pal,
                                            size,
                                            max_w,
                                            &mut measure,
                                        )
                                    });
                                    (s.job, s.secrets, s.grids)
                                } else if let Some(syn) = syntax {
                                    (
                                        hl.job(text, syn, pal.dark, FontId::monospace(mono_size)),
                                        Vec::new(),
                                        Vec::new(),
                                    )
                                } else {
                                    let font = FontId::monospace(mono_size);
                                    (
                                        LayoutJob::simple(text.to_owned(), font, pal.text, f32::INFINITY),
                                        Vec::new(),
                                        Vec::new(),
                                    )
                                };
                                job.wrap.max_width = if wrap { wrap_width } else { f32::INFINITY };
                                let galley = ui.fonts_mut(|f| f.layout_job(job));
                                *cache = Some(LayoutCache {
                                    key,
                                    galley: galley.clone(),
                                    secrets,
                                    grids,
                                });
                                galley
                            };
                        // Placeholder so table and card backgrounds paint behind the text.
                        let backdrop = ui.painter().add(Shape::Noop);
                        let out = egui::TextEdit::multiline(&mut *text)
                            .id(id)
                            .frame(egui::Frame::NONE.inner_margin(margin))
                            .desired_width(if wrap { f32::INFINITY } else { 0.0 })
                            .min_size(vec2(avail.x - 2.0 * extra_side, avail.y))
                            .lock_focus(true)
                            .layouter(&mut layouter)
                            .show(ui);
                        (out, backdrop)
                    };
                    let (out, backdrop) = out;
                    let changed = out.response.changed();
                    if changed {
                        // Line numbers shift on edits: hide again rather than reveal the wrong line.
                        revealed.clear();
                    }
                    let origin = out.galley_pos.to_vec2();
                    let at = |c: usize| out.galley.pos_from_cursor(CCursor::new(c)).translate(origin);

                    // Find highlights (only those in view).
                    let clip = ui.clip_rect();
                    for &(a, b) in &marks {
                        let (ra, rb) = (at(a), at(b));
                        if ra.bottom() < clip.top() || ra.top() > clip.bottom() {
                            continue;
                        }
                        let r = if (ra.top() - rb.top()).abs() < 1.0 {
                            Rect::from_min_max(ra.left_top(), pos2(rb.left(), ra.bottom()))
                        } else {
                            Rect::from_min_max(ra.left_top(), pos2(ra.left() + 8.0, ra.bottom()))
                        };
                        let strong = Some((a, b)) == current;
                        ui.painter().rect_filled(
                            r.expand(1.0),
                            3.0,
                            pal.accent.gamma_multiply(if strong { 0.45 } else { 0.18 }),
                        );
                    }
                    if let Some((a, _)) = jump {
                        ui.scroll_to_rect(at(a).expand(60.0), Some(Align::Center));
                    }

                    // Tables and secrets cards: fills and lines behind the text.
                    if let Some(c) = cache.as_ref() {
                        let shapes = grid_shapes(&c.grids, &at, origin.x, clip, &pal);
                        ui.painter().set(backdrop, Shape::Vec(shapes));
                    }

                    // Secrets: [show] [copy] and the mask, drawn in the reserved space.
                    if let Some(c) = cache.as_ref() {
                        for sec in c.secrets.iter().filter(|s| !s.active) {
                            let r = at(sec.value_char);
                            let x0 = r.left() - markdown::secret_gap(size, sec.revealed);
                            let h = r.height().clamp(18.0, 26.0);
                            let btn = |dx: f32| {
                                Rect::from_min_size(pos2(x0 + dx, r.center().y - h / 2.0), vec2(20.0, h))
                            };
                            let (eye, tip) = if sec.revealed {
                                (Icon::EyeOff, "Hide")
                            } else {
                                (Icon::Eye, "Show")
                            };
                            let base = id.with(sec.line);
                            if icon_button(ui, btn(2.0), base.with("eye"), eye, tip) {
                                toggle = Some(sec.line);
                            }
                            if icon_button(
                                ui,
                                btn(23.0),
                                base.with("copy"),
                                Icon::Copy,
                                "Copy · clipboard clears in 30 s",
                            ) && let Some(v) = text.get(sec.value.clone())
                            {
                                copy = Some(v.to_owned());
                            }
                            if !sec.revealed {
                                ui.painter().text(
                                    pos2(x0 + SECRET_BUTTONS_W, r.center().y),
                                    Align2::LEFT_CENTER,
                                    "••••••••",
                                    FontId::monospace(size * 0.85),
                                    pal.accent2,
                                );
                            }
                        }
                    }
                    let cursor = out.cursor_range.map(|range| {
                        let ci = range.primary.index.0;
                        let end = text.char_indices().nth(ci).map_or(text.len(), |(b, _)| b);
                        let before = &text[..end];
                        let line = before.matches('\n').count() + 1;
                        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                        (line, col)
                    });
                    (changed, cursor)
                })
                .inner
            })
            .inner
        });
        let (changed, cursor) = edit.inner;
        let tab = &mut self.tabs[idx];
        if let Some(c) = cursor {
            tab.cursor = c;
        }
        if changed {
            tab.dirty = true;
            tab.edit_gen += 1;
        }
        if let Some(line) = toggle
            && !tab.revealed.remove(&line)
        {
            tab.revealed.insert(line);
        }
        if let Some(v) = copy {
            self.copy_secret(&ctx, v);
            self.info("Copied to clipboard");
        }
    }
}

/// Backgrounds and lines of the rendered tables and secrets cards that are in view.
fn grid_shapes(
    grids: &[markdown::Grid],
    at: &dyn Fn(usize) -> Rect,
    x0: f32,
    clip: Rect,
    pal: &Palette,
) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let line = Stroke::new(1.0, pal.border);
    let radius = 6u8;
    for g in grids {
        let (Some(&(first, _)), Some(&(_, last))) = (g.rows.first(), g.rows.last()) else {
            continue;
        };
        let (top, bottom) = (at(first).top(), at(last).bottom());
        if bottom < clip.top() || top > clip.bottom() {
            continue;
        }
        let right = x0 + g.cols.last().copied().unwrap_or(0.0);
        let outer = Rect::from_min_max(pos2(x0, top), pos2(right, bottom));
        if g.header {
            let hb = at(g.rows[0].1).bottom();
            let r = CornerRadius {
                nw: radius,
                ne: radius,
                sw: 0,
                se: 0,
            };
            shapes.push(Shape::rect_filled(
                Rect::from_min_max(outer.min, pos2(right, hb)),
                r,
                pal.hover,
            ));
        }
        if g.secrets && g.cols.len() > 1 {
            let r = CornerRadius {
                nw: radius,
                sw: radius,
                ne: 0,
                se: 0,
            };
            let col = Rect::from_min_max(outer.min, pos2(x0 + g.cols[1], bottom));
            shapes.push(Shape::rect_filled(col, r, pal.hover));
        }
        for &(_, end) in &g.rows[..g.rows.len() - 1] {
            let y = at(end).bottom().round() + 0.5;
            shapes.push(Shape::line_segment([pos2(x0, y), pos2(right, y)], line));
        }
        for &cx in &g.cols[1..g.cols.len() - 1] {
            let x = (x0 + cx).round() + 0.5;
            shapes.push(Shape::line_segment([pos2(x, top), pos2(x, bottom)], line));
        }
        shapes.push(Shape::rect_stroke(outer, radius, line, StrokeKind::Inside));
    }
    shapes
}

#[cfg(test)]
mod tests {
    use super::find_all;

    #[test]
    fn finds_case_insensitive_char_ranges() {
        assert_eq!(find_all("Ab ab AB", "ab"), vec![(0, 2), (3, 5), (6, 8)]);
        assert_eq!(find_all("é API é", "api"), vec![(2, 5)]);
        assert!(find_all("abc", "").is_empty());
    }
}
