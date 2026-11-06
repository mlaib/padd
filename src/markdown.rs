//! Markdown live preview: turns Markdown source into a styled `LayoutJob`
//! for the editor. Syntax markers are hidden (transparent, ~0 width) except
//! on the line(s) holding the cursor, so the text and the char indices never
//! change and editing stays exact.
//!
//! Tables and runs of `label::: value` lines render as grids: their syntax is
//! hidden, and each cell starts at its column through the section's
//! `leading_space`. The editor paints the grid lines behind the text.

use std::collections::HashSet;
use std::ops::Range;

use eframe::egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use eframe::egui::{Color32, FontFamily, FontId, Stroke};
use pulldown_cmark::{Alignment, Event, Options, Parser, Tag};

use crate::secrets;
use crate::theme::Palette;

/// Width reserved in front of a secret value for the [show] and [copy] buttons.
pub const SECRET_BUTTONS_W: f32 = 48.0;

/// Space reserved before a secret value: buttons, plus the "••••••••" mask when hidden.
pub fn secret_gap(size: f32, revealed: bool) -> f32 {
    SECRET_BUTTONS_W + if revealed { 0.0 } else { size * 5.2 }
}

type Flags = u32;
const EMPH: Flags = 1;
const STRONG: Flags = 1 << 1;
const STRIKE: Flags = 1 << 2;
const CODE: Flags = 1 << 3;
const LINK: Flags = 1 << 4;
const QUOTE: Flags = 1 << 5;
const MARK: Flags = 1 << 6; // syntax marker, hidden off the active line
const DIM: Flags = 1 << 7; // structural syntax, always visible but faint
const CODEBLOCK: Flags = 1 << 8;
const TABLE: Flags = 1 << 9;
const BULLET: Flags = 1 << 10;
const SEC_LABEL: Flags = 1 << 11;
const SEC_SEP: Flags = 1 << 12; // the ':::'
const SEC_VALUE: Flags = 1 << 14;
const TPIPE: Flags = 1 << 15; // table syntax: pipes, padding, the |---| row
const THEAD: Flags = 1 << 16;
const SROW: Flags = 1 << 17; // a whole `label::: value` line
const SEC_HIDE: Flags = 1 << 18; // syntax of a secrets line (prefix, ':::', spaces)

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct St {
    flags: Flags,
    heading: u8,
}

/// A secret value found in the document, for drawing the overlay buttons.
#[derive(Clone, Debug)]
pub struct Secret {
    pub line: usize,
    pub value: Range<usize>,
    /// Char index of the value start (for `Galley::pos_from_cursor`).
    pub value_char: usize,
    /// The cursor is on this line: shown as raw source, no overlay.
    pub active: bool,
    pub revealed: bool,
}

/// A rendered table or secrets card; the editor paints its lines and fills.
#[derive(Clone, Debug)]
pub struct Grid {
    /// Per row: char index of the line start and of the line end.
    pub rows: Vec<(usize, usize)>,
    /// Column boundaries in points from the text's left edge: 0, …, total width.
    pub cols: Vec<f32>,
    /// The first row is a header (tables).
    pub header: bool,
    /// A secrets card (the first column holds the labels).
    pub secrets: bool,
}

pub struct Styled {
    pub job: LayoutJob,
    pub secrets: Vec<Secret>,
    pub grids: Vec<Grid>,
}

struct TableInfo {
    range: Range<usize>,
    aligns: Vec<Alignment>,
    rows: Vec<RowInfo>,
}

struct RowInfo {
    /// The line, without its newline.
    line: Range<usize>,
    /// Trimmed cell contents (possibly empty), in column order.
    cells: Vec<Range<usize>>,
}

struct SecretLine {
    line_no: usize,
    line: Range<usize>,
    label: Range<usize>,
    value: Range<usize>,
}

/// Measures the width of a run of text in a given format (the editor passes the font system).
pub type Measure<'a> = &'a mut dyn FnMut(&str, &TextFormat) -> f32;

