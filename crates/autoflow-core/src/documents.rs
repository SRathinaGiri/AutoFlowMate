//! Documents: zip archives, Word (.docx) read / fill a template / write, and tables in PDFs.
//!
//! Word files are zip archives of XML. Reading walks `word/document.xml`; filling a template
//! replaces `{{name}}` placeholders even when Word has split them across formatting runs;
//! writing builds a minimal valid .docx by hand, so no Word installation is needed.

use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{json, Map, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

// ----------------------------------------------------------------------- zip

/// Files to put in an archive: (path on disk, name inside the zip).
fn zip_entries(sources: &[PathBuf]) -> Result<Vec<(PathBuf, String)>> {
    let mut out = vec![];
    for src in sources {
        if src.is_dir() {
            let parent = src.parent().unwrap_or(Path::new(""));
            let pattern = format!("{}/**/*", glob::Pattern::escape(&src.display().to_string()));
            for p in glob::glob(&pattern).map_err(|e| anyhow!("bad folder name: {e}"))?.flatten() {
                if p.is_file() {
                    let rel = p.strip_prefix(parent).unwrap_or(&p);
                    out.push((p.clone(), rel.to_string_lossy().replace('\\', "/")));
                }
            }
        } else if src.is_file() {
            out.push((src.clone(), src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        } else {
            bail!("not found: {}", src.display());
        }
    }
    Ok(out)
}

/// Create (or replace) a zip of files and folders. Returns the number of files added.
pub fn zip_create(zip_path: &Path, sources: &[PathBuf]) -> Result<usize> {
    let entries = zip_entries(sources)?;
    if entries.is_empty() {
        bail!("nothing to zip");
    }
    if let Some(dir) = zip_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::File::create(zip_path).with_context(|| format!("cannot create {}", zip_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut seen = std::collections::HashSet::new();
    for (p, name) in &entries {
        if !seen.insert(name.to_lowercase()) {
            bail!("two files would have the same name in the zip: {name}");
        }
        zip.start_file(name.as_str(), opts)?;
        let mut f = std::fs::File::open(p).with_context(|| format!("cannot read {}", p.display()))?;
        std::io::copy(&mut f, &mut zip)?;
    }
    zip.finish()?;
    Ok(entries.len())
}

/// Extract a zip into `dest`. `pattern` (e.g. "*.pdf") limits which files; existing files are
/// kept unless `overwrite`. Returns the extracted paths. Entries that would escape `dest` are refused.
pub fn zip_extract(zip_path: &Path, dest: &Path, pattern: &str, overwrite: bool) -> Result<Vec<String>> {
    let file = std::fs::File::open(zip_path).with_context(|| format!("cannot open {}", zip_path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| anyhow!("{} is not a zip file: {e}", zip_path.display()))?;
    let pat = if pattern.trim().is_empty() { None } else { Some(glob::Pattern::new(&pattern.trim().to_lowercase()).map_err(|e| anyhow!("bad pattern: {e}"))?) };
    let mut out = vec![];
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            bail!("unsafe path inside the zip: {}", entry.name().map(|n| n.into_owned()).unwrap_or_default());
        };
        if entry.is_dir() {
            continue;
        }
        let name = rel.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if pat.as_ref().is_some_and(|p| !p.matches(&name)) {
            continue;
        }
        let target = dest.join(&rel);
        if target.exists() && !overwrite {
            bail!("{} already exists (set overwrite to replace it)", target.display());
        }
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::File::create(&target).with_context(|| format!("cannot write {}", target.display()))?;
        std::io::copy(&mut entry, &mut f)?;
        out.push(target.display().to_string());
    }
    Ok(out)
}

// ---------------------------------------------------------------------- Word

fn read_zip_text(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<Option<String>> {
    match zip.by_name(name) {
        Ok(mut f) => {
            let mut s = String::new();
            f.read_to_string(&mut s)?;
            Ok(Some(s))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Text (paragraphs separated by newlines) and tables (each a list of rows of cell texts).
pub fn word_read(path: &Path) -> Result<(String, Vec<Vec<Vec<String>>>)> {
    use quick_xml::events::Event as X;
    let file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|_| anyhow!("{} is not a .docx file (old .doc files are not supported: save as .docx)", path.display()))?;
    let xml = read_zip_text(&mut zip, "word/document.xml")?.ok_or_else(|| anyhow!("{} has no word/document.xml", path.display()))?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut text = String::new();
    let mut para = String::new();
    let mut in_text = false;
    // Table stack (tables can be nested): rows of cells.
    let mut tables: Vec<Vec<Vec<String>>> = vec![];
    let mut open: Vec<(Vec<Vec<String>>, Vec<String>, String)> = vec![];
    loop {
        match reader.read_event().map_err(|e| anyhow!("bad document XML: {e}"))? {
            X::Start(e) => match e.local_name().as_ref() {
                "t" => in_text = true,
                "tbl" => open.push((vec![], vec![], String::new())),
                "tr" => {
                    if let Some(t) = open.last_mut() {
                        t.1.clear();
                    }
                }
                "tc" => {
                    if let Some(t) = open.last_mut() {
                        t.2.clear();
                    }
                }
                _ => {}
            },
            X::Empty(e) => match e.local_name().as_ref() {
                "tab" => para.push('\t'),
                "br" | "cr" => para.push('\n'),
                _ => {}
            },
            X::Text(t) if in_text => para.push_str(&t.xml10_content()),
            X::GeneralRef(r) if in_text => {
                let name = r.xml10_content();
                para.push_str(match name.as_ref() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                });
            }
            X::End(e) => match e.local_name().as_ref() {
                "t" => in_text = false,
                "p" => {
                    let p = std::mem::take(&mut para);
                    if let Some(t) = open.last_mut() {
                        if !t.2.is_empty() {
                            t.2.push('\n');
                        }
                        t.2.push_str(&p);
                    }
                    text.push_str(&p);
                    text.push('\n');
                }
                "tc" => {
                    if let Some(t) = open.last_mut() {
                        let cell = std::mem::take(&mut t.2);
                        t.1.push(cell);
                    }
                }
                "tr" => {
                    if let Some(t) = open.last_mut() {
                        let row = std::mem::take(&mut t.1);
                        t.0.push(row);
                    }
                }
                "tbl" => {
                    if let Some((rows, _, _)) = open.pop() {
                        tables.push(rows);
                    }
                }
                _ => {}
            },
            X::Eof => break,
            _ => {}
        }
    }
    Ok((text.trim_end().to_string(), tables))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Replace `{{name}}` in every paragraph of one XML part. Word often splits a placeholder over
/// several runs (`{{na` + `me}}`); such a paragraph's text is joined into its first run, which
/// keeps that run's formatting. Returns the names that had no value.
fn fill_part(xml: &str, values: &Map<String, Value>, missing: &mut Vec<String>) -> String {
    let para_re = regex::Regex::new(r"(?s)<w:p[ >].*?</w:p>").unwrap();
    let text_re = regex::Regex::new(r"(?s)(<w:t(?:\s[^>]*)?>)(.*?)(</w:t>)").unwrap();
    let ph_re = regex::Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").unwrap();
    para_re
        .replace_all(xml, |caps: &regex::Captures| {
            let para = &caps[0];
            let joined: String = text_re.captures_iter(para).map(|c| xml_unescape(&c[2])).collect();
            if !ph_re.is_match(&joined) {
                return para.to_string();
            }
            let filled = ph_re.replace_all(&joined, |c: &regex::Captures| {
                let key = c[1].trim();
                match values.get(key).or_else(|| values.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)) {
                    Some(v) => crate::vars::to_text(v),
                    None => {
                        if !missing.iter().any(|m| m == key) {
                            missing.push(key.to_string());
                        }
                        c[0].to_string()
                    }
                }
            });
            // Line breaks in a value become Word line breaks.
            let escaped = xml_escape(&filled).replace('\n', "</w:t><w:br/><w:t xml:space=\"preserve\">");
            let mut first = true;
            text_re
                .replace_all(para, |c: &regex::Captures| {
                    if first {
                        first = false;
                        format!("<w:t xml:space=\"preserve\">{escaped}{}", &c[3])
                    } else {
                        format!("{}{}", &c[1], &c[3])
                    }
                })
                .into_owned()
        })
        .into_owned()
}

/// Copy a .docx template to `out`, replacing `{{name}}` with values (document, headers, footers).
/// Returns the placeholder names that had no value (left as they are).
pub fn word_fill(template: &Path, out: &Path, values: &Map<String, Value>) -> Result<Vec<String>> {
    let file = std::fs::File::open(template).with_context(|| format!("cannot open {}", template.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| anyhow!("{} is not a .docx file", template.display()))?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = out.with_extension("docx.tmp");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&tmp).with_context(|| format!("cannot write {} (is it open in Word?)", out.display()))?);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut missing = vec![];
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().map(|n| n.into_owned()).map_err(|e| anyhow!("bad entry name: {e}"))?;
        let mut data = vec![];
        entry.read_to_end(&mut data)?;
        let is_text_part = name == "word/document.xml" || (name.starts_with("word/header") || name.starts_with("word/footer")) && name.ends_with(".xml");
        w.start_file(name.as_str(), opts)?;
        if is_text_part {
            let xml = String::from_utf8_lossy(&data);
            w.write_all(fill_part(&xml, values, &mut missing).as_bytes())?;
        } else {
            w.write_all(&data)?;
        }
    }
    w.finish()?;
    std::fs::rename(&tmp, out)
        .or_else(|_| std::fs::copy(&tmp, out).map(|_| ()).and_then(|_| std::fs::remove_file(&tmp)))
        .with_context(|| format!("cannot replace {} (is it open in Word?)", out.display()))?;
    Ok(missing)
}

/// Write a new .docx: optional title, paragraphs (one per line of `text`) and an optional table.
pub fn word_write(path: &Path, title: &str, text: &str, table: Option<&crate::data::Table>) -> Result<()> {
    let run = |t: &str| format!("<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>", xml_escape(t));
    let mut body = String::new();
    if !title.trim().is_empty() {
        body.push_str(&format!("<w:p><w:pPr><w:pStyle w:val=\"Title\"/></w:pPr>{}</w:p>", run(title.trim())));
    }
    for line in text.lines() {
        body.push_str(&format!("<w:p>{}</w:p>", if line.is_empty() { String::new() } else { run(line) }));
    }
    if let Some(t) = table {
        let cell = |v: &str, bold: bool| {
            let props = if bold { "<w:rPr><w:b/></w:rPr>" } else { "" };
            format!("<w:tc><w:p><w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r></w:p></w:tc>", xml_escape(v))
        };
        let border = |side: &str| format!("<w:{side} w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/>");
        let borders: String = ["top", "left", "bottom", "right", "insideH", "insideV"].iter().map(|s| border(s)).collect();
        body.push_str(&format!("<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders>{borders}</w:tblBorders></w:tblPr>"));
        if !t.headers.is_empty() {
            body.push_str("<w:tr>");
            for h in &t.headers {
                body.push_str(&cell(h, true));
            }
            body.push_str("</w:tr>");
        }
        for r in &t.rows {
            body.push_str("<w:tr>");
            for v in r {
                body.push_str(&cell(&crate::vars::to_text(v), false));
            }
            body.push_str("</w:tr>");
        }
        body.push_str("</w:tbl><w:p/>");
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}\
<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\" w:header=\"709\" w:footer=\"709\" w:gutter=\"0\"/></w:sectPr>\
</w:body></w:document>"
    );
    const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
</Types>";
    const RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";
    const DOC_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
</Relationships>";
    const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:cs=\"Calibri\"/><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"120\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:spacing w:after=\"240\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"40\"/><w:color w:val=\"1F3864\"/></w:rPr></w:style>\
</w:styles>";
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut w = zip::ZipWriter::new(std::fs::File::create(path).with_context(|| format!("cannot write {} (is it open in Word?)", path.display()))?);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", RELS),
        ("word/_rels/document.xml.rels", DOC_RELS),
        ("word/styles.xml", STYLES),
        ("word/document.xml", document.as_str()),
    ] {
        w.start_file(name, opts)?;
        w.write_all(data.as_bytes())?;
    }
    w.finish()?;
    Ok(())
}

// ----------------------------------------------------------------- PDF tables

/// Page numbers from "1-3,5" ("" = all, "4-" = 4 to the end).
pub fn parse_pages(s: &str) -> Result<Vec<u32>> {
    let mut v = vec![];
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim(), if b.trim().is_empty() { "100000" } else { b.trim() }),
            None => (part, part),
        };
        let (a, b): (u32, u32) = (a.parse().map_err(|_| anyhow!("bad page range `{part}`"))?, b.parse().map_err(|_| anyhow!("bad page range `{part}`"))?);
        if a == 0 || b < a {
            bail!("bad page range `{part}`");
        }
        v.extend(a..=b.min(a + 100_000));
    }
    Ok(v)
}

