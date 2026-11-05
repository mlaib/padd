//! The Padd application: tabs, editor, dialogs, background crypto jobs.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use age::secrecy::SecretString;
use eframe::egui::{
    self, Align2, FontId, Galley, Key, KeyboardShortcut, Modifiers, RichText, TextBuffer, ViewportCommand,
};
use zeroize::{Zeroize, Zeroizing};

use crate::fileio::{self, Format, LoadError, Loaded};
use crate::markdown::{self, SECRET_BUTTONS_W, SECRET_MASK_W, Secret};
use crate::theme::Palette;
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
    live_preview: bool,
    wrap: bool,
    quitting: bool,
    title: String,
    focus_unlock: bool,
}

impl PaddApp {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        let ctx = cc.egui_ctx.clone();
        highlight::preload(move || ctx.request_repaint());
        let (tx, rx) = channel();
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
            live_preview: true,
            wrap: true,
            quitting: false,
            title: String::new(),
            focus_unlock: false,
        };
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
                let mut tab = Tab::new(id, Some(path), text, fmt);
                if tab.text.len() > MAX_EDIT {
                    let lines = std::iter::once(0)
                        .chain(tab.text.match_indices('\n').map(|(i, _)| i + 1))
                        .collect();
                    tab.body = Body::Large { lines };
                }
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
            }
            Ok(Loaded::Encrypted(cipher)) => {
                let id = self.id();
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
            tab.path = Some(path);
            tab.cache = None;
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

    // ---------------------------------------------------------------- ui

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
            self.live_preview = !self.live_preview;
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

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add(egui::Button::new("New").shortcut_text("Ctrl+N")).clicked() {
                    self.new_tab(false);
                }
                if ui.button("New secrets file…").clicked() {
                    self.new_tab(true);
                }
                if ui
                    .add(egui::Button::new("Open…").shortcut_text("Ctrl+O"))
                    .clicked()
                {
                    self.open_dialog(&ctx);
                }
                ui.separator();
                let has = !self.tabs.is_empty();
                if ui
                    .add_enabled(has, egui::Button::new("Save").shortcut_text("Ctrl+S"))
                    .clicked()
                {
                    self.save(&ctx, self.active, false);
                }
                if ui
                    .add_enabled(has, egui::Button::new("Save as…").shortcut_text("Ctrl+Shift+S"))
                    .clicked()
                {
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
                if ui
                    .add_enabled(enc && ready, egui::Button::new("Lock").shortcut_text("Ctrl+L"))
                    .clicked()
                {
                    self.lock(&ctx, self.active);
                }
                ui.separator();
                if ui
                    .add_enabled(has, egui::Button::new("Close tab").shortcut_text("Ctrl+W"))
                    .clicked()
                {
                    self.close_tab(&ctx, self.active, false);
                }
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.live_preview, "Live Markdown preview  (Ctrl+E)");
                ui.checkbox(&mut self.wrap, "Word wrap");
                ui.separator();
                egui::widgets::global_theme_preference_buttons(ui);
            });
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut close = None;
        egui::ScrollArea::horizontal()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (i, t) in self.tabs.iter().enumerate() {
                        let mut title = String::new();
                        if t.encrypted() {
                            title.push_str("🔒 ");
                        }
                        title.push_str(&t.name());
                        if t.dirty {
                            title.push_str(" ●");
                        }
                        let r = ui.selectable_label(i == self.active, title);
                        if r.clicked() {
                            self.active = i;
                        }
                        if r.middle_clicked() {
                            close = Some(i);
                        }
                        if let Some(p) = &t.path {
                            r.on_hover_text(p.display().to_string());
                        }
                        if ui.add(egui::Button::new("×").frame(false).small()).clicked() {
                            close = Some(i);
                        }
                        ui.separator();
                    }
                    if ui
                        .add(egui::Button::new("+").frame(false))
                        .on_hover_text("New tab")
                        .clicked()
                    {
                        self.new_tab(false);
                    }
                });
            });
        if let Some(i) = close {
            self.close_tab(&ctx, i, false);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(t) = self.tabs.get(self.active) {
                let kind = match t.kind {
                    Kind::Markdown => "Markdown",
                    Kind::Code => "Code",
                    Kind::Plain => "Text",
                };
                let mut info = format!("Ln {}, Col {}   {kind}", t.cursor.0, t.cursor.1);
                info.push_str(match t.fmt.encoding {
                    fileio::Encoding::Utf8 => "   UTF-8",
                    fileio::Encoding::Utf8Bom => "   UTF-8 BOM",
                    fileio::Encoding::Utf16Le => "   UTF-16 LE",
                    fileio::Encoding::Utf16Be => "   UTF-16 BE",
                });
                info.push_str(if t.fmt.crlf { "   CRLF" } else { "   LF" });
                if t.encrypted() {
                    info.push_str("   🔒 encrypted");
                }
                if t.saving {
                    info.push_str("   saving…");
                }
                if matches!(t.body, Body::Large { .. }) {
                    info.push_str("   large file: read-only");
                }
                ui.label(RichText::new(info).weak());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(at) = self.clipboard_clear_at {
                    let left = at.saturating_duration_since(Instant::now()).as_secs() + 1;
                    ui.label(RichText::new(format!("clipboard clears in {left}s")).weak());
                }
                if let Some((msg, when, err)) = &self.status
                    && when.elapsed() < Duration::from_secs(6)
                {
                    let color = if *err {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    ui.label(RichText::new(msg).color(color));
                }
            });
        });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.heading(RichText::new("Padd").size(36.0));
            ui.label(RichText::new("Markdown, structured text and secrets").weak());
            ui.add_space(16.0);
            if ui.button("Open file…  (Ctrl+O)").clicked() {
                self.open_dialog(&ctx);
            }
            if ui.button("New Markdown file  (Ctrl+N)").clicked() {
                self.new_tab(false);
            }
            if ui.button("New secrets file").clicked() {
                self.new_tab(true);
            }
            ui.add_space(8.0);
            ui.label(RichText::new("…or drop files here").weak());
        });
    }

    fn locked_view(&mut self, ui: &mut egui::Ui, idx: usize) {
        let ctx = ui.ctx().clone();
        let focus = std::mem::take(&mut self.focus_unlock);
        let mut submit = false;
        let name = self.tabs[idx].name();
        let Body::Locked { input, error, busy } = &mut self.tabs[idx].body else {
            return;
        };
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.label(RichText::new("🔒").size(40.0));
            ui.heading(name);
            ui.label(RichText::new("This file is encrypted. Enter its password.").weak());
            ui.add_space(10.0);
            let r = ui.add_enabled(
                !*busy,
                egui::TextEdit::singleline(input)
                    .password(true)
                    .hint_text("password")
                    .desired_width(260.0),
            );
            if focus {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
            ui.add_space(6.0);
            if *busy {
                ui.spinner();
            } else if ui.button("Unlock").clicked() {
                submit = true;
            }
            if let Some(e) = error {
                ui.colored_label(ui.visuals().error_fg_color, e.as_str());
            }
        });
        if submit {
            self.unlock(&ctx, idx);
        }
    }

    fn large_view(ui: &mut egui::Ui, tab: &Tab) {
        let Body::Large { lines } = &tab.body else { return };
        let font = FontId::monospace(13.0);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        egui::ScrollArea::both()
            .auto_shrink(false)
            .show_rows(ui, row_h, lines.len(), |ui, rows| {
                for r in rows {
                    let start = lines[r];
                    let end = lines.get(r + 1).map_or(tab.text.len(), |e| e - 1);
                    let line = &tab.text[start..end.max(start)];
                    ui.add(
                        egui::Label::new(RichText::new(line).font(font.clone()))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                }
            });
    }

    fn editor(&mut self, ui: &mut egui::Ui, idx: usize) {
        let ctx = ui.ctx().clone();
        let pal = Palette::new(ui.visuals().dark_mode);
        let live = self.live_preview;
        let wrap = self.wrap;
        let tab = &mut self.tabs[idx];
        let id = tab.editor_id();
        let rich = tab.text.len() <= MAX_RICH;

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
        let mono = tab.kind == Kind::Code || (!live && tab.kind == Kind::Markdown);

        let Tab {
            text,
            cache,
            hl,
            revealed,
            kind,
            ..
        } = tab;
        let kind = *kind;
        let margin = if md {
            egui::Margin::symmetric(28, 18)
        } else {
            egui::Margin::symmetric(10, 8)
        };
        let mut copy: Option<String> = None;
        let mut toggle: Option<usize> = None;
        let avail = ui.available_size();
        let scroll = if wrap {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both()
        };
        let edit = scroll.auto_shrink(false).id_salt(id).show(ui, |ui| {
            let out = {
                let mut layouter = |ui: &egui::Ui, buf: &dyn TextBuffer, wrap_width: f32| -> Arc<Galley> {
                    let text = buf.as_str();
                    let key = {
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        (
                            text,
                            &active,
                            wrap_width.to_bits(),
                            pal.dark,
                            md,
                            mono,
                            assets_ready,
                            kind as u8,
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
                    let (mut job, secrets) = if md {
                        let s = markdown::layout(text, active.clone(), revealed, &pal, 15.0);
                        (s.job, s.secrets)
                    } else if let Some(syn) = syntax {
                        (hl.job(text, syn, pal.dark, FontId::monospace(13.5)), Vec::new())
                    } else {
                        let font = if mono || kind != Kind::Markdown {
                            FontId::monospace(13.5)
                        } else {
                            FontId::proportional(15.0)
                        };
                        (
                            egui::text::LayoutJob::simple(text.to_owned(), font, pal.text, f32::INFINITY),
                            Vec::new(),
                        )
                    };
                    job.wrap.max_width = if wrap { wrap_width } else { f32::INFINITY };
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    *cache = Some(LayoutCache {
                        key,
                        galley: galley.clone(),
                        secrets,
                    });
                    galley
                };
                egui::TextEdit::multiline(&mut *text)
                    .id(id)
                    .frame(egui::Frame::NONE.inner_margin(margin))
                    .desired_width(if wrap { f32::INFINITY } else { 0.0 })
                    .min_size(avail)
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .show(ui)
            };
            let changed = out.response.changed();
            if changed {
                // Line numbers shift on edits: hide everything again rather
                // than reveal the wrong line.
                revealed.clear();
            }
            // Overlay [show] [copy] buttons and the mask on each secret.
            if let Some(c) = cache.as_ref() {
                for sec in c.secrets.iter().filter(|s| !s.active) {
                    let pos = out
                        .galley
                        .pos_from_cursor(egui::text::CCursor::new(sec.value_char));
                    let r = pos.translate(out.galley_pos.to_vec2());
                    let gap = SECRET_BUTTONS_W + if sec.revealed { 0.0 } else { SECRET_MASK_W };
                    let h = r.height().max(18.0);
                    let x0 = r.left() - gap;
                    let btn = |dx: f32| {
                        egui::Rect::from_min_size(egui::pos2(x0 + dx, r.top()), egui::vec2(20.0, h))
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
                        btn(22.0),
                        base.with("copy"),
                        Icon::Copy,
                        "Copy (clipboard clears in 30 s)",
                    ) && let Some(v) = text.get(sec.value.clone())
                    {
                        copy = Some(v.to_owned());
                    }
                    if !sec.revealed {
                        ui.painter().text(
                            egui::pos2(x0 + SECRET_BUTTONS_W, r.center().y),
                            Align2::LEFT_CENTER,
                            "••••••••",
                            FontId::monospace(13.0),
                            pal.weak,
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

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else { return };
        let mut action: Option<DialogAction> = None;
        match dialog {
            Dialog::SetPassword {
                p1,
                p2,
                error,
                delete_plain,
                has_plain,
                ..
            } => {
                egui::Modal::new(egui::Id::unique("padd-password")).show(ctx, |ui| {
                    ui.set_width(320.0);
                    ui.heading("Set a password");
                    ui.label(
                        RichText::new("One password per file. There is no way to recover a lost password.")
                            .weak(),
                    );
                    ui.add_space(8.0);
                    let r1 = ui.add(
                        egui::TextEdit::singleline(p1)
                            .password(true)
                            .hint_text("password"),
                    );
                    if !r1.has_focus() && p1.is_empty() && p2.is_empty() {
                        r1.request_focus();
                    }
                    let r2 = ui.add(
                        egui::TextEdit::singleline(p2)
                            .password(true)
                            .hint_text("repeat password"),
                    );
                    if *has_plain {
                        ui.checkbox(delete_plain, "Delete the unencrypted original after saving");
                    }
                    if let Some(e) = error {
                        ui.colored_label(ui.visuals().error_fg_color, e.as_str());
                    }
                    ui.add_space(6.0);
                    let enter = r2.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.horizontal(|ui| {
                        if ui.button("Encrypt & save").clicked() || enter {
                            if p1.chars().count() < MIN_PASSWORD {
                                *error = Some(format!("Use at least {MIN_PASSWORD} characters"));
                            } else if p1 != p2 {
                                *error = Some("Passwords do not match".into());
                            } else {
                                action = Some(DialogAction::PasswordOk);
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            action = Some(DialogAction::Cancel);
                        }
                    });
                });
            }
            Dialog::ConfirmClose(id) => {
                let id = *id;
                let name = self
                    .tab_index(id)
                    .map(|i| self.tabs[i].name())
                    .unwrap_or_default();
                egui::Modal::new(egui::Id::unique("padd-close")).show(ctx, |ui| {
                    ui.heading(format!("Save changes to {name}?"));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            action = Some(DialogAction::SaveTab(id));
                        }
                        if ui.button("Don't save").clicked() {
                            action = Some(DialogAction::CloseTab(id));
                        }
                        if ui.button("Cancel").clicked() {
                            action = Some(DialogAction::Cancel);
                        }
                    });
                });
            }
            Dialog::ConfirmQuit => {
                let names: Vec<String> = self.tabs.iter().filter(|t| t.dirty).map(Tab::name).collect();
                egui::Modal::new(egui::Id::unique("padd-quit")).show(ctx, |ui| {
                    ui.heading("Unsaved changes");
                    for n in &names {
                        ui.label(format!("• {n}"));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Quit without saving").clicked() {
                            action = Some(DialogAction::Quit);
                        }
                        if ui.button("Cancel").clicked() {
                            action = Some(DialogAction::Cancel);
                        }
                    });
                });
            }
        }
        match action {
            None => {}
            Some(DialogAction::Cancel) => self.dialog = None,
            Some(DialogAction::Quit) => {
                self.dialog = None;
                self.quitting = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            Some(DialogAction::CloseTab(id)) => {
                self.dialog = None;
                if let Some(i) = self.tab_index(id) {
                    self.close_tab(ctx, i, true);
                }
            }
            Some(DialogAction::SaveTab(id)) => {
                self.dialog = None;
                if let Some(i) = self.tab_index(id) {
                    self.save(ctx, i, false);
                }
            }
            Some(DialogAction::PasswordOk) => {
                let Some(Dialog::SetPassword {
                    tab,
                    p1,
                    p2,
                    delete_plain,
                    ..
                }) = self.dialog.take()
                else {
                    return;
                };
                let mut p2 = p2;
                p2.zeroize();
                let Some(i) = self.tab_index(tab) else { return };
                let t = &mut self.tabs[i];
                let was_plain = t.password.is_none() && t.cipher.is_none();
                t.password = Some(SecretString::from(p1));
                t.wants_encryption = false;
                if was_plain
                    && let Some(p) = t.path.clone()
                    && !p.to_string_lossy().ends_with(".enc")
                {
                    t.path = Some(PathBuf::from(format!("{}.enc", p.display())));
                    if delete_plain {
                        t.delete_after_save = Some(p);
                    }
                }
                t.dirty = true;
                t.edit_gen += 1;
                self.save(ctx, i, false);
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
}

#[derive(Clone, Copy)]
enum Icon {
    Eye,
    EyeOff,
    Copy,
}

/// A small vector-drawn icon button: identical on every OS, no icon font needed.
fn icon_button(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id, icon: Icon, tip: &str) -> bool {
    let resp = ui
        .interact(rect, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip);
    let color = if resp.hovered() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let stroke = egui::Stroke::new(1.3, color);
    let p = ui.painter();
    let c = rect.center();
    match icon {
        Icon::Eye | Icon::EyeOff => {
            p.add(egui::Shape::ellipse_stroke(c, egui::vec2(7.0, 4.2), stroke));
            p.circle_filled(c, 2.0, color);
            if matches!(icon, Icon::EyeOff) {
                p.line_segment([c + egui::vec2(-7.0, 5.0), c + egui::vec2(7.0, -5.0)], stroke);
            }
        }
        Icon::Copy => {
            let size = egui::vec2(7.0, 8.5);
            let back = egui::Rect::from_min_size(c + egui::vec2(-5.5, -5.5), size);
            let front = back.translate(egui::vec2(3.0, 3.0));
            p.rect_stroke(back, 1.5, stroke, egui::StrokeKind::Inside);
            p.rect_filled(front, 1.5, ui.visuals().panel_fill);
            p.rect_stroke(front, 1.5, stroke, egui::StrokeKind::Inside);
        }
    }
    resp.clicked()
}

enum DialogAction {
    Cancel,
    Quit,
    PasswordOk,
    SaveTab(u64),
    CloseTab(u64),
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

        egui::Panel::top("padd-menu").show(ui, |ui| self.menu_bar(ui));
        if !self.tabs.is_empty() {
            egui::Panel::top("padd-tabs").show(ui, |ui| self.tab_bar(ui));
        }
        egui::Panel::bottom("padd-status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(0))
            .show(ui, |ui| {
                if self.tabs.is_empty() {
                    self.welcome(ui);
                    return;
                }
                let idx = self.active.min(self.tabs.len() - 1);
                self.active = idx;
                match self.tabs[idx].body {
                    Body::Locked { .. } => self.locked_view(ui, idx),
                    Body::Large { .. } => Self::large_view(ui, &self.tabs[idx]),
                    Body::Ready => self.editor(ui, idx),
                }
            });

        self.dialogs(&ctx);
        self.update_title(&ctx);
        self.timers(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for t in &mut self.tabs {
            t.text.zeroize();
        }
    }
}