/// Lay out `text` as live-preview Markdown.
///
/// `active` is the byte range of the line(s) under the cursor/selection.
/// `revealed` holds the line numbers of secrets the user chose to show.
/// `max_width` is the wrap width: a table wider than that is shown as source.
pub fn layout(
    text: &str,
    active: Option<Range<usize>>,
    revealed: &HashSet<usize>,
    pal: &Palette,
    size: f32,
    max_width: f32,
    measure: Measure,
) -> Styled {
    let mut st = vec![St::default(); text.len()];
    let tables = style_markdown(text, &mut st);
    let lines = style_secrets(text, &mut st);

    // Sections must start where a cell, label or value starts, to carry its indent.
    let mut breaks: Vec<usize> = tables
        .iter()
        .flat_map(|t| t.rows.iter().flat_map(|r| r.cells.iter()))
        .filter(|c| !c.is_empty())
        .map(|c| c.start)
        .chain(lines.iter().flat_map(|l| [l.label.start, l.value.start]))
        .collect();
    breaks.sort_unstable();
    breaks.dedup();

    // A table under the cursor shows as source as a whole.
    let mut act: Vec<Range<usize>> = active.into_iter().collect();
    if let Some(a) = act.first().cloned() {
        for t in &tables {
            if a.start < t.range.end && t.range.start <= a.end {
                act.push(t.range.clone());
            }
        }
    }

    loop {
        let is_active = |i: usize| act.iter().any(|r| r.contains(&i) || r.end == i);
        let mut secrets: Vec<Secret> = lines
            .iter()
            .filter(|l| !l.value.is_empty())
            .map(|l| Secret {
                line: l.line_no,
                value: l.value.clone(),
                value_char: 0,
                active: is_active(l.line.start),
                revealed: revealed.contains(&l.line_no),
            })
            .collect();
        let (mut job, chars) = build(text, &st, &breaks, &is_active, &secrets, pal, size);
        let mut m = Widths {
            text,
            cache: vec![None; job.sections.len()],
            f: &mut *measure,
        };
        let pad = (size * 0.75).round();
        let mut grids = Vec::new();

        // Tables.
        let mut too_wide = None;
        for t in tables.iter().filter(|t| !is_active(t.range.start)) {
            let ncols = t.aligns.len();
            let w: Vec<Vec<f32>> = t
                .rows
                .iter()
                .map(|r| r.cells.iter().map(|c| m.span(&job, c.clone())).collect())
                .collect();
            let mut bounds = vec![0.0f32];
            for c in 0..ncols {
                let colw = w.iter().filter_map(|r| r.get(c)).fold(size, |a, &b| a.max(b));
                bounds.push(bounds[c] + colw + 2.0 * pad);
            }
            if bounds[ncols] > max_width {
                too_wide = Some(t.range.clone());
                break;
            }
            for (r, row) in t.rows.iter().enumerate() {
                let targets: Vec<(usize, f32)> = row
                    .cells
                    .iter()
                    .enumerate()
                    .filter(|(c, cell)| !cell.is_empty() && *c < ncols)
                    .map(|(c, cell)| {
                        let free = bounds[c + 1] - bounds[c] - 2.0 * pad - w[r][c];
                        let off = match t.aligns[c] {
                            Alignment::Right => free,
                            Alignment::Center => free / 2.0,
                            _ => 0.0,
                        };
                        (cell.start, bounds[c] + pad + off)
                    })
                    .collect();
                m.place(&mut job, row.line.clone(), &targets);
            }
            grids.push(Grid {
                rows: t
                    .rows
                    .iter()
                    .map(|r| {
                        (
                            char_at(&job, &chars, text, r.line.start),
                            char_at(&job, &chars, text, r.line.end),
                        )
                    })
                    .collect(),
                cols: bounds,
                header: true,
                secrets: false,
            });
        }
        if let Some(r) = too_wide {
            act.push(r);
            continue;
        }

        // Secrets: each run of consecutive `label::: value` lines is one card.
        let mut i = 0;
        while i < lines.len() {
            let mut j = i + 1;
            while j < lines.len() && lines[j].line_no == lines[j - 1].line_no + 1 {
                j += 1;
            }
            let group = &lines[i..j];
            i = j;
            let rev = |l: &SecretLine| revealed.contains(&l.line_no);
            let label_w = group
                .iter()
                .map(|l| m.span(&job, l.label.clone()))
                .fold(size * 4.0, f32::max);
            let col1 = label_w + 2.0 * pad;
            let value_w = group
                .iter()
                .map(|l| {
                    let shown = if rev(l) {
                        m.span(&job, l.value.clone())
                    } else {
                        0.0
                    };
                    secret_gap(size, rev(l)) + shown
                })
                .fold(size * 12.0, f32::max);
            let total = (col1 + value_w + 2.0 * pad).min(max_width.max(col1 + size * 8.0));
            let mut run: Vec<(usize, usize)> = Vec::new();
            for (n, l) in group.iter().enumerate() {
                let inactive = !is_active(l.line.start);
                if inactive {
                    let mut targets = vec![(l.label.start, pad)];
                    if !l.value.is_empty() {
                        targets.push((l.value.start, col1 + pad + secret_gap(size, rev(l))));
                    }
                    m.place(&mut job, l.line.clone(), &targets);
                    run.push((
                        char_at(&job, &chars, text, l.line.start),
                        char_at(&job, &chars, text, l.line.end),
                    ));
                }
                if (!inactive || n + 1 == group.len()) && !run.is_empty() {
                    grids.push(Grid {
                        rows: std::mem::take(&mut run),
                        cols: vec![0.0, col1, total],
                        header: false,
                        secrets: true,
                    });
                }
            }
        }

        for s in &mut secrets {
            s.value_char = char_at(&job, &chars, text, s.value.start);
        }
        return Styled { job, secrets, grids };
    }
}

