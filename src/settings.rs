//! User preferences and recent files, kept in a tiny `key = value` file
//! (no serde: a few lines of parsing beat a few hundred KB of dependencies).
//!
//! Location: `$XDG_CONFIG_HOME/padd/settings.conf` (Linux),
//! `~/Library/Application Support/Padd/settings.conf` (macOS),
//! `%APPDATA%\Padd\settings.conf` (Windows).

use std::path::PathBuf;

pub const TEXT_SIZE_DEFAULT: f32 = 15.0;
pub const TEXT_SIZE_MIN: f32 = 10.0;
pub const TEXT_SIZE_MAX: f32 = 32.0;
const MAX_RECENT: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub text_size: f32,
    pub ui_scale: f32,
    pub theme: ThemeChoice,
    pub live_preview: bool,
    pub wrap: bool,
    /// Markdown in a centred column of comfortable width.
    pub readable_width: bool,
    pub window: Option<(f32, f32)>,
    pub recent: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            text_size: TEXT_SIZE_DEFAULT,
            ui_scale: 1.0,
            theme: ThemeChoice::System,
            live_preview: true,
            wrap: true,
            readable_width: true,
            window: None,
            recent: Vec::new(),
        }
    }
}

fn path() -> Option<PathBuf> {
    let env = |k| std::env::var_os(k).map(PathBuf::from);
    let dir = if cfg!(windows) {
        env("APPDATA")?.join("Padd")
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support/Padd")
    } else {
        env("XDG_CONFIG_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| env("HOME").map(|h| h.join(".config")))?
            .join("padd")
    };
    Some(dir.join("settings.conf"))
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else {
            return s;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            let flag = v == "true";
            match k {
                "text_size" => {
                    if let Ok(x) = v.parse::<f32>() {
                        s.text_size = x.clamp(TEXT_SIZE_MIN, TEXT_SIZE_MAX);
                    }
                }
                "ui_scale" => {
                    if let Ok(x) = v.parse::<f32>() {
                        s.ui_scale = x.clamp(0.75, 2.0);
                    }
                }
                "theme" => {
                    s.theme = match v {
                        "light" => ThemeChoice::Light,
                        "dark" => ThemeChoice::Dark,
                        _ => ThemeChoice::System,
                    }
                }
                "live_preview" => s.live_preview = flag,
                "wrap" => s.wrap = flag,
                "readable_width" => s.readable_width = flag,
                "window" => {
                    if let Some((w, h)) = v.split_once('x')
                        && let (Ok(w), Ok(h)) = (w.parse(), h.parse())
                    {
                        s.window = Some((w, h));
                    }
                }
                "recent" if s.recent.len() < MAX_RECENT => s.recent.push(PathBuf::from(v)),
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        let theme = match self.theme {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        };
        let mut out = format!(
            "# Padd settings\ntext_size = {}\nui_scale = {}\ntheme = {theme}\nlive_preview = {}\nwrap = {}\nreadable_width = {}\n",
            self.text_size, self.ui_scale, self.live_preview, self.wrap, self.readable_width
        );
        if let Some((w, h)) = self.window {
            out.push_str(&format!("window = {}x{}\n", w.round(), h.round()));
        }
        for r in &self.recent {
            out.push_str(&format!("recent = {}\n", r.display()));
        }
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = crate::fileio::atomic_write(&p, out.as_bytes());
    }

    pub fn push_recent(&mut self, p: PathBuf) {
        self.recent.retain(|r| r != &p);
        self.recent.insert(0, p);
        self.recent.truncate(MAX_RECENT);
    }
}
