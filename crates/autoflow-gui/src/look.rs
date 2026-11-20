//! How Studio looks: font, text size and colours, chosen in the View menu and remembered in
//! %APPDATA%\AutoFlowMate\studio.json.

use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    /// 1.0 = normal; the whole interface is scaled.
    pub text_scale: f32,
    /// Use Windows' Segoe UI instead of egui's built-in (thin) font.
    pub system_font: bool,
    /// Keep only one action category open in the palette.
    pub one_category: bool,
    /// Language file code (en = built-in English).
    pub language: String,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self { text_scale: 1.0, system_font: true, one_category: true, language: "en".into() }
    }
}

pub const SCALES: [(f32, &str); 5] = [(0.9, "Small (90%)"), (1.0, "Normal (100%)"), (1.15, "Large (115%)"), (1.3, "Larger (130%)"), (1.6, "Largest (160%)")];

impl UiPrefs {
    fn path() -> PathBuf {
        let base = std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        base.join("AutoFlowMate").join("studio.json")
    }

    pub fn load() -> UiPrefs {
        std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = Self::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, serde_json::to_string_pretty(self).unwrap_or_default());
    }
}

/// Fonts: Segoe UI (regular weight, made for screens) first, egui's fonts after it for symbols
/// and emoji icons Segoe UI lacks.
pub fn apply_fonts(ctx: &egui::Context, prefs: &UiPrefs) {
    let mut fonts = egui::FontDefinitions::default();
    if prefs.system_font {
        let dir = std::env::var("WINDIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(r"C:\Windows")).join("Fonts");
        for (name, file, family) in [("segoe", "segoeui.ttf", FontFamily::Proportional), ("consolas", "consola.ttf", FontFamily::Monospace)] {
            if let Ok(bytes) = std::fs::read(dir.join(file)) {
                fonts.font_data.insert(name.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                if let Some(list) = fonts.families.get_mut(&family) {
                    list.insert(0, name.into());
                }
            }
        }
    }
    ctx.set_fonts(fonts);
}

/// Text sizes, near-black text in the light theme, and the zoom level.
pub fn apply_style(ctx: &egui::Context, prefs: &UiPrefs) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.style_mut_of(theme, |s| {
            s.text_styles = [
                (TextStyle::Small, FontId::proportional(12.0)),
                (TextStyle::Body, FontId::proportional(14.5)),
                (TextStyle::Button, FontId::proportional(14.5)),
                (TextStyle::Heading, FontId::proportional(21.0)),
                (TextStyle::Monospace, FontId::monospace(13.5)),
            ]
            .into();
            s.spacing.interact_size.y = s.spacing.interact_size.y.max(22.0);
            if theme == egui::Theme::Light {
                let v = &mut s.visuals;
                v.widgets.noninteractive.fg_stroke.color = Color32::from_gray(20);
                v.widgets.inactive.fg_stroke.color = Color32::from_gray(15);
                v.widgets.hovered.fg_stroke.color = Color32::BLACK;
                v.widgets.active.fg_stroke.color = Color32::BLACK;
                v.weak_text_color = Some(Color32::from_gray(90));
            } else {
                s.visuals.widgets.noninteractive.fg_stroke.color = Color32::from_gray(225);
                s.visuals.widgets.inactive.fg_stroke.color = Color32::from_gray(235);
                s.visuals.weak_text_color = Some(Color32::from_gray(165));
            }
        });
    }
    ctx.set_zoom_factor(prefs.text_scale.clamp(0.75, 2.0));
}

// ------------------------------------------------------------------ status colours

static DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called once per frame with the current theme, so status colours stay readable.
pub fn set_dark(dark: bool) {
    DARK.store(dark, std::sync::atomic::Ordering::Relaxed);
}

fn dark() -> bool {
    DARK.load(std::sync::atomic::Ordering::Relaxed)
}

/// Success / "yes" / stored: dark green on light backgrounds, light green on dark.
pub fn ok() -> Color32 {
    if dark() {
        Color32::from_rgb(120, 215, 130)
    } else {
        Color32::from_rgb(0, 110, 40)
    }
}

/// Failure / "no" / stop / delete.
pub fn err() -> Color32 {
    if dark() {
        Color32::from_rgb(255, 125, 125)
    } else {
        Color32::from_rgb(185, 20, 20)
    }
}

/// Warnings / "not stored" / paused.
pub fn warn() -> Color32 {
    if dark() {
        Color32::from_rgb(245, 185, 85)
    } else {
        Color32::from_rgb(160, 85, 0)
    }
}
