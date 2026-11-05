//! Modal dialogs: password, unsaved changes, shortcuts, about.

use std::path::PathBuf;

use age::secrecy::SecretString;
use eframe::egui::{self, Key, RichText, ViewportCommand, vec2};
use zeroize::Zeroize;

use super::chrome::{primary_button, secondary_button};
use super::{Dialog, MIN_PASSWORD, PaddApp, Tab};
use crate::theme::Palette;

enum Action {
    Cancel,
    Quit,
    PasswordOk,
    SaveTab(u64),
    CloseTab(u64),
}

fn title(ui: &mut egui::Ui, pal: &Palette, text: &str) {
    ui.label(RichText::new(text).size(18.0).color(pal.heading));
    ui.add_space(4.0);
}

impl PaddApp {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else { return };
        let pal = Palette::new(ctx.global_style().visuals.dark_mode);
        let mut action: Option<Action> = None;
        let modal = |id: &str| {
            egui::Modal::new(egui::Id::unique(id)).backdrop_color(egui::Color32::from_black_alpha(120))
        };
        match dialog {
            Dialog::SetPassword {
                p1,
                p2,
                error,
                delete_plain,
                has_plain,
                ..
            } => {
                let r = modal("padd-password").show(ctx, |ui| {
                    ui.set_width(340.0);
                    title(ui, &pal, "Protect with a password");
                    ui.label(
                        RichText::new("Each file has its own password. A lost password cannot be recovered.")
                            .color(pal.weak),
                    );
                    ui.add_space(12.0);
                    let field = |s| {
                        egui::TextEdit::singleline(s)
                            .password(true)
                            .desired_width(f32::INFINITY)
                            .margin(vec2(10.0, 7.0))
                    };
                    let empty = p1.is_empty() && p2.is_empty();
                    let r1 = ui.add(field(p1).hint_text("Password"));
                    if empty && !r1.has_focus() {
                        r1.request_focus();
                    }
                    let r2 = ui.add(field(p2).hint_text("Repeat password"));
                    // strength hint
                    let n = p1.chars().count();
                    let (label, color) = match n {
                        0 => ("", pal.weak),
                        1..MIN_PASSWORD => ("Too short", ui.visuals().error_fg_color),
                        8..=11 => ("Fair", pal.accent),
                        _ => ("Strong", egui::Color32::from_rgb(0x5F, 0xC2, 0x8A)),
                    };
                    ui.label(RichText::new(label).size(12.0).color(color));
                    if *has_plain {
                        ui.checkbox(delete_plain, "Delete the unencrypted original after saving");
                    }
                    if let Some(e) = error {
                        ui.colored_label(ui.visuals().error_fg_color, e.as_str());
                    }
                    ui.add_space(10.0);
                    let enter = r2.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.horizontal(|ui| {
                        if primary_button(ui, "Encrypt & save").clicked() || enter {
                            if p1.chars().count() < MIN_PASSWORD {
                                *error = Some(format!("Use at least {MIN_PASSWORD} characters"));
                            } else if p1 != p2 {
                                *error = Some("Passwords do not match".into());
                            } else {
                                action = Some(Action::PasswordOk);
                            }
                        }
                        if secondary_button(ui, "Cancel").clicked() {
                            action = Some(Action::Cancel);
                        }
                    });
                });
                if r.should_close() && action.is_none() {
                    action = Some(Action::Cancel);
                }
            }
            Dialog::ConfirmClose(id) => {
                let id = *id;
                let name = self
                    .tab_index(id)
                    .map(|i| self.tabs[i].name())
                    .unwrap_or_default();
                let r = modal("padd-close").show(ctx, |ui| {
                    ui.set_width(340.0);
                    title(ui, &pal, &format!("Save changes to “{name}”?"));
                    ui.label(
                        RichText::new("Your changes will be lost if you don't save them.").color(pal.weak),
                    );
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, "Save").clicked() {
                            action = Some(Action::SaveTab(id));
                        }
                        if secondary_button(ui, "Don't save").clicked() {
                            action = Some(Action::CloseTab(id));
                        }
                        if secondary_button(ui, "Cancel").clicked() {
                            action = Some(Action::Cancel);
                        }
                    });
                });
                if r.should_close() && action.is_none() {
                    action = Some(Action::Cancel);
                }
            }
            Dialog::ConfirmQuit => {
                let names: Vec<String> = self.tabs.iter().filter(|t| t.dirty).map(Tab::name).collect();
                let r = modal("padd-quit").show(ctx, |ui| {
                    ui.set_width(340.0);
                    title(ui, &pal, "Quit with unsaved changes?");
                    for n in &names {
                        ui.label(RichText::new(format!("•  {n}")).color(pal.text));
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if secondary_button(ui, "Cancel").clicked() {
                            action = Some(Action::Cancel);
                        }
                        let quit = egui::Button::new(
                            RichText::new("Quit without saving").color(egui::Color32::WHITE),
                        )
                        .fill(ui.visuals().error_fg_color)
                        .corner_radius(7.0)
                        .min_size(vec2(96.0, 30.0));
                        if ui.add(quit).clicked() {
                            action = Some(Action::Quit);
                        }
                    });
                });
                if r.should_close() && action.is_none() {
                    action = Some(Action::Cancel);
                }
            }
            Dialog::Shortcuts => {
                let r = modal("padd-keys").show(ctx, |ui| {
                    ui.set_width(380.0);
                    title(ui, &pal, "Keyboard shortcuts");
                    ui.add_space(6.0);
                    let cmd = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
                    let rows = [
                        ("New / Open", format!("{cmd}N  ·  {cmd}O")),
                        ("Save / Save as", format!("{cmd}S  ·  {cmd}Shift+S")),
                        ("Close tab", format!("{cmd}W")),
                        ("Next / previous tab", "Ctrl+Tab  ·  Ctrl+Shift+Tab".into()),
                        ("Find", format!("{cmd}F   (Enter / Shift+Enter)")),
                        ("Text size", format!("{cmd}+  ·  {cmd}−  ·  {cmd}0")),
                        ("Text size (mouse)", format!("{cmd}wheel")),
                        ("Live preview on/off", format!("{cmd}E")),
                        ("Lock encrypted tab", format!("{cmd}L")),
                    ];
                    egui::Grid::new("keys")
                        .num_columns(2)
                        .spacing(vec2(24.0, 8.0))
                        .show(ui, |ui| {
                            for (what, keys) in rows {
                                ui.label(RichText::new(what).color(pal.text));
                                ui.label(RichText::new(keys).monospace().color(pal.accent));
                                ui.end_row();
                            }
                        });
                    ui.add_space(14.0);
                    if secondary_button(ui, "Close").clicked() {
                        action = Some(Action::Cancel);
                    }
                });
                if r.should_close() {
                    action = Some(Action::Cancel);
                }
            }
            Dialog::About => {
                let logo = self.logo.id();
                let r = modal("padd-about").show(ctx, |ui| {
                    ui.set_width(320.0);
                    ui.vertical_centered(|ui| {
                        ui.add(egui::Image::new((logo, vec2(72.0, 72.0))));
                        ui.add_space(6.0);
                        ui.label(RichText::new("Padd").size(24.0).color(pal.heading));
                        ui.label(
                            RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(pal.weak),
                        );
                        ui.add_space(8.0);
                        ui.label(RichText::new(env!("CARGO_PKG_DESCRIPTION")).color(pal.text));
                        ui.add_space(14.0);
                        if secondary_button(ui, "Close").clicked() {
                            action = Some(Action::Cancel);
                        }
                    });
                });
                if r.should_close() {
                    action = Some(Action::Cancel);
                }
            }
        }

        match action {
            None => {}
            Some(Action::Cancel) => self.dialog = None,
            Some(Action::Quit) => {
                self.dialog = None;
                self.quitting = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            Some(Action::CloseTab(id)) => {
                self.dialog = None;
                if let Some(i) = self.tab_index(id) {
                    self.close_tab(ctx, i, true);
                }
            }
            Some(Action::SaveTab(id)) => {
                self.dialog = None;
                if let Some(i) = self.tab_index(id) {
                    self.save(ctx, i, false);
                }
            }
            Some(Action::PasswordOk) => {
                let Some(Dialog::SetPassword {
                    tab,
                    p1,
                    mut p2,
                    delete_plain,
                    ..
                }) = self.dialog.take()
                else {
                    return;
                };
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
}
