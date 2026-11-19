//! Text recognition with the OCR engine built into Windows 10/11
//! (Windows.Media.Ocr): no extra download, works offline. The languages
//! available are the Windows display/input languages installed on the PC
//! (Settings > Time & language > Language; add a language with "Optical character recognition").

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use xcap::image::{imageops, RgbaImage};

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    /// Bounding box in the pixels of the recognised image.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, Default)]
pub struct OcrOutput {
    pub text: String,
    pub lines: Vec<Line>,
}

/// A found piece of text, as a box in the recognised image's pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Hit {
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Recognise text in an image. `scale` enlarges it first (2 helps with small
/// screen fonts); word boxes are given in the original image's pixels.
/// `language` is a tag such as "en-US" or "hi" (empty = the user's languages).
#[cfg(windows)]
pub fn recognize(img: &RgbaImage, language: &str, scale: f32) -> Result<OcrOutput> {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;

    // WinRT needs the thread initialised; "already initialised" results are fine.
    unsafe {
        let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
    }

    let max = OcrEngine::MaxImageDimension().unwrap_or(10_000) as f32;
    let (w0, h0) = (img.width().max(1) as f32, img.height().max(1) as f32);
    let scale = scale.clamp(1.0, 4.0).min(max / w0).min(max / h0).max(0.05);
    let scaled;
    let img = if (scale - 1.0).abs() > 0.01 {
        scaled = imageops::resize(img, (w0 * scale) as u32, (h0 * scale) as u32, imageops::FilterType::CatmullRom);
        &scaled
    } else {
        img
    };

    let engine = if language.trim().is_empty() {
        OcrEngine::TryCreateFromUserProfileLanguages()
    } else {
        let lang = Language::CreateLanguage(&HSTRING::from(language.trim()))?;
        if !OcrEngine::IsLanguageSupported(&lang).unwrap_or(false) {
            bail!("OCR language `{language}` is not installed. Available: {}. Add it in Settings > Time & language > Language", languages().join(", "));
        }
        OcrEngine::TryCreateFromLanguage(&lang)
    }
    .map_err(|e| anyhow!("Windows OCR is not available: {e}"))?;

    let mut bgra = img.as_raw().clone();
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bgra)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, img.width() as i32, img.height() as i32)?;
    let result = engine.RecognizeAsync(&bitmap)?.join().map_err(|e| anyhow!("OCR failed: {e}"))?;

    let mut out = OcrOutput::default();
    for line in result.Lines()? {
        let mut words = vec![];
        for w in line.Words()? {
            let r = w.BoundingRect()?;
            words.push(Word { text: w.Text()?.to_string(), x: r.X / scale, y: r.Y / scale, w: r.Width / scale, h: r.Height / scale });
        }
        out.lines.push(Line { text: line.Text()?.to_string(), words });
    }
    out.text = out.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
    Ok(out)
}

#[cfg(not(windows))]
pub fn recognize(_img: &RgbaImage, _language: &str, _scale: f32) -> Result<OcrOutput> {
    bail!("OCR is only available on Windows")
}

/// OCR languages installed on this PC (tags such as en-US).
#[cfg(windows)]
pub fn languages() -> Vec<String> {
    windows::Media::Ocr::OcrEngine::AvailableRecognizerLanguages()
        .map(|v| v.into_iter().filter_map(|l| l.LanguageTag().ok()).map(|t| t.to_string()).collect())
        .unwrap_or_default()
}

#[cfg(not(windows))]
pub fn languages() -> Vec<String> {
    vec![]
}

fn norm(s: &str, ignore_case: bool) -> String {
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if ignore_case {
        s.to_lowercase()
    } else {
        s
    }
}

/// Every place `needle` appears (it may span several words of a line).
pub fn find(out: &OcrOutput, needle: &str, ignore_case: bool) -> Vec<Hit> {
    let needle = norm(needle, ignore_case);
    if needle.is_empty() {
        return vec![];
    }
    let mut hits = vec![];
    for line in &out.lines {
        // Rebuild the line from its words, remembering where each word starts.
        let mut text = String::new();
        let mut spans = vec![];
        for (i, w) in line.words.iter().enumerate() {
            if i > 0 {
                text.push(' ');
            }
            let start = text.len();
            text.push_str(&norm(&w.text, ignore_case));
            spans.push((start, text.len()));
        }
        let mut from = 0;
        while let Some(pos) = text[from..].find(&needle) {
            let (s, e) = (from + pos, from + pos + needle.len());
            let covered: Vec<&Word> = line.words.iter().zip(&spans).filter(|(_, (a, b))| *a < e && *b > s).map(|(w, _)| w).collect();
            if let (Some(first), Some(last)) = (covered.first(), covered.last()) {
                // Narrow the box to the matched characters for a partial first/last word.
                let (fs, fe) = spans[line.words.iter().position(|w| std::ptr::eq(w, *first)).unwrap()];
                let (ls, le) = spans[line.words.iter().position(|w| std::ptr::eq(w, *last)).unwrap()];
                let frac = |a: usize, b: usize, at: usize| if b > a { (at.clamp(a, b) - a) as f32 / (b - a) as f32 } else { 0.0 };
                let x0 = first.x + first.w * frac(fs, fe, s);
                let x1 = last.x + last.w * frac(ls, le, e);
                let y0 = covered.iter().map(|w| w.y).fold(f32::MAX, f32::min);
                let y1 = covered.iter().map(|w| w.y + w.h).fold(f32::MIN, f32::max);
                hits.push(Hit { x: x0, y: y0, w: (x1 - x0).max(1.0), h: (y1 - y0).max(1.0) });
            }
            from = e;
        }
    }
    hits
}

/// The words as JSON records with screen coordinates (offset added).
pub fn words_json(out: &OcrOutput, ox: i32, oy: i32) -> Value {
    Value::Array(
        out.lines
            .iter()
            .flat_map(|l| &l.words)
            .map(|w| {
                json!({
                    "text": w.text,
                    "x": ox + w.x.round() as i32, "y": oy + w.y.round() as i32,
                    "width": w.w.round() as i32, "height": w.h.round() as i32,
                    "center_x": ox + (w.x + w.w / 2.0).round() as i32, "center_y": oy + (w.y + w.h / 2.0).round() as i32,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, x: f32) -> Word {
        Word { text: text.into(), x, y: 10.0, w: text.len() as f32 * 10.0, h: 20.0 }
    }

    #[test]
    fn find_spans_words() {
        let out = OcrOutput {
            text: String::new(),
            lines: vec![
                Line { text: "File Save As".into(), words: vec![w("File", 0.0), w("Save", 50.0), w("As", 100.0)] },
                Line { text: "Saved".into(), words: vec![w("Saved", 0.0)] },
            ],
        };
        let h = find(&out, "save as", true);
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].x, h[0].w), (50.0, 70.0));
        assert_eq!(find(&out, "Save", false).len(), 2, "also inside \"Saved\"");
        let part = find(&out, "aved", true);
        assert_eq!(part.len(), 1);
        assert!((part[0].x - 10.0).abs() < 0.01);
        assert!(find(&out, "save", false).is_empty());
    }

    /// Renders nothing but checks the Windows OCR engine can be created and runs.
    #[cfg(windows)]
    #[test]
    fn windows_ocr_runs() {
        let img = RgbaImage::from_pixel(200, 60, xcap::image::Rgba([255, 255, 255, 255]));
        match recognize(&img, "", 1.0) {
            Ok(out) => assert!(out.text.trim().is_empty()),
            Err(e) => eprintln!("OCR unavailable on this machine: {e}"),
        }
    }
}
