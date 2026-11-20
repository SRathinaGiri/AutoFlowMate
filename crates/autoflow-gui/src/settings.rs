//! File > Settings: the provider lists the `provider` fields offer, editable by the user and
//! saved to %APPDATA%\AutoFlowMate\presets.json (used by flows run from Studio, the CLI and
//! scheduled tasks on this PC).

use autoflow_core::presets::{AiApi, AiPreset, MailPreset, Presets};
use eframe::egui::{self, Color32, RichText};

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Ai,
    Mail,
}

pub struct SettingsUi {
    presets: Presets,
    tab: Tab,
    message: Option<(bool, String)>,
    /// Comma-separated model lists being edited, per AI row.
    models_buf: Vec<String>,
}

impl SettingsUi {
    pub fn new() -> Self {
        let presets = Presets::load();
        let models_buf = presets.ai.iter().map(|p| p.models.join(", ")).collect();
        Self { presets, tab: Tab::Ai, message: None, models_buf }
    }
}

/// Show the window. Returns true when the presets were saved (so the editor reloads them).
pub fn show(ctx: &egui::Context, state: &mut Option<SettingsUi>) -> bool {
    let Some(s) = state.as_mut() else { return false };
    let mut open = true;
    let mut saved = false;
    let mut close = false;
    egui::Window::new("⚙ Settings").open(&mut open).default_width(1180.0).default_height(460.0).resizable(true).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut s.tab, Tab::Ai, RichText::new("🤖 AI providers").strong());
            ui.selectable_value(&mut s.tab, Tab::Mail, RichText::new("✉ E-mail providers").strong());
        });
        ui.weak(format!(
            "The names here are what a step's Provider field offers. Saved in {} for this PC (flows that use a provider you add need it on the PC that runs them).",
            Presets::path().display()
        ));
        ui.separator();
        egui::ScrollArea::both().max_height(330.0).auto_shrink([false, true]).show(ui, |ui| match s.tab {
            Tab::Ai => ai_table(ui, s),
            Tab::Mail => mail_table(ui, s),
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("➕ Add").clicked() {
                match s.tab {
                    Tab::Ai => {
                        s.presets.ai.push(AiPreset { name: "new_provider".into(), base_url: "https://".into(), needs_key: true, ..AiPreset::default() });
                        s.models_buf.push(String::new());
                    }
                    Tab::Mail => s.presets.mail.push(MailPreset {
                        name: "my_server".into(),
                        smtp_port: 587,
                        smtp_security: "starttls".into(),
                        imap_port: 993,
                        ..MailPreset::default()
                    }),
                }
            }
            if ui.button("↺ Reset to built-in").on_hover_text("Replace this tab's list with the providers AutoFlowMate comes with").clicked() {
                let b = Presets::builtin();
                match s.tab {
                    Tab::Ai => {
                        s.presets.ai = b.ai;
                        s.models_buf = s.presets.ai.iter().map(|p| p.models.join(", ")).collect();
                    }
                    Tab::Mail => s.presets.mail = b.mail,
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if ui.button(RichText::new("💾 Save").strong()).clicked() {
                    for (p, m) in s.presets.ai.iter_mut().zip(&s.models_buf) {
                        p.models = m.split(',').map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect();
                        p.name = p.name.trim().to_string();
                    }
                    for p in &mut s.presets.mail {
                        p.name = p.name.trim().to_string();
                    }
                    match s.presets.save() {
                        Ok(path) => {
                            s.message = Some((true, format!("saved to {}", path.display())));
                            saved = true;
                        }
                        Err(e) => s.message = Some((false, format!("{e:#}"))),
                    }
                }
            });
        });
        if let Some((ok, m)) = &s.message {
            ui.colored_label(if *ok { Color32::from_rgb(70, 170, 90) } else { Color32::from_rgb(220, 90, 90) }, m);
        }
    });
    if !open || close {
        *state = None;
    }
    saved
}

fn ai_table(ui: &mut egui::Ui, s: &mut SettingsUi) {
    let mut remove = None;
    egui::Grid::new("ai_presets").striped(true).num_columns(8).spacing([8.0, 6.0]).show(ui, |ui| {
        for h in ["Name", "API", "Server URL", "Default model", "Model list (comma-separated)", "Key", "Key name", ""] {
            ui.label(RichText::new(h).strong());
        }
        ui.end_row();
        for (i, p) in s.presets.ai.iter_mut().enumerate() {
            ui.add_sized([110.0, 22.0], egui::TextEdit::singleline(&mut p.name));
            egui::ComboBox::from_id_salt(("api", i)).width(130.0).selected_text(p.api.label()).show_ui(ui, |ui| {
                for a in AiApi::ALL {
                    ui.selectable_value(&mut p.api, a, a.label());
                }
            });
            ui.add_sized([230.0, 22.0], egui::TextEdit::singleline(&mut p.base_url).hint_text("http://localhost:1234/v1"));
            ui.add_sized([130.0, 22.0], egui::TextEdit::singleline(&mut p.default_model).hint_text("(loaded model)"));
            ui.add_sized([180.0, 22.0], egui::TextEdit::singleline(&mut s.models_buf[i]).hint_text("model-a, model-b"));
            ui.checkbox(&mut p.needs_key, "").on_hover_text("The service needs an API key");
            ui.add_enabled_ui(p.needs_key, |ui| {
                ui.add_sized([90.0, 22.0], egui::TextEdit::singleline(&mut p.key_secret).hint_text("secret name"))
                    .on_hover_text("Credential Manager name used when a step leaves the key empty (autoflow secret set NAME)");
            });
            if ui.small_button("🗑").on_hover_text("Delete this provider").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        s.presets.ai.remove(i);
        s.models_buf.remove(i);
    }
    ui.add_space(6.0);
    ui.weak("OpenAI-compatible fits LM Studio, Ollama, vLLM, LocalAI, Groq, OpenRouter, Together and most others: give the server URL ending in /v1.");
}

fn mail_table(ui: &mut egui::Ui, s: &mut SettingsUi) {
    let mut remove = None;
    egui::Grid::new("mail_presets").striped(true).num_columns(7).spacing([8.0, 6.0]).show(ui, |ui| {
        for h in ["Name", "SMTP server (send)", "Port", "Security", "IMAP server (read)", "Port", ""] {
            ui.label(RichText::new(h).strong());
        }
        ui.end_row();
        for (i, p) in s.presets.mail.iter_mut().enumerate() {
            ui.add_sized([110.0, 22.0], egui::TextEdit::singleline(&mut p.name));
            ui.add_sized([190.0, 22.0], egui::TextEdit::singleline(&mut p.smtp_host).hint_text("smtp.example.com"));
            ui.add(egui::DragValue::new(&mut p.smtp_port).range(0..=65535));
            egui::ComboBox::from_id_salt(("sec", i)).width(90.0).selected_text(p.smtp_security.clone()).show_ui(ui, |ui| {
                for o in ["starttls", "tls", "none"] {
                    ui.selectable_value(&mut p.smtp_security, o.to_string(), o);
                }
            });
            ui.add_sized([190.0, 22.0], egui::TextEdit::singleline(&mut p.imap_host).hint_text("imap.example.com"));
            ui.add(egui::DragValue::new(&mut p.imap_port).range(0..=65535));
            if ui.small_button("🗑").on_hover_text("Delete this provider").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        s.presets.mail.remove(i);
    }
    ui.add_space(6.0);
    ui.weak("Security: starttls (port 587) or tls (port 465) for sending. Reading uses IMAP over TLS (port 993).");
}