/// Section widths, measured on demand.
struct Widths<'a, 'm> {
    text: &'a str,
    cache: Vec<Option<f32>>,
    f: &'m mut dyn FnMut(&str, &TextFormat) -> f32,
}

impl Widths<'_, '_> {
    fn width(&mut self, job: &LayoutJob, idx: usize) -> f32 {
        if let Some(w) = self.cache[idx] {
            return w;
        }
        let s = &job.sections[idx];
        let w = (self.f)(&self.text[s.byte_range.start.0..s.byte_range.end.0], &s.format);
        self.cache[idx] = Some(w);
        w
    }

    /// Width of the sections inside `r` (which starts on a section boundary).
    fn span(&mut self, job: &LayoutJob, r: Range<usize>) -> f32 {
        let mut idx = job.sections.partition_point(|s| s.byte_range.start.0 < r.start);
        let mut w = 0.0;
        while idx < job.sections.len() && job.sections[idx].byte_range.start.0 < r.end {
            w += self.width(job, idx);
            idx += 1;
        }
        w
    }

    /// Indent the sections of one line so each target byte starts at its x.
    fn place(&mut self, job: &mut LayoutJob, line: Range<usize>, targets: &[(usize, f32)]) {
        let mut idx = job
            .sections
            .partition_point(|s| s.byte_range.start.0 < line.start);
        let mut x = 0.0;
        while idx < job.sections.len() && job.sections[idx].byte_range.start.0 < line.end {
            let start = job.sections[idx].byte_range.start.0;
            if let Some(&(_, tx)) = targets.iter().find(|(p, _)| *p == start) {
                let lead = (tx - x).max(0.0);
                job.sections[idx].leading_space = lead;
                x += lead;
            }
            x += self.width(job, idx);
            idx += 1;
        }
    }
}

/// Char index of byte `b`, from the char index of each section start.
fn char_at(job: &LayoutJob, chars: &[usize], text: &str, b: usize) -> usize {
    let idx = job
        .sections
        .partition_point(|s| s.byte_range.start.0 <= b)
        .saturating_sub(1);
    match job.sections.get(idx) {
        Some(s) => {
            chars[idx]
                + text[s.byte_range.start.0..b.max(s.byte_range.start.0)]
                    .chars()
                    .count()
        }
        None => 0,
    }
}

