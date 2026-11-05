//! Markdown live preview: turns Markdown source into a styled `LayoutJob`
//! for the editor. Syntax markers are hidden (transparent, ~0 width) except
//! on the line(s) holding the cursor, so the text and the char indices never
//! change and editing stays exact.

use std::collections::HashSet;
use std::ops::Range;

use eframe::egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use eframe::egui::{Color32, FontFamily, FontId, Stroke};
use pulldown_cmark::{Event, Options, Parser, Tag};

use crate::secrets;
use crate::theme::Palette;

/// Width reserved in front of a secret value for the [show] and [copy] buttons.
pub const SECRET_BUTTONS_W: f32 = 46.0;
/// Extra width for the "••••••••" mask when the value is hidden.
pub const SECRET_MASK_W: f32 = 70.0;

const EMPH: u16 = 1;
const STRONG: u16 = 1 << 1;
const STRIKE: u16 = 1 << 2;
const CODE: u16 = 1 << 3;
const LINK: u16 = 1 << 4;
const QUOTE: u16 = 1 << 5;
const MARK: u16 = 1 << 6; // syntax marker, hidden off the active line
const DIM: u16 = 1 << 7; // structural syntax, always visible but faint
const CODEBLOCK: u16 = 1 << 8;
const TABLE: u16 = 1 << 9;
const BULLET: u16 = 1 << 10;
const SEC_LABEL: u16 = 1 << 11;
const SEC_SEP: u16 = 1 << 12; // the visible ':' of ':::'
const SEC_SEP_HIDE: u16 = 1 << 13; // the '::' that is hidden off the active line
const SEC_VALUE: u16 = 1 << 14;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct St {
    flags: u16,
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

pub struct Styled {
    pub job: LayoutJob,
    pub secrets: Vec<Secret>,
}

/// Lay out `text` as live-preview Markdown.
///
/// `active` is the byte range of the line(s) under the cursor/selection.
/// `revealed` holds the line numbers of secrets the user chose to show.
pub fn layout(
    text: &str,
    active: Option<Range<usize>>,
    revealed: &HashSet<usize>,
    pal: &Palette,
    size: f32,
) -> Styled {
    let mut st = vec![St::default(); text.len()];
    style_markdown(text, &mut st);
    let secret_spans = style_secrets(text, &mut st);

    let is_active = |i: usize| active.as_ref().is_some_and(|r| r.contains(&i) || r.end == i);
    let mut secrets: Vec<Secret> = secret_spans
        .into_iter()
        .map(|(line, value)| Secret {
            active: is_active(value.start),
            revealed: revealed.contains(&line),
            line,
            value,
            value_char: 0,
        })
        .collect();

    let mut job = LayoutJob {
        text: text.to_owned(),
        ..Default::default()
    };
    let n = text.len();
    let (mut i, mut chars, mut k) = (0usize, 0usize, 0usize);
    while i < n {
        let s = st[i];
        let act = is_active(i);
        // A secret value always starts its own section (it carries leading space).
        let sec_here = secrets.get(k).filter(|sec| sec.value.start == i);
        let mut j = i + 1;
        while j < n
            && st[j] == s
            && is_active(j) == act
            && secrets.get(k).is_none_or(|sec| sec.value.start != j)
        {
            j += 1;
        }
        while j < n && !text.is_char_boundary(j) {
            j += 1;
        }
        let mut leading = 0.0;
        let mut hidden_value = false;
        if let Some(sec) = sec_here {
            if !sec.active {
                leading = SECRET_BUTTONS_W + if sec.revealed { 0.0 } else { SECRET_MASK_W };
                hidden_value = !sec.revealed;
            }
            secrets[k].value_char = chars;
            k += 1;
        } else if s.flags & SEC_VALUE != 0 {
            // continuation of a value section (split by char boundary etc.)
            hidden_value = secrets
                .get(k.wrapping_sub(1))
                .is_some_and(|sec| !sec.active && !sec.revealed && sec.value.contains(&i));
        }
        let format = format_for(s, act, hidden_value, pal, size);
        job.sections.push(LayoutSection {
            leading_space: leading,
            byte_range: ByteIndex(i)..ByteIndex(j),
            format,
        });
        chars += text[i..j].chars().count();
        i = j;
    }
    Styled { job, secrets }
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
    if (f & MARK != 0 || f & SEC_SEP_HIDE != 0) && !act {
        return hidden();
    }
    if f & SEC_VALUE != 0 && hidden_value {
        return hidden();
    }
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
    if f & STRONG != 0 {
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
    if f & (CODE | CODEBLOCK | TABLE | SEC_VALUE) != 0 {
        fmt.font_id.family = FontFamily::Monospace;
        fmt.font_id.size *= 0.92;
    }
    if f & (CODE | CODEBLOCK) != 0 {
        fmt.background = pal.code_bg;
        fmt.color = pal.code;
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
    if f & SEC_SEP != 0 {
        fmt.color = pal.weak;
    }
    if f & (MARK | DIM | SEC_SEP_HIDE) != 0 {
        fmt.color = pal.weak;
        fmt.background = Color32::TRANSPARENT;
    }
    fmt
}

fn set(st: &mut [St], r: Range<usize>, flag: u16) {
    let end = r.end.min(st.len());
    for s in &mut st[r.start.min(end)..end] {
        s.flags |= flag;
    }
}

fn count_prefix(s: &str, c: u8) -> usize {
    s.bytes().take_while(|&b| b == c).count()
}

fn style_markdown(text: &str, st: &mut [St]) {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
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
            Event::Start(Tag::Table(_)) => set(st, r, TABLE),
            _ => {}
        }
    }
}

/// Mark `label::: value` lines. Returns (line number, value byte range).
fn style_secrets(text: &str, st: &mut [St]) -> Vec<(usize, Range<usize>)> {
    let mut out = Vec::new();
    let mut off = 0;
    for (line_no, line) in text.split_inclusive('\n').enumerate() {
        let start = off;
        off += line.len();
        if st.get(start).is_some_and(|s| s.flags & CODEBLOCK != 0) {
            continue;
        }
        let Some(sp) = secrets::parse_line(line) else {
            continue;
        };
        let abs = |r: Range<usize>| start + r.start..start + r.end;
        let (label, sep, value) = (abs(sp.label), abs(sp.sep), abs(sp.value));
        for s in &mut st[label.clone()] {
            s.flags = (s.flags & !(MARK | DIM)) | SEC_LABEL;
        }
        for (n, s) in st[sep.clone()].iter_mut().enumerate() {
            *s = St {
                flags: if n == 0 { SEC_SEP } else { SEC_SEP_HIDE },
                heading: s.heading,
            };
        }
        for s in &mut st[value.clone()] {
            *s = St {
                flags: SEC_VALUE,
                heading: 0,
            };
        }
        if !value.is_empty() {
            out.push((line_no, value));
        }
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

    fn flags_at(text: &str, needle: &str) -> u16 {
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

    #[test]
    fn secrets_found_and_hidden() {
        let t = "## API\nkey::: sk-abc_def_\n```\nnot::: secret\n```\n";
        let pal = Palette::new(true);
        let out = layout(t, None, &HashSet::new(), &pal, 14.0);
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
        let out = layout(t, Some(act), &HashSet::new(), &pal, 14.0);
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
        let out = layout(t, Some(0..5), &HashSet::new(), &Palette::new(false), 14.0);
        let mut pos = 0;
        for s in &out.job.sections {
            assert_eq!(s.byte_range.start.0, pos);
            assert!(t.is_char_boundary(s.byte_range.end.0));
            pos = s.byte_range.end.0;
        }
        assert_eq!(pos, t.len());
    }
}
