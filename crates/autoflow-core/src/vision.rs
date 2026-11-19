//! Screen capture and image location (PyAutoGUI locateOnScreen equivalent).
//!
//! Matching uses zero-mean normalized cross-correlation (ZNCC, the same score
//! as OpenCV's TM_CCOEFF_NORMED that PyAutoGUI's `confidence` uses). The
//! search runs on a down-scaled pyramid level first, then refines candidates
//! at full resolution. Rows are split across threads.

use anyhow::{anyhow, bail, Context as _, Result};
use serde::Serialize;
use xcap::image::{imageops, GrayImage, RgbaImage};

use crate::actions::Region;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Match {
    /// Centre X in screen coordinates.
    pub x: i32,
    /// Centre Y in screen coordinates.
    pub y: i32,
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// Similarity 0..1.
    pub score: f64,
}

pub fn load_template(path: &std::path::Path) -> Result<GrayImage> {
    let img = xcap::image::open(path).with_context(|| format!("cannot open image {}", path.display()))?;
    Ok(img.to_luma8())
}

/// Capture the given region, or every monitor. Returns (image, screen-x, screen-y) per capture.
pub fn capture(region: Option<Region>) -> Result<Vec<(RgbaImage, i32, i32)>> {
    match region {
        Some(r) => {
            if r.width == 0 || r.height == 0 {
                bail!("region width and height must be > 0");
            }
            let m = xcap::Monitor::from_point(r.x, r.y).map_err(|e| anyhow!("no monitor at ({}, {}): {e}", r.x, r.y))?;
            let (mx, my) = (m.x()?, m.y()?);
            let (mw, mh) = (m.width()?, m.height()?);
            let rx = (r.x - mx).max(0) as u32;
            let ry = (r.y - my).max(0) as u32;
            let w = r.width.min(mw.saturating_sub(rx));
            let h = r.height.min(mh.saturating_sub(ry));
            Ok(vec![(m.capture_region(rx, ry, w, h)?, mx + rx as i32, my + ry as i32)])
        }
        None => {
            let mut out = vec![];
            for m in xcap::Monitor::all()? {
                out.push((m.capture_image()?, m.x()?, m.y()?));
            }
            Ok(out)
        }
    }
}

/// Save raw RGBA bytes as a PNG file.
pub fn save_rgba_png(path: &std::path::Path, w: u32, h: u32, rgba: Vec<u8>) -> Result<()> {
    let img = RgbaImage::from_raw(w, h, rgba).ok_or_else(|| anyhow!("image buffer has the wrong size"))?;
    img.save(path).with_context(|| format!("save {}", path.display()))?;
    Ok(())
}