/// Split the text into styled sections. Returns the job and each section's start char index.
fn build(
    text: &str,
    st: &[St],
    breaks: &[usize],
    is_active: &dyn Fn(usize) -> bool,
    secrets: &[Secret],
    pal: &Palette,
    size: f32,
) -> (LayoutJob, Vec<usize>) {
    let mut job = LayoutJob {
        text: text.to_owned(),
        ..Default::default()
    };
    let mut chars_at = Vec::new();
    let n = text.len();
    let (mut i, mut chars, mut b, mut k) = (0usize, 0usize, 0usize, 0usize);
    while i < n {
        let s = st[i];
        let act = is_active(i);
        while b < breaks.len() && breaks[b] <= i {
            b += 1;
        }
        let next_break = breaks.get(b).copied().unwrap_or(n);
        let mut j = i + 1;
        while j < n && j < next_break && st[j] == s && is_active(j) == act {
            j += 1;
        }
        while j < n && !text.is_char_boundary(j) {
            j += 1;
        }
        while k < secrets.len() && secrets[k].value.end <= i {
            k += 1;
        }
        let hidden_value = s.flags & SEC_VALUE != 0
            && secrets
                .get(k)
                .is_some_and(|sec| !sec.active && !sec.revealed && sec.value.contains(&i));
        job.sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: ByteIndex(i)..ByteIndex(j),
            format: format_for(s, act, hidden_value, pal, size),
        });
        chars_at.push(chars);
        chars += text[i..j].chars().count();
        i = j;
    }
    (job, chars_at)
}

fn hidden() -> TextFormat {
    TextFormat {
        font_id: FontId::proportional(0.5),
        color: Color32::TRANSPARENT,
        ..Default::default()
    }
}

fn format_for(s: St, act: bool, hidden_value: bool, pal: &Palette, size: f32) -> TextFormat {
    let f = s.flags;
    if f & (MARK | TPIPE | SEC_HIDE) != 0 && !act {
        return hidden();
    }
    if f & SEC_VALUE != 0 && hidden_value {
        return hidden();
    }
    let grid = !act && f & (TABLE | SROW) != 0;
    let mut fmt = TextFormat {
        font_id: FontId::new(size, FontFamily::Proportional),
        color: pal.text,
        ..Default::default()
    };
    if s.heading > 0 {
        let scale = [1.9, 1.55, 1.3, 1.15, 1.05, 1.0][(s.heading as usize - 1).min(5)];
        fmt.font_id.size = size * scale;
        fmt.color = pal.heading;
    }
    if f & (STRONG | THEAD) != 0 && !(f & THEAD != 0 && act) {
        fmt.color = pal.strong;
        fmt.extra_letter_spacing = 0.3;
    }
    if f & EMPH != 0 {
        fmt.italics = true;
    }
    if f & STRIKE != 0 {
        fmt.strikethrough = Stroke::new(1.0, pal.weak);
        fmt.color = pal.weak;
    }
    if f & (CODE | CODEBLOCK | SEC_VALUE) != 0 || (f & TABLE != 0 && act) {
        fmt.font_id.family = FontFamily::Monospace;
        fmt.font_id.size *= 0.92;
    }
    if f & (CODE | CODEBLOCK) != 0 {
        fmt.background = pal.code_bg;
        fmt.color = pal.code;
    }
    if grid && f & CODE != 0 {
        // Keep the code background snug inside a tall table row.
        fmt.valign = eframe::egui::Align::Center;
    }
    if f & LINK != 0 {
        fmt.color = pal.link;
        fmt.underline = Stroke::new(1.0, pal.link.gamma_multiply(0.5));
    }
    if f & QUOTE != 0 {
        fmt.color = pal.weak;
        fmt.italics = true;
    }
    if f & BULLET != 0 {
        fmt.color = pal.accent;
    }
    if f & SEC_LABEL != 0 {
        fmt.color = pal.accent;
    }
    if f & (MARK | DIM | SEC_SEP | TPIPE) != 0 {
        fmt.color = pal.weak;
        fmt.background = Color32::TRANSPARENT;
    }
    // Comfortable reading rhythm: 1.5× line height for prose, a bit tighter for code,
    // and roomy rows in tables and secrets cards.
    fmt.line_height = Some(if grid && f & CODE == 0 {
        (size * 2.1).round()
    } else if f & (CODE | CODEBLOCK | TABLE) != 0 {
        (fmt.font_id.size * 1.35).round()
    } else {
        (fmt.font_id.size * 1.5).round()
    });
    fmt
}

fn set(st: &mut [St], r: Range<usize>, flag: Flags) {
    let end = r.end.min(st.len());
    for s in &mut st[r.start.min(end)..end] {
        s.flags |= flag;
    }
}

fn count_prefix(s: &str, c: u8) -> usize {
    s.bytes().take_while(|&b| b == c).count()
}