#[derive(Debug, Clone)]
struct Glyph {
    page: u32,
    x: f64,
    y: f64,
    w: f64,
    size: f64,
    ch: String,
}

#[derive(Default)]
struct Collector {
    page: u32,
    flip: Option<pdf_extract::Transform>,
    glyphs: Vec<Glyph>,
    /// Lower-left corner of each page's media box, in page order.
    origins: Vec<(u32, f64, f64)>,
}

impl pdf_extract::OutputDev for Collector {
    fn begin_page(&mut self, page: u32, media: &pdf_extract::MediaBox, _: Option<(f64, f64, f64, f64)>) -> Result<(), pdf_extract::OutputError> {
        self.page = page;
        self.flip = Some(pdf_extract::Transform::row_major(1., 0., 0., -1., 0., media.ury - media.lly));
        self.origins.push((page, media.llx, media.lly));
        Ok(())
    }
    fn end_page(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
    fn output_character(&mut self, trm: &pdf_extract::Transform, width: f64, _spacing: f64, font_size: f64, ch: &str) -> Result<(), pdf_extract::OutputError> {
        let flip = self.flip.unwrap_or_else(pdf_extract::Transform::identity);
        let p = trm.post_transform(&flip);
        let scale = (trm.m11 * trm.m11 + trm.m12 * trm.m12).sqrt().max(0.01);
        self.glyphs.push(Glyph { page: self.page, x: p.m31, y: p.m32, w: width * font_size * scale, size: font_size * scale, ch: ch.to_string() });
        Ok(())
    }
    fn begin_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
    fn end_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
    fn end_line(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
}

/// Rows of a table in a PDF (text PDFs, not scans). Characters are grouped into lines by their
/// height on the page, and a line is split into cells where the gap between characters is wider
/// than `gap` times the font size (default 0.5, about two and a half spaces). Lines with fewer than
/// `min_columns` cells are dropped, which removes headings and running text around the table.
pub fn pdf_table(path: &Path, pages: &[u32], gap: f64, min_columns: usize) -> Result<Vec<Vec<Value>>> {
    let (lines, _) = pdf_lines(path, pages, gap)?;
    Ok(lines.into_iter().filter(|(_, cells)| cells.len() >= min_columns.max(1)).map(|(_, cells)| cells.into_iter().map(|c| json!(c)).collect()).collect())
}

/// Text of a PDF laid out by position: one line per text line, cells separated by two spaces,
/// pages separated by a blank line. Returns (text, page count).
pub fn pdf_text(path: &Path, pages: &[u32]) -> Result<(String, usize)> {
    let (lines, n) = pdf_lines(path, pages, 0.0)?;
    let mut out = String::new();
    let mut last_page = None;
    for (page, cells) in lines {
        if last_page.is_some() && last_page != Some(page) {
            out.push('\n');
        }
        last_page = Some(page);
        out.push_str(&cells.join("  "));
        out.push('\n');
    }
    Ok((out.trim_end().to_string(), n))
}

/// A character on a PDF page: position in points from the page's top-left corner (`y` is the
/// baseline), advance width and font size.
#[derive(Debug, Clone)]
pub struct PageGlyph {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub size: f64,
    pub ch: String,
}

/// The characters of one PDF page.
#[derive(Debug, Clone, Default)]
pub struct PageGlyphs {
    pub glyphs: Vec<PageGlyph>,
}

/// Every page's characters with their positions, in page order (pages without text are empty).
pub fn pdf_glyph_pages(path: &Path) -> Result<Vec<PageGlyphs>> {
    let doc = pdf_extract::Document::load(path).map_err(|e| anyhow!("cannot open PDF {}: {e}", path.display()))?;
    let mut c = Collector::default();
    pdf_extract::output_doc(&doc, &mut c).map_err(|e| anyhow!("cannot read PDF {}: {e}", path.display()))?;
    let mut pages: Vec<PageGlyphs> = vec![PageGlyphs::default(); c.origins.len()];
    let index: std::collections::HashMap<u32, (usize, f64, f64)> = c.origins.iter().enumerate().map(|(i, (p, x, y))| (*p, (i, *x, *y))).collect();
    for g in c.glyphs {
        if let Some((i, llx, lly)) = index.get(&g.page) {
            pages[*i].glyphs.push(PageGlyph { x: g.x - llx, y: g.y + lly, w: g.w, size: g.size, ch: g.ch });
        }
    }
    Ok(pages)
}

/// Lines of a PDF: (page, cells).
type PdfLines = Vec<(u32, Vec<String>)>;

/// Lines of a PDF as (page, cells), plus the page count.
fn pdf_lines(path: &Path, pages: &[u32], gap: f64) -> Result<(PdfLines, usize)> {
    let doc = pdf_extract::Document::load(path).map_err(|e| anyhow!("cannot open PDF {}: {e}", path.display()))?;
    let page_count = doc.get_pages().len();
    let mut c = Collector::default();
    pdf_extract::output_doc(&doc, &mut c).map_err(|e| anyhow!("cannot read PDF {}: {e}", path.display()))?;
    let gap = if gap <= 0.0 { 0.5 } else { gap };
    let mut glyphs: Vec<Glyph> = c.glyphs.into_iter().filter(|g| pages.is_empty() || pages.contains(&g.page)).collect();
    glyphs.sort_by(|a, b| a.page.cmp(&b.page).then(a.y.total_cmp(&b.y)).then(a.x.total_cmp(&b.x)));
    // Lines: glyphs whose baselines are within a third of the font size.
    let mut lines: Vec<Vec<Glyph>> = vec![];
    for g in glyphs {
        match lines.last_mut() {
            Some(l) if l[0].page == g.page && (l[0].y - g.y).abs() <= l[0].size.max(g.size) * 0.35 => l.push(g),
            _ => lines.push(vec![g]),
        }
    }
    let mut rows = vec![];
    for mut line in lines {
        let page = line[0].page;
        line.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut cells: Vec<String> = vec![String::new()];
        let mut end: Option<f64> = None;
        // Most PDFs contain real space characters; only guess spaces for those that don't
        // (character widths are not always reliable, so a guess can split a word).
        let has_spaces = line.iter().any(|g| g.ch == " ");
        for g in &line {
            if let Some(e) = end {
                let space = g.x - e;
                if space > g.size * gap {
                    cells.push(String::new());
                } else if !has_spaces && space > g.size * 0.3 && !cells.last().unwrap().ends_with(' ') && !g.ch.trim().is_empty() {
                    cells.last_mut().unwrap().push(' ');
                }
            }
            cells.last_mut().unwrap().push_str(&g.ch);
            end = Some(g.x + g.w);
        }
        let cells: Vec<String> = cells.into_iter().map(|c| c.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|c| !c.is_empty()).collect();
        if !cells.is_empty() {
            rows.push((page, cells));
        }
    }
    Ok((rows, page_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_round_trip_and_zip_slip() {
        let dir = std::env::temp_dir().join(format!("afm_zip_{}", std::process::id()));
        let src = dir.join("in");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), "A").unwrap();
        std::fs::write(src.join("sub").join("b.pdf"), "B").unwrap();
        let z = dir.join("out.zip");
        assert_eq!(zip_create(&z, std::slice::from_ref(&src)).unwrap(), 2);
        let got = zip_extract(&z, &dir.join("x"), "", false).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(std::fs::read_to_string(dir.join("x").join("in").join("sub").join("b.pdf")).unwrap(), "B");
        assert!(zip_extract(&z, &dir.join("x"), "", false).is_err(), "existing files are not overwritten");
        assert_eq!(zip_extract(&z, &dir.join("y"), "*.pdf", false).unwrap().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn word_write_fill_read() {
        let dir = std::env::temp_dir().join(format!("afm_docx_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tpl = dir.join("tpl.docx");
        let t = crate::data::to_table(&json!([{"Item": "Pen", "Qty": 2}]), &[]).unwrap();
        word_write(&tpl, "Letter", "Dear {{name}},\nAmount: {{ amount }} & more", Some(&t)).unwrap();
        // Simulate Word splitting a placeholder across two runs.
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&tpl).unwrap()).unwrap();
        let xml = read_zip_text(&mut zip, "word/document.xml").unwrap().unwrap();
        let split = xml.replace("Dear {{name}},", "Dear {{na</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>me}},");
        let values: Map<String, Value> = serde_json::from_value(json!({"name": "Mr. O'Neil <ACME>", "amount": 1180})).unwrap();
        assert_eq!(
            fill_part(&split, &values, &mut vec![]).matches("Mr. O&apos;Neil").count()
                + fill_part(&split, &values, &mut vec![]).matches("Mr. O'Neil &lt;ACME&gt;").count(),
            1
        );
        let out = dir.join("out.docx");
        let missing = word_fill(&tpl, &out, &serde_json::from_value(json!({"name": "Anitha"})).unwrap()).unwrap();
        assert_eq!(missing, vec!["amount".to_string()]);
        let (text, tables) = word_read(&out).unwrap();
        assert!(text.contains("Dear Anitha,"), "{text}");
        assert!(text.contains("Amount: {{ amount }} & more"), "{text}");
        assert_eq!(tables, vec![vec![vec!["Item".to_string(), "Qty".to_string()], vec!["Pen".to_string(), "2".to_string()]]]);
        std::fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod pdf_tests {
    #[test]
    fn gstin_pdf_table() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/data/gstins.pdf");
        let rows = super::pdf_table(&p, &[], 0.0, 2).unwrap();
        for r in &rows {
            eprintln!("{r:?}");
        }
        assert_eq!(rows.len(), 3, "header + 2 rows");
        assert_eq!(rows[1], vec![serde_json::json!("09AAACH7409R1ZZ"), serde_json::json!("Hindustan Copper Ltd"), serde_json::json!("Uttar Pradesh")]);
    }

    #[test]
    fn invoice_items_pdf_table() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/data/invoice_items.pdf");
        let rows = super::pdf_table(&p, &[], 0.0, 5).unwrap();
        for r in &rows {
            eprintln!("{r:?}");
        }
        assert_eq!(rows.len(), 4, "header + 3 items");
        assert_eq!(rows[0].len(), 6);
        assert_eq!(rows[2], ["2", "Cable lugs", "8536", "500", "4.25", "2,125.00"].map(|s| serde_json::json!(s)).to_vec());
    }
}
