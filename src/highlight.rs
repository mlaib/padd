//! Syntax highlighting for structured text (JSON, YAML, XML, code…).
//!
//! * syntect's grammar set is deserialised on a background thread at startup,
//!   so it never delays the first frame.
//! * Highlighting is incremental: each line keeps the parser state it started
//!   with and its result. After an edit only the changed lines are re-parsed,
//!   and parsing stops as soon as the state matches the cached state again.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use eframe::egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use eframe::egui::{Color32, FontId};
use syntect::highlighting::{HighlightIterator, HighlightState, Highlighter as SynHl, Theme, ThemeSet};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

pub struct Assets {
    pub syntaxes: SyntaxSet,
    dark: Theme,
    light: Theme,
}

static ASSETS: OnceLock<Assets> = OnceLock::new();

/// Load the grammars in the background; `done` is called when they are ready.
pub fn preload(done: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        ASSETS.get_or_init(|| {
            let mut themes = ThemeSet::load_defaults().themes;
            Assets {
                syntaxes: SyntaxSet::load_defaults_newlines(),
                dark: themes.remove("base16-ocean.dark").unwrap_or_default(),
                light: themes.remove("InspiredGitHub").unwrap_or_default(),
            }
        });
        done();
    });
}

pub fn assets() -> Option<&'static Assets> {
    ASSETS.get()
}

/// Find the grammar for a file name (extension first, then well-known names).
pub fn syntax_for(name: &str, first_line: &str) -> Option<&'static SyntaxReference> {
    let a = assets()?;
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map_or(lower.as_str(), |(_, e)| e);
    let alias = match ext {
        "jsonc" | "json5" | "geojson" | "ipynb" => "json",
        "yml" => "yaml",
        "htm" | "xhtml" => "html",
        "svg" | "plist" | "csproj" | "xaml" => "xml",
        "zsh" | "bash" | "env" | "envrc" | "profile" | "bashrc" | "zshrc" => "sh",
        "conf" | "cfg" | "ini" | "toml" | "properties" => "", // no grammar in the default set
        other => other,
    };
    if alias.is_empty() {
        return None;
    }
    a.syntaxes
        .find_syntax_by_extension(alias)
        .or_else(|| a.syntaxes.find_syntax_by_first_line(first_line))
        .filter(|s| s.name != "Plain Text")
}

#[derive(Clone)]
struct Line {
    hash: u64,
    start: (ParseState, HighlightState),
    end: (ParseState, HighlightState),
    /// (byte length, colour) runs within the line
    runs: Vec<(u32, Color32)>,
}

/// Per-document incremental highlighter.
#[derive(Default)]
pub struct Highlighter {
    syntax: Option<String>,
    dark: bool,
    lines: Vec<Line>,
}

impl Highlighter {
    pub fn reset(&mut self) {
        self.lines.clear();
        self.syntax = None;
    }

    /// Build a highlighted `LayoutJob` for `text`.
    pub fn job(&mut self, text: &str, syntax: &SyntaxReference, dark: bool, font: FontId) -> LayoutJob {
        let a = assets().expect("assets loaded");
        if self.syntax.as_deref() != Some(syntax.name.as_str()) || self.dark != dark {
            self.lines.clear();
            self.syntax = Some(syntax.name.clone());
            self.dark = dark;
        }
        let theme = if dark { &a.dark } else { &a.light };
        let hl = SynHl::new(theme);
        let mut state = (
            ParseState::new(syntax),
            HighlightState::new(&hl, ScopeStack::new()),
        );
        let mut job = LayoutJob {
            text: text.to_owned(),
            ..Default::default()
        };
        let src: Vec<&str> = text.split_inclusive('\n').collect();
        let hashes: Vec<u64> = src
            .iter()
            .map(|l| {
                let mut h = DefaultHasher::new();
                l.hash(&mut h);
                h.finish()
            })
            .collect();
        // Align old and new lines on their common prefix and suffix, so an
        // inserted or deleted line does not invalidate everything after it.
        let (old_n, new_n) = (self.lines.len(), src.len());
        let prefix = self
            .lines
            .iter()
            .zip(&hashes)
            .take_while(|(o, h)| o.hash == **h)
            .count();
        let max_suffix = old_n.min(new_n) - prefix;
        let suffix = self.lines[old_n - max_suffix..]
            .iter()
            .rev()
            .zip(hashes[new_n - max_suffix..].iter().rev())
            .take_while(|(o, h)| o.hash == **h)
            .count();
        let mut new_lines = Vec::with_capacity(new_n);
        let mut off = 0usize;
        for (i, (&line, &hash)) in src.iter().zip(&hashes).enumerate() {
            let old_i = if i < prefix {
                Some(i)
            } else if i >= new_n - suffix {
                Some(i + old_n - new_n)
            } else {
                None
            };
            let cached = old_i
                .and_then(|o| self.lines.get(o))
                .filter(|c| c.hash == hash && c.start == state);
            let entry = match cached {
                Some(c) => c.clone(),
                None => {
                    let start = state.clone();
                    let (mut ps, mut hs) = state.clone();
                    let runs = match ps.parse_line(line, &a.syntaxes) {
                        Ok(ops) => HighlightIterator::new(&mut hs, &ops, line, &hl)
                            .map(|(style, s)| {
                                let c = style.foreground;
                                (s.len() as u32, Color32::from_rgb(c.r, c.g, c.b))
                            })
                            .collect(),
                        Err(_) => vec![(line.len() as u32, Color32::GRAY)],
                    };
                    Line {
                        hash,
                        start,
                        end: (ps, hs),
                        runs,
                    }
                }
            };
            state = entry.end.clone();
            for &(len, color) in &entry.runs {
                let len = len as usize;
                push(&mut job, off..off + len, color, &font);
                off += len;
            }
            new_lines.push(entry);
        }
        self.lines = new_lines;
        job
    }
}

fn push(job: &mut LayoutJob, r: std::ops::Range<usize>, color: Color32, font: &FontId) {
    if r.is_empty() {
        return;
    }
    if let Some(last) = job.sections.last_mut()
        && last.format.color == color
        && last.byte_range.end.0 == r.start
    {
        last.byte_range.end = ByteIndex(r.end);
        return;
    }
    job.sections.push(LayoutSection {
        leading_space: 0.0,
        byte_range: ByteIndex(r.start)..ByteIndex(r.end),
        format: TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incremental_matches_full() {
        let (tx, rx) = std::sync::mpsc::channel();
        preload(move || tx.send(()).unwrap());
        rx.recv().unwrap();
        let syn = syntax_for("a.json", "").expect("json grammar");
        let font = FontId::monospace(13.0);
        let a = "{\n  \"a\": 1,\n  \"b\": \"x\"\n}\n";
        let b = "{\n  \"a\": 12,\n  \"b\": \"x\"\n}\n";
        let mut inc = Highlighter::default();
        inc.job(a, syn, true, font.clone());
        let j1 = inc.job(b, syn, true, font.clone());
        let j2 = Highlighter::default().job(b, syn, true, font);
        assert_eq!(j1.sections.len(), j2.sections.len());
        for (x, y) in j1.sections.iter().zip(&j2.sections) {
            assert_eq!(x.byte_range, y.byte_range);
            assert_eq!(x.format.color, y.format.color);
        }
    }
}