fn style_markdown(text: &str, st: &mut [St]) -> Vec<TableInfo> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut tables: Vec<TableInfo> = Vec::new();
    let mut in_head = false;
    for (ev, r) in Parser::new_ext(text, opts).into_offset_iter() {
        let src = &text[r.clone()];
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                let lvl = level as u8;
                for s in &mut st[r.clone()] {
                    s.heading = lvl;
                }
                let lead = src.len() - src.trim_start_matches(' ').len();
                let hashes = count_prefix(&src[lead..], b'#');
                if hashes > 0 {
                    let after = &src[lead + hashes..];
                    let sp = after.len() - after.trim_start_matches([' ', '\t']).len();
                    set(st, r.start..r.start + lead + hashes + sp, MARK);
                } else if let Some(nl) = src.trim_end().rfind('\n') {
                    // setext heading: the ===/--- underline
                    set(st, r.start + nl + 1..r.end, MARK);
                }
            }
            Event::Start(Tag::Emphasis) => {
                set(st, r.clone(), EMPH);
                set(st, r.start..r.start + 1, MARK);
                set(st, r.end - 1..r.end, MARK);
            }
            Event::Start(Tag::Strong) => {
                set(st, r.clone(), STRONG);
                set(st, r.start..r.start + 2, MARK);
                set(st, r.end.saturating_sub(2)..r.end, MARK);
            }
            Event::Start(Tag::Strikethrough) => {
                set(st, r.clone(), STRIKE);
                let n = count_prefix(src, b'~').clamp(1, 2);
                set(st, r.start..r.start + n, MARK);
                set(st, r.end.saturating_sub(n)..r.end, MARK);
            }
            Event::Code(_) => {
                set(st, r.clone(), CODE);
                let n = count_prefix(src, b'`');
                set(st, r.start..r.start + n, MARK);
                set(st, r.end.saturating_sub(n)..r.end, MARK);
            }
            Event::Start(Tag::Link { .. }) => {
                set(st, r.clone(), LINK);
                if src.starts_with('<') && src.ends_with('>') {
                    set(st, r.start..r.start + 1, MARK);
                    set(st, r.end - 1..r.end, MARK);
                } else if src.starts_with('[') {
                    set(st, r.start..r.start + 1, MARK);
                    if let Some(p) = src.rfind("](").or_else(|| src.rfind("][")) {
                        set(st, r.start + p..r.end, MARK);
                    } else if src.ends_with(']') {
                        set(st, r.end - 1..r.end, MARK);
                    }
                }
            }
            Event::Start(Tag::Image { .. }) | Event::Html(_) | Event::InlineHtml(_) | Event::Rule => {
                set(st, r.clone(), DIM);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                set(st, r.clone(), QUOTE);
                let mut off = r.start;
                for line in src.split_inclusive('\n') {
                    let t = line.trim_start();
                    if t.starts_with('>') {
                        let p = off + (line.len() - t.len());
                        set(st, p..p + 1, BULLET);
                    }
                    off += line.len();
                }
            }
            Event::Start(Tag::Item) => {
                let lead = src.len() - src.trim_start().len();
                let tok = src[lead..]
                    .bytes()
                    .take_while(|b| b.is_ascii_digit() || matches!(b, b'-' | b'*' | b'+' | b'.' | b')'))
                    .count();
                set(st, r.start + lead..r.start + lead + tok, BULLET);
            }
            Event::TaskListMarker(_) => set(st, r, BULLET),
            Event::Start(Tag::CodeBlock(kind)) => {
                set(st, r.clone(), CODEBLOCK);
                if matches!(kind, pulldown_cmark::CodeBlockKind::Fenced(_)) {
                    let first = src.find('\n').map_or(r.end, |p| r.start + p);
                    set(st, r.start..first, DIM);
                    let body = src.trim_end_matches('\n');
                    if let Some(p) = body.rfind('\n') {
                        let last = &body[p + 1..];
                        if last.trim_start().starts_with("```") || last.trim_start().starts_with("~~~") {
                            set(st, r.start + p + 1..r.start + body.len(), DIM);
                        }
                    }
                }
            }
            Event::Start(Tag::Table(aligns)) => {
                set(st, r.clone(), TABLE | TPIPE);
                tables.push(TableInfo {
                    range: r,
                    aligns,
                    rows: Vec::new(),
                });
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                in_head = matches!(ev, Event::Start(Tag::TableHead));
                if let Some(t) = tables.last_mut() {
                    let line = r.start..r.start + src.trim_end_matches(['\n', '\r']).len();
                    t.rows.push(RowInfo {
                        line,
                        cells: Vec::new(),
                    });
                }
            }
            Event::Start(Tag::TableCell) => {
                let lead = src.len() - src.trim_start().len();
                let cell = r.start + lead..r.start + src.trim_end().len().max(lead);
                for s in &mut st[cell.clone()] {
                    s.flags &= !TPIPE;
                    if in_head {
                        s.flags |= THEAD;
                    }
                }
                if let Some(row) = tables.last_mut().and_then(|t| t.rows.last_mut()) {
                    row.cells.push(cell);
                }
            }
            _ => {}
        }
    }
    tables
}

