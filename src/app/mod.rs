//! The Padd application state: tabs, files, encryption jobs.
//! The UI lives in the submodules.

mod chrome;
mod dialogs;
mod editor;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use age::secrecy::SecretString;
use eframe::egui::{self, Galley, Key, KeyboardShortcut, Modifiers, ViewportCommand};
use zeroize::{Zeroize, Zeroizing};

use crate::fileio::{self, Format, LoadError, Loaded};
use crate::markdown::Secret;
use crate::settings::{self, Settings, ThemeChoice};
use crate::{crypto, highlight, secrets};

/// Above this size, styling is switched off (plain text).
const MAX_RICH: usize = 1024 * 1024;
/// Above this size, files open in the virtualised read-only viewer.
const MAX_EDIT: usize = 4 * 1024 * 1024;
const CLIPBOARD_CLEAR: Duration = Duration::from_secs(30);
const AUTO_LOCK: Duration = Duration::from_secs(10 * 60);
const MIN_PASSWORD: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Markdown,
    Code,
    Plain,
}

fn kind_for(name: &str) -> Kind {
    let lower = name.to_ascii_lowercase();
    let lower = lower.strip_suffix(".enc").unwrap_or(&lower);
    match lower.rsplit_once('.').map(|(_, e)| e) {
        Some("md" | "markdown" | "mdown" | "mkd" | "mdx") => Kind::Markdown,
        Some("txt" | "text" | "log") | None => Kind::Plain,
        Some(_) => Kind::Code,
    }
}

enum Body {
    Ready,
    Locked {
        input: String,
        error: Option<String>,
        busy: bool,
    },
    Large {
        lines: Vec<usize>,
    },
}

struct LayoutCache {
    key: u64,
    galley: Arc<Galley>,
    secrets: Vec<Secret>,
    grids: Vec<crate::markdown::Grid>,
}

struct Tab {
    id: u64,
    path: Option<PathBuf>,
    kind: Kind,
    text: String,
    fmt: Format,
    body: Body,
    dirty: bool,
    edit_gen: u64,
    saving: bool,
    /// Set for encrypted documents (kept only while unlocked).
    password: Option<SecretString>,
    /// Last ciphertext read or written, used to re-lock without disk access.
    cipher: Option<Vec<u8>>,
    /// Delete this plaintext file after the first successful encrypted save.
    delete_after_save: Option<PathBuf>,
    /// Save (as encrypted) once a password is set — "New secrets file".
    wants_encryption: bool,
    revealed: HashSet<usize>,
    cache: Option<LayoutCache>,
    hl: highlight::Highlighter,
    cursor: (usize, usize),
}

impl Tab {
    fn new(id: u64, path: Option<PathBuf>, text: String, fmt: Format) -> Self {
        let kind = path
            .as_deref()
            .map_or(Kind::Markdown, |p| kind_for(&file_name(p)));
        Self {
            id,
            path,
            kind,
            text,
            fmt,
            body: Body::Ready,
            dirty: false,
            edit_gen: 0,
            saving: false,
            password: None,
            cipher: None,
            delete_after_save: None,
            wants_encryption: false,
            revealed: HashSet::new(),
            cache: None,
            hl: Default::default(),
            cursor: (1, 1),
        }
    }

    fn name(&self) -> String {
        self.path.as_deref().map_or_else(|| "Untitled".into(), file_name)
    }

    fn encrypted(&self) -> bool {
        self.password.is_some() || self.cipher.is_some() || matches!(self.body, Body::Locked { .. })
    }

    fn editor_id(&self) -> egui::Id {
        egui::Id::unique(("padd-editor", self.id))
    }

    /// Wipe the plaintext and every egui copy we control (undo history, galley).
    fn wipe(&mut self, ctx: &egui::Context) {
        self.text.zeroize();
        self.cache = None;
        self.hl.reset();
        self.revealed.clear();
        let id = self.editor_id();
        ctx.data_mut(|d| d.remove::<egui::text_edit::TextEditState>(id));
    }
}