/// Colour of one screen pixel.
pub fn pixel(x: i32, y: i32) -> Result<[u8; 3]> {
    let caps = capture(Some(Region { x, y, width: 1, height: 1 }))?;
    let (img, _, _) = caps.first().ok_or_else(|| anyhow!("capture failed"))?;
    let p = img.get_pixel(0, 0);
    Ok([p[0], p[1], p[2]])
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

pub fn parse_hex(s: &str) -> Result<[u8; 3]> {
    let t = s.trim().trim_start_matches('#');
    if t.len() != 6 {
        bail!("colour must look like #RRGGBB, got `{s}`");
    }
    let v = u32::from_str_radix(t, 16).map_err(|_| anyhow!("bad colour `{s}`"))?;
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Find `template` on screen. Returns matches with score >= confidence, best first.
pub fn locate(template: &GrayImage, region: Option<Region>, confidence: f64, all: bool) -> Result<Vec<Match>> {
    let mut found = vec![];
    for (img, ox, oy) in capture(region)? {
        let gray = xcap::image::DynamicImage::ImageRgba8(img).to_luma8();
        for (x, y, score) in search(&gray, template, confidence, all) {
            found.push(Match {
                x: ox + x as i32 + template.width() as i32 / 2,
                y: oy + y as i32 + template.height() as i32 / 2,
                left: ox + x as i32,
                top: oy + y as i32,
                width: template.width(),
                height: template.height(),
                score,
            });
        }
    }
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    if !all {
        found.truncate(1);
    }
    Ok(found)
}

/// Positions (top-left) where `t` matches `img` with ZNCC >= threshold.
pub fn search(img: &GrayImage, t: &GrayImage, threshold: f64, all: bool) -> Vec<(u32, u32, f64)> {
    let (iw, ih, tw, th) = (img.width(), img.height(), t.width(), t.height());
    if tw == 0 || th == 0 || tw > iw || th > ih {
        return vec![];
    }
    let scale = (tw.min(th) / 16).clamp(1, 4);
    let mut cands: Vec<(u32, u32, f64)> = if scale > 1 {
        let si = imageops::resize(img, iw / scale, ih / scale, imageops::FilterType::Triangle);
        let st = imageops::resize(t, (tw / scale).max(1), (th / scale).max(1), imageops::FilterType::Triangle);
        let coarse = Plane::new(&si).scan(&Tmpl::new(&st), (threshold - 0.2).max(0.3));
        let full = Plane::new(img);
        let tm = Tmpl::new(t);
        let r = scale as i64 + 1;
        let mut refined = vec![];
        for (cx, cy, _) in nms(coarse, st.width(), st.height(), 40) {
            let (bx, by) = (cx as i64 * scale as i64, cy as i64 * scale as i64);
            let mut best = (0u32, 0u32, f64::MIN);
            for y in (by - r).max(0)..=(by + r).min((ih - th) as i64) {
                for x in (bx - r).max(0)..=(bx + r).min((iw - tw) as i64) {
                    let s = full.score(&tm, x as u32, y as u32);
                    if s > best.2 {
                        best = (x as u32, y as u32, s);
                    }
                }
            }
            if best.2 >= threshold {
                refined.push(best);
            }
        }
        refined
    } else {
        Plane::new(img).scan(&Tmpl::new(t), threshold)
    };
    cands = nms(cands, tw, th, if all { 500 } else { 1 });
    cands
}

/// Non-maximum suppression: keep the best of overlapping hits.
fn nms(mut v: Vec<(u32, u32, f64)>, tw: u32, th: u32, limit: usize) -> Vec<(u32, u32, f64)> {
    v.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut keep: Vec<(u32, u32, f64)> = vec![];
    for c in v {
        if keep.len() >= limit {
            break;
        }
        let overlaps = keep.iter().any(|k| k.0.abs_diff(c.0) < tw / 2 + 1 && k.1.abs_diff(c.1) < th / 2 + 1);
        if !overlaps {
            keep.push(c);
        }
    }
    keep
}

/// Image with integral tables for O(1) window mean/variance.
struct Plane {
    w: usize,
    h: usize,
    px: Vec<f32>,
    sum: Vec<f64>,
    sq: Vec<f64>,
}

struct Tmpl {
    w: usize,
    h: usize,
    /// Zero-mean template values.
    z: Vec<f32>,
    norm: f64,
    raw: Vec<f32>,
}

impl Tmpl {
    fn new(t: &GrayImage) -> Self {
        let raw: Vec<f32> = t.pixels().map(|p| p[0] as f32).collect();
        let mean = raw.iter().map(|&v| v as f64).sum::<f64>() / raw.len() as f64;
        let z: Vec<f32> = raw.iter().map(|&v| (v as f64 - mean) as f32).collect();
        let norm = z.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>().sqrt();
        Self { w: t.width() as usize, h: t.height() as usize, z, norm, raw }
    }
}

impl Plane {
    fn new(img: &GrayImage) -> Self {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let px: Vec<f32> = img.pixels().map(|p| p[0] as f32).collect();
        let mut sum = vec![0f64; (w + 1) * (h + 1)];
        let mut sq = vec![0f64; (w + 1) * (h + 1)];
        for y in 0..h {
            let (mut rs, mut rq) = (0f64, 0f64);
            for x in 0..w {
                let v = px[y * w + x] as f64;
                rs += v;
                rq += v * v;
                sum[(y + 1) * (w + 1) + x + 1] = sum[y * (w + 1) + x + 1] + rs;
                sq[(y + 1) * (w + 1) + x + 1] = sq[y * (w + 1) + x + 1] + rq;
            }
        }
        Self { w, h, px, sum, sq }
    }

    fn rect(&self, t: &[f64], x: usize, y: usize, w: usize, h: usize) -> f64 {
        let s = self.w + 1;
        t[(y + h) * s + x + w] - t[y * s + x + w] - t[(y + h) * s + x] + t[y * s + x]
    }

    /// ZNCC at top-left (x, y). Flat templates fall back to mean absolute difference.
    fn score(&self, t: &Tmpl, x: u32, y: u32) -> f64 {
        let (x, y) = (x as usize, y as usize);
        let n = (t.w * t.h) as f64;
        if t.norm < 1e-6 {
            let mut diff = 0f64;
            for ty in 0..t.h {
                let row = &self.px[(y + ty) * self.w + x..(y + ty) * self.w + x + t.w];
                for (a, b) in row.iter().zip(&t.raw[ty * t.w..(ty + 1) * t.w]) {
                    diff += (a - b).abs() as f64;
                }
            }
            return 1.0 - diff / (n * 255.0);
        }
        let s = self.rect(&self.sum, x, y, t.w, t.h);
        let q = self.rect(&self.sq, x, y, t.w, t.h);
        let var = q - s * s / n;
        if var < 1e-6 {
            return 0.0;
        }
        let mut cross = 0f32;
        for ty in 0..t.h {
            let row = &self.px[(y + ty) * self.w + x..(y + ty) * self.w + x + t.w];
            let trow = &t.z[ty * t.w..(ty + 1) * t.w];
            cross += row.iter().zip(trow).map(|(a, b)| a * b).sum::<f32>();
        }
        cross as f64 / (var.sqrt() * t.norm)
    }

    /// Score every position; return those >= threshold.
    fn scan(&self, t: &Tmpl, threshold: f64) -> Vec<(u32, u32, f64)> {
        if t.w > self.w || t.h > self.h {
            return vec![];
        }
        let rows = self.h - t.h + 1;
        let cols = self.w - t.w + 1;
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(rows);
        let chunk = rows.div_ceil(threads);
        std::thread::scope(|sc| {
            let handles: Vec<_> = (0..threads)
                .map(|i| {
                    sc.spawn(move || {
                        let mut out = vec![];
                        for y in i * chunk..((i + 1) * chunk).min(rows) {
                            for x in 0..cols {
                                let s = self.score(t, x as u32, y as u32);
                                if s >= threshold {
                                    out.push((x as u32, y as u32, s));
                                }
                            }
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcap::image::Luma;

    fn noise(w: u32, h: u32, seed: u32) -> GrayImage {
        let mut s = seed;
        GrayImage::from_fn(w, h, |_, _| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            Luma([(s >> 16) as u8])
        })
    }

    #[test]
    fn finds_exact_and_offset_template() {
        let img = noise(400, 300, 7);
        for (tx, ty, tw, th) in [(123, 77, 20, 14), (250, 180, 64, 48)] {
            let t = imageops::crop_imm(&img, tx, ty, tw, th).to_image();
            let hits = search(&img, &t, 0.9, false);
            assert_eq!(hits.len(), 1, "{tw}x{th}");
            assert_eq!((hits[0].0, hits[0].1), (tx, ty), "{tw}x{th}");
            assert!(hits[0].2 > 0.99);
        }
    }

    #[test]
    fn rejects_missing_template() {
        let img = noise(300, 200, 1);
        let other = noise(30, 30, 99);
        assert!(search(&img, &other, 0.9, false).is_empty());
    }

    #[test]
    fn colours() {
        assert_eq!(parse_hex("#FF8000").unwrap(), [255, 128, 0]);
        assert_eq!(hex([1, 2, 255]), "#0102FF");
    }
}