/// Mark `label::: value` lines (outside code blocks and tables).
fn style_secrets(text: &str, st: &mut [St]) -> Vec<SecretLine> {
    let mut out = Vec::new();
    let mut off = 0;
    for (line_no, line) in text.split_inclusive('\n').enumerate() {
        let start = off;
        off += line.len();
        if st.get(start).is_some_and(|s| s.flags & (CODEBLOCK | TABLE) != 0) {
            continue;
        }
        let Some(sp) = secrets::parse_line(line) else {
            continue;
        };
        let abs = |r: Range<usize>| start + r.start..start + r.end;
        let (label, sep, value) = (abs(sp.label), abs(sp.sep), abs(sp.value));
        let end = start + line.trim_end_matches(['\n', '\r']).len();
        for s in &mut st[start..off] {
            s.flags = (s.flags & !(MARK | DIM)) | SROW | SEC_HIDE;
        }
        for s in &mut st[label.clone()] {
            s.flags = (s.flags & !SEC_HIDE) | SEC_LABEL;
        }
        for s in &mut st[sep] {
            *s = St {
                flags: SEC_SEP | SEC_HIDE | SROW,
                heading: s.heading,
            };
        }
        for s in &mut st[value.clone()] {
            *s = St {
                flags: SEC_VALUE | SROW,
                heading: 0,
            };
        }
        out.push(SecretLine {
            line_no,
            line: start..end,
            label,
            value,
        });
    }
    out
}