impl Drop for Tab {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

enum Job {
    Decrypted {
        tab: u64,
        pass: SecretString,
        result: Result<String, String>,
    },
    Saved {
        tab: u64,
        generation: u64,
        result: Result<Option<Vec<u8>>, String>,
    },
}

enum Dialog {
    SetPassword {
        tab: u64,
        p1: String,
        p2: String,
        error: Option<String>,
        delete_plain: bool,
        has_plain: bool,
    },
    ConfirmClose(u64),
    ConfirmQuit,
    Shortcuts,
    About,
}

/// Find bar state (Ctrl+F).
#[derive(Default)]
struct Find {
    query: String,
    /// Char ranges of matches in the active document.
    matches: Vec<(usize, usize)>,
    current: usize,
    /// What `matches` was computed for: (tab id, edit generation, query).
    key: (u64, u64, String),
    focus: bool,
    /// Select and scroll to the current match on the next frame.
    jump: bool,
}

pub struct PaddApp {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    dialog: Option<Dialog>,
    status: Option<(String, Instant, bool)>,
    clipboard_clear_at: Option<Instant>,
    tx: Sender<Job>,
    rx: Receiver<Job>,
    last_input: Instant,
    settings: Settings,
    saved_settings: Settings,
    find: Option<Find>,
    logo: egui::TextureHandle,
    quitting: bool,
    title: String,
    focus_unlock: bool,
}

impl PaddApp {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        let ctx = cc.egui_ctx.clone();
        highlight::preload(move || ctx.request_repaint());
        let (tx, rx) = channel();
        crate::theme::install(&cc.egui_ctx);
        let settings = Settings::load();
        let logo = cc.egui_ctx.load_texture(
            "padd-logo",
            egui::ColorImage::from_rgba_unmultiplied([128, 128], crate::ICON_RGBA),
            egui::TextureOptions::LINEAR,
        );
        let mut app = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            dialog: None,
            status: None,
            clipboard_clear_at: None,
            tx,
            rx,
            last_input: Instant::now(),
            saved_settings: settings.clone(),
            settings,
            find: None,
            logo,
            quitting: false,
            title: String::new(),
            focus_unlock: false,
        };
        app.apply_view_settings(&cc.egui_ctx);
        for f in files {
            app.open_path(&cc.egui_ctx, f);
        }
        app
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn info(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now(), false));
    }

    fn error(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now(), true));
    }

    // ---------------------------------------------------------------- files

    fn new_tab(&mut self, secrets: bool) {
        let id = self.id();
        let text = if secrets {
            secrets::TEMPLATE.to_owned()
        } else {
            String::new()
        };
        let mut tab = Tab::new(id, None, text, Format::default());
        tab.wants_encryption = secrets;
        tab.dirty = secrets;
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let mut dlg = rfd::FileDialog::new();
        if let Some(dir) = self.current_dir() {
            dlg = dlg.set_directory(dir);
        }
        for p in dlg.pick_files().unwrap_or_default() {
            self.open_path(ctx, p);
        }
    }

    fn current_dir(&self) -> Option<PathBuf> {
        self.tabs
            .get(self.active)?
            .path
            .as_ref()?
            .parent()
            .map(Path::to_path_buf)
    }

    fn open_path(&mut self, ctx: &egui::Context, path: PathBuf) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if let Some(i) = self.tabs.iter().position(|t| t.path.as_deref() == Some(&path)) {
            self.active = i;
            return;
        }
        match fileio::load(&path) {
            Ok(Loaded::Text(text, fmt)) => {
                let id = self.id();
                let mut tab = Tab::new(id, Some(path.clone()), text, fmt);
                if tab.text.len() > MAX_EDIT {
                    let lines = std::iter::once(0)
                        .chain(tab.text.match_indices('\n').map(|(i, _)| i + 1))
                        .collect();
                    tab.body = Body::Large { lines };
                }
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                self.settings.push_recent(path);
            }
            Ok(Loaded::Encrypted(cipher)) => {
                let id = self.id();
                self.settings.push_recent(path.clone());
                let mut tab = Tab::new(id, Some(path), String::new(), Format::default());
                tab.cipher = Some(cipher);
                tab.body = Body::Locked {
                    input: String::new(),
                    error: None,
                    busy: false,
                };
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                self.focus_unlock = true;
            }
            Err(LoadError::Binary) => self.error(format!("{}: binary file, not opened", file_name(&path))),
            Err(e) => self.error(format!("{}: {e}", file_name(&path))),
        }
        ctx.request_repaint();
    }

    fn save(&mut self, ctx: &egui::Context, idx: usize, save_as: bool) {
        let Some(tab) = self.tabs.get_mut(idx) else { return };
        if !matches!(tab.body, Body::Ready) || tab.saving {
            return;
        }
        if tab.wants_encryption && tab.password.is_none() {
            let id = tab.id;
            self.dialog = Some(Dialog::SetPassword {
                tab: id,
                p1: String::new(),
                p2: String::new(),
                error: None,
                delete_plain: false,
                has_plain: false,
            });
            return;
        }
        if save_as || tab.path.is_none() {
            let encrypted = tab.password.is_some();
            let mut name = tab.path.as_deref().map_or_else(
                || {
                    if encrypted {
                        "secrets.md.enc".into()
                    } else {
                        "untitled.md".into()
                    }
                },
                file_name,
            );
            if encrypted && !name.ends_with(".enc") {
                name.push_str(".enc");
            }
            let mut dlg = rfd::FileDialog::new().set_file_name(&name);
            if let Some(dir) = tab.path.as_deref().and_then(Path::parent) {
                dlg = dlg.set_directory(dir);
            }
            let Some(mut path) = dlg.save_file() else { return };
            if encrypted && !path.to_string_lossy().ends_with(".enc") {
                path = PathBuf::from(format!("{}.enc", path.display()));
            }
            tab.kind = kind_for(&file_name(&path));
            tab.path = Some(path.clone());
            tab.cache = None;
            self.settings.push_recent(path);
        }
        let path = tab.path.clone().expect("path set");
        let generation = tab.edit_gen;
        match &tab.password {
            None => {
                let bytes = fileio::encode(&tab.text, tab.fmt);
                match fileio::atomic_write(&path, &bytes) {
                    Ok(()) => {
                        tab.dirty = false;
                        let msg = format!("Saved {}", file_name(&path));
                        self.info(msg);
                    }
                    Err(e) => self.error(format!("Save failed: {e}")),
                }
            }
            Some(pass) => {
                // Encrypt on a worker thread: scrypt takes ~0.5 s by design.
                tab.saving = true;
                let plain = Zeroizing::new(fileio::encode(&tab.text, tab.fmt));
                let pass = pass.clone();
                let (tx, ctx, id) = (self.tx.clone(), ctx.clone(), tab.id);
                std::thread::spawn(move || {
                    let result = crypto::encrypt(&plain, &pass)
                        .map_err(|e| e.to_string())
                        .and_then(|ct| {
                            fileio::atomic_write(&path, &ct).map_err(|e| e.to_string())?;
                            Ok(Some(ct))
                        });
                    let _ = tx.send(Job::Saved {
                        tab: id,
                        generation,
                        result,
                    });
                    ctx.request_repaint();
                });
            }
        }
    }

    fn unlock(&mut self, ctx: &egui::Context, idx: usize) {
        let tab = &mut self.tabs[idx];
        let (Body::Locked { input, error, busy }, Some(cipher)) = (&mut tab.body, &tab.cipher) else {
            return;
        };
        if *busy || input.is_empty() {
            return;
        }
        *busy = true;
        *error = None;
        let pass = SecretString::from(std::mem::take(input));
        let cipher = cipher.clone();
        let (tx, ctx, id) = (self.tx.clone(), ctx.clone(), tab.id);
        std::thread::spawn(move || {
            let result = crypto::decrypt(&cipher, &pass)
                .map_err(|e| e.to_string())
                .and_then(|mut pt| {
                    let s = String::from_utf8(std::mem::take(&mut *pt))
                        .map_err(|_| "decrypted content is not UTF-8 text".to_string())?;
                    Ok(s)
                });
            let _ = tx.send(Job::Decrypted {
                tab: id,
                pass,
                result,
            });
            ctx.request_repaint();
        });
    }

    fn lock(&mut self, ctx: &egui::Context, idx: usize) {
        let tab = &mut self.tabs[idx];
        if tab.password.is_none() || tab.cipher.is_none() {
            return;
        }
        if tab.dirty || tab.saving {
            self.error("Save before locking");
            return;
        }
        tab.wipe(ctx);
        tab.password = None;
        tab.body = Body::Locked {
            input: String::new(),
            error: None,
            busy: false,
        };
        self.focus_unlock = true;
    }

    fn poll_jobs(&mut self) {
        while let Ok(job) = self.rx.try_recv() {
            match job {
                Job::Decrypted { tab, pass, result } => {
                    let Some(i) = self.tab_index(tab) else { continue };
                    let t = &mut self.tabs[i];
                    match result {
                        Ok(mut text) => {
                            t.fmt.crlf = text.contains("\r\n");
                            if t.fmt.crlf {
                                let normal = text.replace("\r\n", "\n");
                                text.zeroize();
                                text = normal;
                            }
                            t.text = text;
                            t.password = Some(pass);
                            t.body = Body::Ready;
                            t.dirty = false;
                            self.last_input = Instant::now();
                        }
                        Err(e) => {
                            t.body = Body::Locked {
                                input: String::new(),
                                error: Some(e),
                                busy: false,
                            };
                            self.focus_unlock = true;
                        }
                    }
                }
                Job::Saved {
                    tab,
                    generation,
                    result,
                } => {
                    let Some(i) = self.tab_index(tab) else { continue };
                    let t = &mut self.tabs[i];
                    t.saving = false;
                    match result {
                        Ok(cipher) => {
                            if cipher.is_some() {
                                t.cipher = cipher;
                            }
                            if t.edit_gen == generation {
                                t.dirty = false;
                            }
                            t.wants_encryption = false;
                            let mut msg = format!("Saved encrypted {}", t.name());
                            if let Some(plain) = t.delete_after_save.take() {
                                match std::fs::remove_file(&plain) {
                                    Ok(()) => {
                                        msg.push_str(&format!(" · deleted plaintext {}", file_name(&plain)))
                                    }
                                    Err(e) => msg.push_str(&format!(" · could not delete plaintext: {e}")),
                                }
                            }
                            self.info(msg);
                        }
                        Err(e) => self.error(format!("Save failed: {e}")),
                    }
                }
            }
        }
    }

    fn close_tab(&mut self, ctx: &egui::Context, idx: usize, force: bool) {
        if idx >= self.tabs.len() {
            return;
        }
        if self.tabs[idx].dirty && !force {
            self.dialog = Some(Dialog::ConfirmClose(self.tabs[idx].id));
            return;
        }
        let mut tab = self.tabs.remove(idx);
        tab.wipe(ctx);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
    }

    fn copy_secret(&mut self, ctx: &egui::Context, value: String) {
        ctx.copy_text(value);
        self.clipboard_clear_at = Some(Instant::now() + CLIPBOARD_CLEAR);
    }
}