/// Byte range covering every line touched by the char range `[a, b]`.
pub fn active_lines(text: &str, a: usize, b: usize) -> Range<usize> {
    let (a, b) = (a.min(b), a.max(b));
    let mut start = None;
    let mut end = text.len();
    for (ci, (bi, _)) in text.char_indices().enumerate() {
        if ci == a {
            start = Some(bi);
        }
        if ci == b {
            end = bi;
            break;
        }
    }
    let start = start.unwrap_or(text.len());
    let ls = text[..start].rfind('\n').map_or(0, |p| p + 1);
    let le = text[end..].find('\n').map_or(text.len(), |p| end + p);
    ls..le
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags_at(text: &str, needle: &str) -> Flags {
        let mut st = vec![St::default(); text.len()];
        style_markdown(text, &mut st);
        style_secrets(text, &mut st);
        st[text.find(needle).unwrap()].flags
    }

    #[test]
    fn markers_are_marked() {
        let t = "# Title\n\nsome **bold** and `code`\n";
        assert!(flags_at(t, "# ") & MARK != 0);
        assert!(flags_at(t, "**bold") & MARK != 0);
        assert!(flags_at(t, "bold**") & STRONG != 0);
        assert!(flags_at(t, "`code") & MARK != 0);
    }

    /// Fake font metrics: every char is half the font size wide.
    fn lay(t: &str, active: Option<Range<usize>>, max_width: f32) -> Styled {
        let mut m = |s: &str, f: &TextFormat| s.chars().count() as f32 * f.font_id.size * 0.5;
        layout(
            t,
            active,
            &HashSet::new(),
            &Palette::new(true),
            14.0,
            max_width,
            &mut m,
        )
    }

    fn section_at<'a>(out: &'a Styled, b: usize) -> &'a LayoutSection {
        out.job
            .sections
            .iter()
            .find(|x| x.byte_range.start.0 == b)
            .unwrap()
    }

    #[test]
    fn tables_render_as_grids() {
        let t = "text\n\n| a | bbbbbb |\n|---|--:|\n| cccccc | d |\n\nafter\n";
        let out = lay(t, None, 1000.0);
        assert_eq!(out.grids.len(), 1);
        let g = &out.grids[0];
        assert_eq!(g.rows.len(), 2);
        assert_eq!(g.cols.len(), 3);
        // pipes hidden, cells start at their column
        assert_eq!(
            section_at(&out, t.find("| a").unwrap()).format.color,
            Color32::TRANSPARENT
        );
        // x where the section at byte `b` starts, with the fake metrics of `lay`
        let x = |b: usize| {
            let line = t[..b].rfind('\n').map_or(0, |p| p + 1);
            let mut x = 0.0;
            for s in out.job.sections.iter().filter(|s| s.byte_range.start.0 >= line) {
                x += s.leading_space;
                if s.byte_range.start.0 == b {
                    return x;
                }
                let r = s.byte_range.start.0..s.byte_range.end.0;
                x += t[r].chars().count() as f32 * s.format.font_id.size * 0.5;
            }
            unreachable!()
        };
        let at = |n: &str| t.find(n).unwrap();
        assert!((x(at("a |")) - x(at("cccccc"))).abs() < 0.01);
        // right-aligned column: "bbbbbb" and "d" end at the same x
        let (b, d) = (x(at("bbbbbb")) + 6.0 * 7.0, x(at("d |")) + 7.0);
        assert!((b - d).abs() < 0.01, "{b} {d}");
        // cursor in the table: shown as source
        let row = t.find("| c").unwrap();
        let out = lay(t, Some(active_lines(t, row, row)), 1000.0);
        assert!(out.grids.is_empty());
        // too wide for the window: shown as source
        assert!(lay(t, None, 60.0).grids.is_empty());
    }

    #[test]
    fn secrets_lines_form_a_card() {
        let t = "## Keys\na::: 1\nlonger label::: 2\n\nother::: 3\n";
        let out = lay(t, None, 1000.0);
        assert_eq!(out.grids.len(), 2);
        assert_eq!(out.grids[0].rows.len(), 2);
        assert!(out.grids[0].secrets);
        // values aligned in one column, ':::' hidden
        let v1 = out.secrets[0].value_char;
        let v2 = out.secrets[1].value_char;
        let x = |c: usize| {
            let b = t.char_indices().nth(c).unwrap().0;
            section_at(&out, b).leading_space
        };
        assert!(x(v1) > x(v2));
        assert_eq!(
            section_at(&out, t.find(":::").unwrap()).format.color,
            Color32::TRANSPARENT
        );
        // the line under the cursor leaves the card
        let l = t.find("longer").unwrap();
        let out = lay(t, Some(active_lines(t, l, l)), 1000.0);
        assert_eq!(out.grids[0].rows.len(), 1);
    }

    #[test]
    fn secrets_found_and_hidden() {
        let t = "## API\nkey::: sk-abc_def_\n```\nnot::: secret\n```\n";
        let out = lay(t, None, 1000.0);
        assert_eq!(out.secrets.len(), 1);
        let s = &out.secrets[0];
        assert_eq!(&t[s.value.clone()], "sk-abc_def_");
        assert_eq!(s.value_char, t[..s.value.start].chars().count());
        // value section is transparent and carries leading space for the buttons
        let sec = out
            .job
            .sections
            .iter()
            .find(|x| x.byte_range.start.0 == s.value.start)
            .unwrap();
        assert_eq!(sec.format.color, Color32::TRANSPARENT);
        assert!(sec.leading_space > 0.0);
        // on the active line the raw value shows
        let act = active_lines(t, 8, 8);
        let out = lay(t, Some(act), 1000.0);
        let sec = out
            .job
            .sections
            .iter()
            .find(|x| x.byte_range.start.0 == s.value.start)
            .unwrap();
        assert_ne!(sec.format.color, Color32::TRANSPARENT);
    }

    #[test]
    fn sections_cover_text() {
        let t = "héllo *wörld* ✓\n- [x] task\n> quote\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let out = lay(t, Some(0..5), 1000.0);
        let mut pos = 0;
        for s in &out.job.sections {
            assert_eq!(s.byte_range.start.0, pos);
            assert!(t.is_char_boundary(s.byte_range.end.0));
            pos = s.byte_range.end.0;
        }
        assert_eq!(pos, t.len());
    }
}