// -------------------------------------------------------------------- view & frame

impl PaddApp {
    fn apply_view_settings(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.settings.theme {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        });
        ctx.set_zoom_factor(self.settings.ui_scale);
    }

    fn set_text_size(&mut self, size: f32) {
        let size = (size * 2.0).round() / 2.0;
        self.settings.text_size = size.clamp(settings::TEXT_SIZE_MIN, settings::TEXT_SIZE_MAX);
    }

    fn open_find(&mut self) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        if !matches!(t.body, Body::Ready) {
            return;
        }
        let find = self.find.get_or_insert_with(Find::default);
        find.focus = true;
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let cmd_shift = |k| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, k);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if pressed(cmd_shift(Key::S)) {
            self.save(ctx, self.active, true);
        } else if pressed(cmd(Key::S)) {
            self.save(ctx, self.active, false);
        }
        if pressed(cmd(Key::O)) {
            self.open_dialog(ctx);
        }
        if pressed(cmd(Key::N)) {
            self.new_tab(false);
        }
        if pressed(cmd(Key::W)) {
            self.close_tab(ctx, self.active, false);
        }
        if pressed(cmd(Key::L)) && !self.tabs.is_empty() {
            self.lock(ctx, self.active);
        }
        if pressed(cmd(Key::E)) {
            self.settings.live_preview = !self.settings.live_preview;
        }
        if pressed(cmd(Key::F)) {
            self.open_find();
        }
        // Text size: Ctrl + / Ctrl - / Ctrl 0, and Ctrl + mouse wheel.
        if pressed(cmd(Key::Plus)) || pressed(cmd(Key::Equals)) || pressed(cmd_shift(Key::Equals)) {
            self.set_text_size(self.settings.text_size + 1.0);
        }
        if pressed(cmd(Key::Minus)) {
            self.set_text_size(self.settings.text_size - 1.0);
        }
        if pressed(cmd(Key::Num0)) {
            self.set_text_size(settings::TEXT_SIZE_DEFAULT);
        }
        let zoom = ctx.input(|i| i.zoom_delta());
        if zoom != 1.0 {
            self.set_text_size(self.settings.text_size * zoom);
        }
        let n = self.tabs.len();
        if n > 0 {
            if pressed(KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::SHIFT,
                Key::Tab,
            )) {
                self.active = (self.active + n - 1) % n;
            } else if pressed(KeyboardShortcut::new(Modifiers::CTRL, Key::Tab)) {
                self.active = (self.active + 1) % n;
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.tabs.get(self.active) {
            Some(t) => format!("{}{} — Padd", t.name(), if t.dirty { " ●" } else { "" }),
            None => "Padd".into(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn timers(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if ctx.input(|i| !i.events.is_empty() || i.pointer.is_moving()) {
            self.last_input = now;
        }
        if let Some(at) = self.clipboard_clear_at {
            if now >= at {
                ctx.copy_text(String::new());
                self.clipboard_clear_at = None;
                self.info("Clipboard cleared");
            } else {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }
        // Auto-lock idle, saved, unlocked encrypted tabs.
        let lockable: Vec<usize> = (0..self.tabs.len())
            .filter(|&i| {
                let t = &self.tabs[i];
                t.password.is_some() && !t.dirty && !t.saving && matches!(t.body, Body::Ready)
            })
            .collect();
        if !lockable.is_empty() {
            let idle = now - self.last_input;
            if idle >= AUTO_LOCK {
                for i in lockable {
                    self.lock(ctx, i);
                }
                self.info("Encrypted tabs locked after inactivity");
            } else {
                ctx.request_repaint_after(AUTO_LOCK - idle);
            }
        }
        if let Some((_, when, _)) = &self.status {
            let left = Duration::from_secs(6).saturating_sub(when.elapsed());
            if !left.is_zero() {
                ctx.request_repaint_after(left);
            }
        }
    }

    /// Persist settings when they change (cheap: compared once per frame).
    fn persist(&mut self, ctx: &egui::Context) {
        if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
            let size = (
                r.width() * self.settings.ui_scale,
                r.height() * self.settings.ui_scale,
            );
            if !ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
                self.settings.window = Some(size);
            }
        }
        let mut cmp = self.settings.clone();
        cmp.window = self.saved_settings.window;
        if cmp != self.saved_settings {
            if cmp.theme != self.saved_settings.theme || cmp.ui_scale != self.saved_settings.ui_scale {
                self.apply_view_settings(ctx);
            }
            self.settings.save();
            self.saved_settings = self.settings.clone();
        }
    }
}

impl eframe::App for PaddApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_jobs();
        if self.dialog.is_none() {
            self.shortcuts(&ctx);
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        for p in dropped {
            self.open_path(&ctx, p);
        }

        if ctx.input(|i| i.viewport().close_requested())
            && !self.quitting
            && self.tabs.iter().any(|t| t.dirty)
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::ConfirmQuit);
        }

        self.chrome(ui);
        self.dialogs(&ctx);
        self.update_title(&ctx);
        self.timers(&ctx);
        self.persist(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.settings.save();
        for t in &mut self.tabs {
            t.text.zeroize();
        }
    }
}
