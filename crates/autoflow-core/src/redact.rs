//! Redaction of personal data, offline and without AI: Indian identifiers covered by the DPDP Act
//! (PAN, Aadhaar, mobile numbers, bank accounts...), labelled details such as "Name:", "DOB:" or
//! "UHID:", your own regular expressions and word lists.
//!
//! Identifiers are found with regular expressions; Aadhaar numbers must also pass the Verhoeff
//! check and card numbers the Luhn check (`strict`), which removes most false alarms. For labelled
//! details only the value is redacted, so "Name: Ravi Kumar" becomes "Name: [NAME]".
//!
//! Files: text files, Word (.docx) and Excel (.xlsx) are redacted in place in a copy, keeping their
//! formatting. PDFs and pictures are turned into pictures with black boxes over the personal data,
//! so nothing hidden is left behind; text PDFs use the PDF's own character positions, scanned
//! pages are read with Windows OCR.

use anyhow::{anyhow, bail, Context as _, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

/// How redacted text is replaced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaskStyle {
    /// The kind in brackets: [PAN], [MOBILE], [NAME]...
    #[default]
    Label,
    /// Every letter and digit becomes X; spaces and dashes stay (XXXXX XXXXX).
    Mask,
    /// Like mask, but the last 4 letters/digits stay visible (XXXXXXXX1234), handy for audit trails.
    Partial,
    /// The text given in `replace_with`.
    Text,
}

/// When PDFs and pictures are read with OCR.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OcrMode {
    /// Text PDFs use their own text; pages without text (scans) are read with OCR.
    #[default]
    Auto,
    /// Also OCR pages that have text (catches pictures of text inside text PDFs; slower).
    Always,
    /// Never OCR: scanned pages and pictures are left unredacted (and reported).
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Check {
    None,
    Verhoeff,
    Luhn,
}

struct Detector {
    kind: String,
    re: Regex,
    check: Check,
}

/// Built-in rules: (name, label, description).
pub const RULES: &[(&str, &str, &str)] = &[
    ("pan", "PAN", "PAN card number, e.g. ABCPE1234F"),
    ("aadhaar", "AADHAAR", "Aadhaar number, 12 digits (with or without spaces); checked with the Verhoeff checksum"),
    ("mobile", "MOBILE", "Indian mobile number: 98765 43210, +91 98765 43210, 09876543210"),
    ("email", "EMAIL", "E-mail address"),
    ("bank_account", "BANK_ACCOUNT", "Bank account number after a label such as A/c No, Account No, SB A/c"),
    ("ifsc", "IFSC", "IFSC code, e.g. SBIN0001234"),
    ("card", "CARD", "Debit / credit card number (13-19 digits); checked with the Luhn checksum"),
    ("upi", "UPI", "UPI ID, e.g. ravi@okaxis"),
    ("passport", "PASSPORT", "Indian passport number, e.g. K1234567"),
    ("voter_id", "VOTER_ID", "Voter ID (EPIC), e.g. ABC1234567"),
    ("driving_licence", "DRIVING_LICENCE", "Driving licence number, e.g. TN01 20190012345"),
    ("dob", "DOB", "Date of birth after a label: DOB, D.O.B., Date of Birth, Born on"),
    ("age", "AGE", "Age after a label: Age: 45"),
    ("gender", "GENDER", "Sex / gender after a label: Sex: M, Gender: Female"),
    ("name", "NAME", "Names after a label (Name:, Patient Name:, Father's Name:, S/o, D/o, W/o) or a title (Mr, Mrs, Ms, Dr, Shri, Smt)"),
    ("patient_id", "PATIENT_ID", "Hospital numbers after a label: UHID, MRN, MR No, IP No, OP No, Patient ID, Reg No"),
    ("health", "HEALTH", "Medical details after a label: Diagnosis, Complaints, Condition, Disease (to the end of the line)"),
    ("address", "ADDRESS", "Address after a label: Address:, R/o, Residing at (to the end of the line)"),
    ("pincode", "PINCODE", "PIN code after a label: PIN, Pincode, Postal code"),
];

/// Rules used when none are chosen: all of them.
pub fn default_rules() -> Vec<String> {
    RULES.iter().map(|r| r.0.to_string()).collect()
}

/// A person's name after a label: capitalised words separated by single spaces (two spaces end it,
/// as between table cells).
const NAME_VALUE: &str = r"(?P<value>[A-Z][A-Za-z'.-]*(?: [A-Z][A-Za-z'.-]*){0,4})";

fn builtin(rule: &str) -> Option<Vec<(&'static str, String, Check)>> {
    let r = |s: &str| s.to_string();
    Some(match rule {
        "pan" => vec![("PAN", r(r"\b[A-Z]{3}[ABCFGHJLPT][A-Z]\d{4}[A-Z]\b"), Check::None)],
        "aadhaar" => vec![("AADHAAR", r(r"\b[2-9]\d{3}[ -]?\d{4}[ -]?\d{4}\b"), Check::Verhoeff)],
        "mobile" => vec![("MOBILE", r(r"(?:\+91[ -]?[6-9]\d{4}[ -]?\d{5}|\b0?[6-9]\d{4}[ -]?\d{5})\b"), Check::None)],
        "email" => vec![("EMAIL", r(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"), Check::None)],
        "bank_account" => {
            vec![("BANK_ACCOUNT", r(r"(?i:\b(?:a/c|acc(?:oun)?t|acct)\.?(?: ?(?:no|number|num)\.?)?\s*[:#\-]?\s*)(?P<value>\d[\d -]{6,22}\d)\b"), Check::None)]
        }
        "ifsc" => vec![("IFSC", r(r"\b[A-Z]{4}0[A-Z0-9]{6}\b"), Check::None)],
        "card" => vec![("CARD", r(r"\b\d(?:[ -]?\d){12,18}\b"), Check::Luhn)],
        "upi" => vec![("UPI", r(r"\b[A-Za-z0-9._-]{2,}@[A-Za-z]{2,}\b"), Check::None)],
        "passport" => vec![("PASSPORT", r(r"\b[A-PR-WY][1-9]\d ?\d{4}[1-9]\b"), Check::None)],
        "voter_id" => vec![("VOTER_ID", r(r"\b[A-Z]{3}\d{7}\b"), Check::None)],
        "driving_licence" => vec![("DRIVING_LICENCE", r(r"\b[A-Z]{2}[ -]?\d{2}[ -]?(?:19|20)\d{2}[ -]?\d{7}\b"), Check::None)],
        "dob" => vec![(
            "DOB",
            r(
                r"(?i:\b(?:d\.? ?o\.? ?b\.?|date of birth|birth ?date|born on)\s*[:\-]?\s*)(?P<value>\d{1,2}[ ./-](?:\d{1,2}|[A-Za-z]{3,9})[ ./-]\d{2,4}|\d{4}-\d{2}-\d{2})",
            ),
            Check::None,
        )],
        "age" => vec![("AGE", r(r"(?i:\bage\s*[:\-]\s*)(?P<value>\d{1,3}(?: ?(?:yrs?|years?))?)\b"), Check::None)],
        "gender" => vec![("GENDER", r(r"(?i:\b(?:sex|gender)\s*[:\-/]\s*)(?P<value>(?i:male|female|transgender|other|m|f|t))\b"), Check::None)],
        "name" => vec![
            (
                "NAME",
                format!(
                    r"(?i:\b(?:(?:patient|father|mother|husband|wife|spouse|guardian|nominee|customer|applicant|employee|account holder)(?:'?s)? ?name|name|guardian|nominee)\s*[:\-]\s*){NAME_VALUE}"
                ),
                Check::None,
            ),
            ("NAME", format!(r"(?i:\b[SDWCH]/O\b\.?\s*[:\-]?\s*){NAME_VALUE}"), Check::None),
            ("NAME", format!(r"\b(?:Mr|Mrs|Ms|Miss|Dr|Shri|Sri|Smt|Kum|Selvi|Thiru|Tmt)\.? {NAME_VALUE}"), Check::None),
        ],
        "patient_id" => vec![(
            "PATIENT_ID",
            r(
                r"(?i:\b(?:uhid|mrn|mr ?no|ip ?no|op ?no|patient ?id|reg(?:istration)?\.? ?no|hospital ?no|case ?no)\.?\s*[:#\-]?\s*)(?P<value>[A-Za-z0-9][A-Za-z0-9/-]{2,}(?: \d[A-Za-z0-9/-]*)?)",
            ),
            Check::None,
        )],
        "health" => vec![(
            "HEALTH",
            r(
                r"(?i:\b(?:provisional diagnosis|final diagnosis|diagnosis|chief complaints?|complaints?|condition|disease)\s*[:\-]\s*)(?P<value>[^\r\n]{2,160})",
            ),
            Check::None,
        )],
        "address" => vec![("ADDRESS", r(r"(?i:\b(?:address|addr|residing at|r/o)\.?\s*[:\-]\s*)(?P<value>[^\r\n]{5,200})"), Check::None)],
        "pincode" => vec![("PINCODE", r(r"(?i:\b(?:pin ?code|pin|postal code)\s*[:\-]?\s*)(?P<value>[1-9]\d{2} ?\d{3})\b"), Check::None)],
        _ => return None,
    })
}

/// Settings for a redactor.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Built-in rule names (see RULES); empty = all.
    pub rules: Vec<String>,
    /// Your own patterns: (label, regular expression). A group named `value` limits the redaction
    /// to that part, e.g. `Emp(?:loyee)? ?ID:\s*(?P<value>\w+)`.
    pub custom: Vec<(String, String)>,
    /// Words or phrases to redact wherever they appear (whole words, any case), e.g. names.
    pub words: Vec<String>,
    pub style: MaskStyle,
    pub replace_with: String,
    /// Check Aadhaar (Verhoeff) and card (Luhn) checksums; numbers that fail are left alone.
    pub strict: bool,
}

/// A found piece of personal data: byte range in the text and its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: String,
}

/// Counts per kind, e.g. {"PAN": 2, "MOBILE": 5}.
pub type Counts = BTreeMap<String, usize>;

pub struct Redactor {
    detectors: Vec<Detector>,
    style: MaskStyle,
    replace_with: String,
    strict: bool,
    /// Identifier rules that are on and have an OCR-tolerant form.
    loose: Vec<Loose>,
    /// Everything that decides what gets redacted, as text (part of the manifest's policy hash).
    policy: String,
}

/// An identifier as OCR may misread it: look-alike characters (O/0, I/1, S/5...) allowed, then
/// fixed position by position to what the identifier needs there and checked strictly.
struct Loose {
    kind: &'static str,
    re: Regex,
    /// Per letter/digit position: 'A' = letter, '9' = digit, '0' = the digit zero, '*' = either.
    shape: &'static str,
    check: fn(&str) -> bool,
}

const DIGITISH: &str = "0-9OoDQIl|iZzSsGbBgq";
const LETTERISH: &str = "A-Z015826";

fn loose_rules(on: &[String]) -> Vec<Loose> {
    let d = DIGITISH;
    let l = LETTERISH;
    let mut v = vec![];
    let has = |n: &str| on.iter().any(|r| r == n);
    if has("pan") {
        v.push(Loose {
            kind: "PAN",
            re: Regex::new(&format!(r"\b[{l}]{{5}}[{d}]{{4}}[{l}]\b")).unwrap(),
            shape: "AAAAA9999A",
            check: |c| Regex::new(r"^[A-Z]{3}[ABCFGHJLPT][A-Z]\d{4}[A-Z]$").unwrap().is_match(c),
        });
    }
    if has("aadhaar") {
        v.push(Loose {
            kind: "AADHAAR",
            re: Regex::new(&format!(r"\b[{d}]{{4}}[ -]?[{d}]{{4}}[ -]?[{d}]{{4}}\b")).unwrap(),
            shape: "999999999999",
            check: |c| {
                let ds: Vec<u8> = c.bytes().map(|b| b - b'0').collect();
                ds[0] >= 2 && verhoeff_ok(&ds)
            },
        });
    }
    if has("mobile") {
        v.push(Loose {
            kind: "MOBILE",
            re: Regex::new(&format!(r"\b[{d}]{{5}}[ -]?[{d}]{{5}}\b")).unwrap(),
            shape: "9999999999",
            check: |c| matches!(c.as_bytes()[0], b'6'..=b'9'),
        });
    }
    if has("card") {
        v.push(Loose {
            kind: "CARD",
            re: Regex::new(&format!(r"\b[{d}]{{4}}[ -]?[{d}]{{4}}[ -]?[{d}]{{4}}[ -]?[{d}]{{4}}\b")).unwrap(),
            shape: "9999999999999999",
            check: |c| luhn_ok(&c.bytes().map(|b| b - b'0').collect::<Vec<_>>()),
        });
    }
    if has("ifsc") {
        v.push(Loose {
            kind: "IFSC",
            re: Regex::new(&format!(r"\b[{l}]{{4}}[0OoD][A-Z0-9]{{6}}\b")).unwrap(),
            shape: "AAAA0******",
            check: |c| Regex::new(r"^[A-Z]{4}0[A-Z0-9]{6}$").unwrap().is_match(c),
        });
    }
    if has("voter_id") {
        v.push(Loose { kind: "VOTER_ID", re: Regex::new(&format!(r"\b[{l}]{{3}}[{d}]{{7}}\b")).unwrap(), shape: "AAA9999999", check: |_| true });
    }
    if has("passport") {
        v.push(Loose {
            kind: "PASSPORT",
            re: Regex::new(&format!(r"\b[{l}][{d}]{{2}} ?[{d}]{{5}}\b")).unwrap(),
            shape: "A9999999",
            check: |c| Regex::new(r"^[A-PR-WY][1-9]\d{5}[1-9]$").unwrap().is_match(c),
        });
    }
    v
}

fn as_digit(c: char) -> Option<char> {
    Some(match c {
        '0'..='9' => c,
        'O' | 'o' | 'D' | 'Q' => '0',
        'I' | 'l' | '|' | 'i' => '1',
        'Z' | 'z' => '2',
        'S' | 's' => '5',
        'G' | 'b' => '6',
        'B' => '8',
        'g' | 'q' => '9',
        _ => return None,
    })
}

fn as_letter(c: char) -> Option<char> {
    Some(match c {
        'A'..='Z' => c,
        '0' => 'O',
        '1' => 'I',
        '5' => 'S',
        '8' => 'B',
        '2' => 'Z',
        '6' => 'G',
        _ => return None,
    })
}

/// The identifier with each position fixed to its kind, or None if it doesn't fit, or if most of
/// its digits had to be guessed (then it is probably a word, not a number).
fn canonical(m: &str, shape: &str) -> Option<String> {
    let chars: Vec<char> = m.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    if chars.len() != shape.len() {
        return None;
    }
    let (mut out, mut digit_slots, mut real_digits) = (String::new(), 0, 0);
    for (c, k) in chars.iter().zip(shape.chars()) {
        let fixed = match k {
            'A' => as_letter(*c)?,
            '9' | '0' => {
                digit_slots += 1;
                if c.is_ascii_digit() {
                    real_digits += 1;
                }
                as_digit(*c)?
            }
            _ => c.to_ascii_uppercase(),
        };
        out.push(fixed);
    }
    if digit_slots > 0 && real_digits * 2 < digit_slots {
        return None;
    }
    Some(out)
}

impl Redactor {
    pub fn new(o: &Options) -> Result<Self> {
        let mut detectors = vec![];
        // No rules = every built-in rule; "none" = only your own patterns and words.
        let rules: Vec<String> = if o.rules.is_empty() { default_rules() } else { o.rules.clone() };
        for rule in &rules {
            let name = rule.trim().to_ascii_lowercase().replace([' ', '-'], "_");
            if name.is_empty() || name == "none" {
                continue;
            }
            if name == "all" {
                for r in RULES {
                    for (kind, pat, check) in builtin(r.0).unwrap() {
                        detectors.push(Detector { kind: kind.to_string(), re: Regex::new(&pat).expect("built-in pattern"), check });
                    }
                }
                continue;
            }
            let Some(list) = builtin(&name) else {
                bail!("unknown redaction rule `{rule}`; known rules: {}", RULES.iter().map(|r| r.0).collect::<Vec<_>>().join(", "));
            };
            for (kind, pat, check) in list {
                detectors.push(Detector { kind: kind.to_string(), re: Regex::new(&pat).expect("built-in pattern"), check });
            }
        }
        for (label, pat) in &o.custom {
            if pat.trim().is_empty() {
                continue;
            }
            let re = Regex::new(pat).map_err(|e| anyhow!("bad pattern for `{label}`: {e}"))?;
            let kind = if label.trim().is_empty() { "CUSTOM".to_string() } else { label.trim().to_ascii_uppercase().replace(' ', "_") };
            detectors.push(Detector { kind, re, check: Check::None });
        }
        let mut words: Vec<String> = o.words.iter().map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
        if !words.is_empty() {
            words.sort_by_key(|w| std::cmp::Reverse(w.len()));
            words.dedup();
            let alt = words.iter().map(|w| regex::escape(w).replace(' ', r"\s+")).collect::<Vec<_>>().join("|");
            let re = regex::RegexBuilder::new(&format!(r"\b(?:{alt})\b"))
                .case_insensitive(true)
                .size_limit(1 << 28)
                .build()
                .map_err(|e| anyhow!("word list: {e}"))?;
            detectors.push(Detector { kind: "WORD".into(), re, check: Check::None });
        }
        if detectors.is_empty() {
            bail!("no redaction rules: choose rules, add patterns or words");
        }
        let on: Vec<String> = rules
            .iter()
            .flat_map(|r| {
                let n = r.trim().to_ascii_lowercase().replace([' ', '-'], "_");
                if n == "all" {
                    default_rules()
                } else {
                    vec![n]
                }
            })
            .collect();
        let mut sorted_rules = on.clone();
        sorted_rules.sort();
        let policy = format!("{sorted_rules:?}|{:?}|{words:?}|{:?}|{:?}|{}", o.custom, o.style, o.replace_with, o.strict);
        Ok(Self { detectors, style: o.style, replace_with: o.replace_with.clone(), strict: o.strict, loose: loose_rules(&on), policy })
    }

    /// Personal data in `text`, without overlaps (the longer match wins), in order.
    pub fn find(&self, text: &str) -> Vec<Span> {
        let mut spans = vec![];
        for d in &self.detectors {
            let has_value = d.re.capture_names().any(|n| n == Some("value"));
            for c in d.re.captures_iter(text) {
                let m = if has_value { c.name("value") } else { c.get(0) };
                let Some(m) = m else { continue };
                if m.as_str().trim().is_empty() {
                    continue;
                }
                if self.strict {
                    let digits: Vec<u8> = m.as_str().bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect();
                    let ok = match d.check {
                        Check::None => true,
                        Check::Verhoeff => digits.len() == 12 && verhoeff_ok(&digits),
                        Check::Luhn => (13..=19).contains(&digits.len()) && luhn_ok(&digits),
                    };
                    if !ok {
                        continue;
                    }
                }
                // Trim spaces a pattern may have taken at the end.
                let s = m.as_str();
                let end = m.start() + s.trim_end().len();
                spans.push(Span { start: m.start(), end, kind: d.kind.clone() });
            }
        }
        merge_spans(spans)
    }
}

/// Sort and merge overlapping matches into one span covering both (never less than either), named
/// after the longest match in it.
fn merge_spans(mut spans: Vec<Span>) -> Vec<Span> {
    {
        spans.sort_by(|a, b| a.start.cmp(&b.start).then((b.end - b.start).cmp(&(a.end - a.start))));
        let mut out: Vec<(Span, usize)> = vec![];
        for s in spans {
            let len = s.end - s.start;
            match out.last_mut() {
                Some((last, longest)) if s.start < last.end => {
                    if len > *longest {
                        last.kind = s.kind.clone();
                        *longest = len;
                    }
                    last.end = last.end.max(s.end);
                }
                _ => out.push((s, len)),
            }
        }
        out.into_iter().map(|(s, _)| s).collect()
    }
}

impl Redactor {
    /// Like `find`, for text read with OCR: identifiers with look-alike characters (a PAN read
    /// as DEFP14567J) are found too.
    pub fn find_ocr(&self, text: &str) -> Vec<Span> {
        let mut spans = self.find(text);
        // OCR often reads a capital I at the start of a word as a small l ("lyer"): read the text
        // again with those fixed (same length, so the positions stay right).
        let cs: Vec<char> = text.chars().collect();
        let fixed: String = cs
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let word_start = i == 0 || !cs[i - 1].is_alphanumeric();
                if c == 'l' && word_start && cs.get(i + 1).is_some_and(|n| n.is_lowercase()) {
                    'I'
                } else {
                    c
                }
            })
            .collect();
        if fixed != text {
            spans.extend(self.find(&fixed));
        }
        for l in &self.loose {
            for m in l.re.find_iter(text) {
                if let Some(c) = canonical(m.as_str(), l.shape) {
                    if (l.check)(&c) {
                        spans.push(Span { start: m.start(), end: m.end(), kind: l.kind.to_string() });
                    }
                }
            }
        }
        merge_spans(spans)
    }

    /// What replaces `original` (a span of the given kind).
    pub fn replacement(&self, kind: &str, original: &str) -> String {
        match self.style {
            MaskStyle::Label => format!("[{kind}]"),
            MaskStyle::Text => self.replace_with.clone(),
            MaskStyle::Mask => original.chars().map(|c| if c.is_alphanumeric() { 'X' } else { c }).collect(),
            MaskStyle::Partial => {
                let total = original.chars().filter(|c| c.is_alphanumeric()).count();
                let keep_from = total.saturating_sub(4);
                let mut i = 0;
                original
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() {
                            i += 1;
                            if i > keep_from {
                                c
                            } else {
                                'X'
                            }
                        } else {
                            c
                        }
                    })
                    .collect()
            }
        }
    }

    /// Redact text: the new text and what was found.
    pub fn redact(&self, text: &str) -> (String, Counts) {
        let spans = self.find(text);
        let mut counts = Counts::new();
        let mut out = String::with_capacity(text.len());
        let mut pos = 0;
        for s in &spans {
            out.push_str(&text[pos..s.start]);
            out.push_str(&self.replacement(&s.kind, &text[s.start..s.end]));
            *counts.entry(s.kind.clone()).or_default() += 1;
            pos = s.end;
        }
        out.push_str(&text[pos..]);
        (out, counts)
    }
}

pub fn add_counts(total: &mut Counts, more: &Counts) {
    for (k, v) in more {
        *total.entry(k.clone()).or_default() += v;
    }
}

/// "PAN: 2, MOBILE: 1"
pub fn counts_text(c: &Counts) -> String {
    c.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", ")
}

// ---------------------------------------------------------------- checksums

const VERHOEFF_D: [[u8; 10]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    [1, 2, 3, 4, 0, 6, 7, 8, 9, 5],
    [2, 3, 4, 0, 1, 7, 8, 9, 5, 6],
    [3, 4, 0, 1, 2, 8, 9, 5, 6, 7],
    [4, 0, 1, 2, 3, 9, 5, 6, 7, 8],
    [5, 9, 8, 7, 6, 0, 4, 3, 2, 1],
    [6, 5, 9, 8, 7, 1, 0, 4, 3, 2],
    [7, 6, 5, 9, 8, 2, 1, 0, 4, 3],
    [8, 7, 6, 5, 9, 3, 2, 1, 0, 4],
    [9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
];
const VERHOEFF_P: [[u8; 10]; 8] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    [1, 5, 7, 6, 2, 8, 3, 0, 9, 4],
    [5, 8, 0, 3, 7, 9, 6, 1, 4, 2],
    [8, 9, 1, 6, 0, 4, 3, 5, 2, 7],
    [9, 4, 5, 3, 1, 2, 6, 8, 7, 0],
    [4, 2, 8, 6, 5, 7, 3, 9, 0, 1],
    [2, 7, 9, 3, 8, 0, 6, 4, 1, 5],
    [7, 0, 4, 6, 9, 1, 3, 2, 5, 8],
];

/// Verhoeff checksum (Aadhaar's last digit).
pub fn verhoeff_ok(digits: &[u8]) -> bool {
    let mut c = 0u8;
    for (i, &d) in digits.iter().rev().enumerate() {
        c = VERHOEFF_D[c as usize][VERHOEFF_P[i % 8][d as usize] as usize];
    }
    c == 0
}

/// Luhn checksum (card numbers).
pub fn luhn_ok(digits: &[u8]) -> bool {
    let mut sum = 0u32;
    for (i, &d) in digits.iter().rev().enumerate() {
        let mut v = d as u32;
        if i % 2 == 1 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    sum.is_multiple_of(10)
}

// ---------------------------------------------------------------- XML (Word, Excel)

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(r"&(#x[0-9A-Fa-f]+|#\d+|lt|gt|amp|quot|apos);").unwrap());
    RE.replace_all(s, |c: &regex::Captures| {
        let e = &c[1];
        match e {
            "lt" => "<".into(),
            "gt" => ">".into(),
            "amp" => "&".into(),
            "quot" => "\"".into(),
            "apos" => "'".into(),
            _ => {
                let n = if let Some(h) = e.strip_prefix("#x") { u32::from_str_radix(h, 16).ok() } else { e[1..].parse().ok() };
                n.and_then(char::from_u32).map(|c| c.to_string()).unwrap_or_else(|| c[0].to_string())
            }
        }
    })
    .into_owned()
}

/// Redact the text of an XML part. Text elements (`<w:t>` in Word, `<t>` in Excel) between two
/// boundaries (paragraphs, shared strings) are read together, so data split over formatting runs
/// is still found; each piece keeps its own formatting.
fn redact_xml(xml: &str, tokens: &Regex, red: &Redactor, counts: &mut Counts) -> Option<String> {
    struct Seg {
        content: (usize, usize),
        text: String,
    }
    let mut groups: Vec<Vec<Seg>> = vec![vec![]];
    for c in tokens.captures_iter(xml) {
        if c.name("b").is_some() {
            groups.push(vec![]);
        } else if let Some(m) = c.name("c") {
            groups.last_mut().unwrap().push(Seg { content: (m.start(), m.end()), text: xml_unescape(m.as_str()) });
        }
    }
    let mut edits: Vec<((usize, usize), String)> = vec![];
    for g in groups.iter().filter(|g| !g.is_empty()) {
        let joined: String = g.iter().map(|s| s.text.as_str()).collect();
        let spans = red.find(&joined);
        if spans.is_empty() {
            continue;
        }
        for s in &spans {
            *counts.entry(s.kind.clone()).or_default() += 1;
        }
        let mut offset = 0;
        for seg in g {
            let (a, b) = (offset, offset + seg.text.len());
            offset = b;
            if !spans.iter().any(|s| s.start < b && s.end > a) {
                continue;
            }
            let mut out = String::new();
            let mut pos = a;
            for s in spans.iter().filter(|s| s.start < b && s.end > a) {
                if s.start > pos {
                    out.push_str(&joined[pos..s.start]);
                }
                if s.start >= a {
                    out.push_str(&red.replacement(&s.kind, &joined[s.start..s.end]));
                }
                pos = s.end.min(b).max(pos);
            }
            if pos < b {
                out.push_str(&joined[pos..b]);
            }
            edits.push((seg.content, xml_escape(&out)));
        }
    }
    if edits.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(xml.len());
    let mut pos = 0;
    for ((a, b), t) in edits {
        out.push_str(&xml[pos..a]);
        out.push_str(&t);
        pos = b;
    }
    out.push_str(&xml[pos..]);
    Some(out)
}

fn word_tokens() -> Regex {
    Regex::new(r"(?P<b><w:p[ >/]|</w:p>|<w:tc[ >]|</w:tc>)|<w:(?:t|delText|instrText)(?:\s[^>]*)?>(?P<c>[^<]*)</w:(?:t|delText|instrText)>").unwrap()
}

fn excel_tokens() -> Regex {
    Regex::new(r"(?P<b><si>|</si>|<is>|</is>|<comment[ >]|</comment>)|<t(?:\s[^>]*)?>(?P<c>[^<]*)</t>").unwrap()
}

/// Excel cells holding a plain number (no formula, not text), e.g. a mobile number typed as a number.
fn redact_numeric_cells(xml: &str, red: &Redactor, counts: &mut Counts) -> Option<String> {
    let re = Regex::new(r#"<c\b(?P<a>[^>]*)><v>(?P<v>[^<]*)</v></c>"#).unwrap();
    let t_attr = Regex::new(r#"\st="[^"]*""#).unwrap();
    let mut changed = false;
    let out = re.replace_all(xml, |c: &regex::Captures| {
        let attrs = &c["a"];
        let is_number = !attrs.contains(" t=") || attrs.contains(r#" t="n""#);
        if !is_number {
            return c[0].to_string();
        }
        let v = &c["v"];
        // 9.87654321E9 -> 9876543210
        let text = match v.parse::<f64>() {
            Ok(f) if v.contains(['E', 'e']) && f.fract() == 0.0 && f.abs() < 1e19 => format!("{}", f as i128),
            _ => v.to_string(),
        };
        let (new, found) = red.redact(&text);
        if found.is_empty() {
            return c[0].to_string();
        }
        add_counts(counts, &found);
        changed = true;
        let attrs = t_attr.replace_all(attrs, "");
        format!(r#"<c{attrs} t="inlineStr"><is><t>{}</t></is></c>"#, xml_escape(&new))
    });
    changed.then(|| out.into_owned())
}

/// Document properties (docProps/core.xml, app.xml, custom.xml): author fields are emptied and
/// every other property text (title, subject, keywords, comments, custom values) is redacted.
fn clean_props(xml: &str, red: &Redactor, counts: &mut Counts) -> String {
    let authors = Regex::new(r"(?s)<(dc:creator|cp:lastModifiedBy|Manager)>.*?</(?:dc:creator|cp:lastModifiedBy|Manager)>").unwrap();
    let xml = authors.replace_all(xml, "<$1></$1>");
    let text = Regex::new(r">([^<>]+)<").unwrap();
    text.replace_all(&xml, |c: &regex::Captures| {
        let (new, found) = red.redact(&xml_unescape(&c[1]));
        add_counts(counts, &found);
        format!(">{}<", xml_escape(&new))
    })
    .into_owned()
}

/// Names of people stored in attributes: comment and tracked-change authors, Excel comment
/// authors and threaded-comment people.
fn clear_author_attributes(xml: &str) -> String {
    let attrs = Regex::new(r#"\b(w:author|w:initials|displayName|userId|providerId)="[^"]*""#).unwrap();
    let excel_authors = Regex::new(r"(?s)<author>.*?</author>").unwrap();
    let xml = attrs.replace_all(xml, r#"$1="""#);
    excel_authors.replace_all(&xml, "<author></author>").into_owned()
}

fn redact_office(input: &Path, output: &Path, red: &Redactor, fo: &FileOptions) -> Result<Outcome> {
    let ext = ext_of(input);
    let file = std::fs::File::open(input).with_context(|| format!("cannot open {}", input.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| anyhow!("not a valid .{ext} file (or it is password-protected)"))?;
    if zip.len() > MAX_OFFICE_PARTS {
        bail!("{} parts inside, over the limit of {MAX_OFFICE_PARTS}", zip.len());
    }
    let mut unpacked = 0u64;
    for i in 0..zip.len() {
        unpacked += zip.by_index_raw(i).map(|e| e.size()).unwrap_or(0);
    }
    if unpacked > MAX_OFFICE_UNPACKED {
        bail!("unpacks to {} MB, over the limit of {} MB", unpacked >> 20, MAX_OFFICE_UNPACKED >> 20);
    }
    let mut o = Outcome::default();
    let (word, excel) = (word_tokens(), excel_tokens());
    let mut parts: Vec<(String, Vec<u8>)> = vec![];
    let (mut pictures, mut unreadable) = (0usize, 0usize);
    for i in 0..zip.len() {
        fo.check_stop()?;
        let mut entry = zip.by_index(i)?;
        let name = entry.name().map(|n| n.into_owned()).map_err(|e| anyhow!("bad entry name: {e}"))?;
        let mut data = vec![];
        entry.by_ref().take(MAX_OFFICE_UNPACKED).read_to_end(&mut data)?;
        let xml_part = name.ends_with(".xml");
        let mut new = if xml_part && fo.clear_metadata && name.starts_with("docProps/") {
            Some(clean_props(&String::from_utf8_lossy(&data), red, &mut o.counts))
        } else if xml_part && ext == "docx" && name.starts_with("word/") {
            redact_xml(&String::from_utf8_lossy(&data), &word, red, &mut o.counts)
        } else if xml_part && ext == "xlsx" && name.starts_with("xl/") {
            let xml = String::from_utf8_lossy(&data);
            let mut cur = redact_xml(&xml, &excel, red, &mut o.counts);
            if name.starts_with("xl/worksheets/") {
                let base = cur.clone().unwrap_or_else(|| xml.to_string());
                if let Some(n) = redact_numeric_cells(&base, red, &mut o.counts) {
                    cur = Some(n);
                }
            }
            cur
        } else {
            None
        };
        if xml_part && fo.clear_metadata && (name.starts_with("word/") || name.starts_with("xl/")) {
            let base = new.clone().unwrap_or_else(|| String::from_utf8_lossy(&data).into_owned());
            let cleared = clear_author_attributes(&base);
            if cleared != base {
                new = Some(cleared);
            }
        }
        let mut bytes = new.map(String::into_bytes).unwrap_or(data);
        // Pictures inside the document: read with OCR and black out like picture files.
        if name.starts_with("word/media/") || name.starts_with("xl/media/") {
            pictures += 1;
            let pext = ext_of(Path::new(&name));
            if IMAGE_EXT.contains(&pext.as_str()) && fo.ocr != OcrMode::Never {
                match load_picture(&bytes) {
                    Ok(mut img) => {
                        let (rects, verified) = check_picture(&img, red, fo, &mut o.counts)?;
                        if !verified {
                            unreadable += 1;
                        } else if !rects.is_empty() {
                            black_out(&mut img, &rects);
                            bytes = encode_like(img, &pext)?;
                        }
                    }
                    Err(_) => unreadable += 1,
                }
            } else {
                unreadable += 1;
            }
        }
        parts.push((name, bytes));
    }
    if unreadable > 0 {
        o.unverified += unreadable;
        o.notes.push(format!("{unreadable} of {pictures} picture(s) inside could not be read (format, no readable text, or ocr off)"));
    } else if pictures > 0 {
        o.notes.push(format!("{pictures} picture(s) inside checked with OCR"));
    }
    if !fo.dry_run && o.unverified == 0 {
        write_zip(output, &parts)?;
    }
    Ok(o)
}

fn write_zip(output: &Path, parts: &[(String, Vec<u8>)]) -> Result<()> {
    let tmp = temp_for(output);
    let r = write_zip_to(&tmp, output, parts).and_then(|_| replace_file(&tmp, output));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn write_zip_to(tmp: &Path, output: &Path, parts: &[(String, Vec<u8>)]) -> Result<()> {
    {
        let mut w = zip::ZipWriter::new(std::fs::File::create(tmp).with_context(|| format!("cannot write {}", output.display()))?);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in parts {
            w.start_file(name.as_str(), opts)?;
            w.write_all(data)?;
        }
        w.finish()?;
    }
    Ok(())
}

/// A temporary file next to `out` (same folder, so the final rename is atomic).
fn temp_for(out: &Path) -> PathBuf {
    let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    out.with_file_name(format!(".{name}.{}-{n}.afmtmp", std::process::id()))
}

/// Write a file all at once: to a temporary file first, then renamed into place, so a failure or
/// a stop never leaves a half-written output.
fn write_atomic(out: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = temp_for(out);
    let r = std::fs::write(&tmp, bytes).with_context(|| format!("cannot write {}", out.display())).and_then(|_| replace_file(&tmp, out));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn replace_file(tmp: &Path, out: &Path) -> Result<()> {
    std::fs::rename(tmp, out)
        .or_else(|_| std::fs::copy(tmp, out).map(|_| ()).and_then(|_| std::fs::remove_file(tmp)))
        .with_context(|| format!("cannot write {} (is it open in another program?)", out.display()))
}

// ---------------------------------------------------------------- text files

/// Limits that keep one huge or malformed file from exhausting memory or disk. A file over a
/// limit fails (and is not written).
pub const MAX_FILE_BYTES: u64 = 1 << 30; // 1 GB
pub const MAX_PDF_PAGES: usize = 5_000;
pub const MAX_PICTURE_PIXELS: u64 = 150_000_000; // 150 megapixels
pub const MAX_OFFICE_UNPACKED: u64 = 4 << 30; // 4 GB in total inside a .docx / .xlsx
pub const MAX_OFFICE_PARTS: usize = 50_000;

/// Open a picture with size limits.
fn load_picture(bytes: &[u8]) -> Result<image::RgbaImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(40_000);
    limits.max_image_height = Some(40_000);
    limits.max_alloc = Some(MAX_PICTURE_PIXELS * 4);
    reader.limits(limits);
    let (w, h) = reader.into_dimensions().map_err(|e| anyhow!("cannot read picture: {e}"))?;
    if w as u64 * h as u64 > MAX_PICTURE_PIXELS {
        bail!("picture is {w} x {h} pixels, over the limit of {} megapixels", MAX_PICTURE_PIXELS / 1_000_000);
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PICTURE_PIXELS * 4);
    reader.limits(limits);
    Ok(reader.decode().map_err(|e| anyhow!("cannot read picture: {e}"))?.to_rgba8())
}

const TEXT_EXT: &[&str] = &["txt", "csv", "tsv", "json", "xml", "html", "htm", "md", "log", "ini", "yaml", "yml", "sql", "eml", "rtf"];
const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "bmp", "tif", "tiff"];

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Can this file be redacted?
pub fn supported(p: &Path) -> bool {
    let e = ext_of(p);
    TEXT_EXT.contains(&e.as_str()) || IMAGE_EXT.contains(&e.as_str()) || e == "docx" || e == "xlsx" || e == "pdf"
}

fn read_text_any(p: &Path) -> Result<String> {
    let bytes = std::fs::read(p).with_context(|| format!("cannot read {}", p.display()))?;
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Ok(String::from_utf8_lossy(rest).into_owned());
    }
    if bytes.len() >= 2 && (bytes[0..2] == [0xFF, 0xFE] || bytes[0..2] == [0xFE, 0xFF]) {
        let le = bytes[0] == 0xFF;
        let units: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
        return Ok(String::from_utf16_lossy(&units));
    }
    match String::from_utf8(bytes) {
        Ok(s) => Ok(s),
        // Old ANSI files: Windows-1252 is close enough for digits and Latin letters.
        Err(e) => Ok(e.into_bytes().iter().map(|&b| b as char).collect()),
    }
}

// ---------------------------------------------------------------- PDFs and pictures

/// Settings for files.
#[derive(Debug, Clone)]
pub struct FileOptions {
    pub clear_metadata: bool,
    pub ocr: OcrMode,
    pub ocr_language: String,
    /// Resolution of redacted PDF pages (dots per inch).
    pub dpi: u32,
    pub dry_run: bool,
    /// Set to stop: checked before every page and picture.
    pub stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// OCR also reads text turned sideways or upside down (four passes).
    pub ocr_rotated: bool,
}

impl Default for FileOptions {
    fn default() -> Self {
        Self { clear_metadata: true, ocr: OcrMode::Auto, ocr_language: String::new(), dpi: 150, dry_run: false, stop: Default::default(), ocr_rotated: true }
    }
}

impl FileOptions {
    fn check_stop(&self) -> Result<()> {
        if self.stop.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("stopped");
        }
        Ok(())
    }
}

/// What redacting one file found.
#[derive(Debug, Default)]
pub struct Outcome {
    pub counts: Counts,
    pub pages: usize,
    /// Pages or pictures that could not be read (no text recognised on non-blank content, a
    /// picture format OCR can't open, or OCR switched off). Such a file is not written: it may
    /// still hold personal data.
    pub unverified: usize,
    pub notes: Vec<String>,
}

/// A box to black out, in image pixels.
#[derive(Debug, Clone, Copy)]
struct Rect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

/// Text with a box for every byte (spaces between words have none).
struct BoxedLine {
    text: String,
    boxes: Vec<Option<Rect>>,
}

impl BoxedLine {
    fn new() -> Self {
        Self { text: String::new(), boxes: vec![] }
    }
    fn push(&mut self, s: &str, r: Option<Rect>) {
        self.text.push_str(s);
        self.boxes.extend(std::iter::repeat_n(r, s.len()));
    }
}

/// Personal data in some lines: (kind, box) for each match; `ocr` = the lines come from OCR.
fn hits(lines: &[BoxedLine], red: &Redactor, ocr: bool) -> Vec<(String, Rect)> {
    let mut out = vec![];
    for l in lines {
        let spans = if ocr { red.find_ocr(&l.text) } else { red.find(&l.text) };
        for s in spans {
            let mut r: Option<Rect> = None;
            for b in l.boxes[s.start..s.end].iter().flatten() {
                r = Some(match r {
                    None => *b,
                    Some(a) => Rect { x0: a.x0.min(b.x0), y0: a.y0.min(b.y0), x1: a.x1.max(b.x1), y1: a.y1.max(b.y1) },
                });
            }
            if let Some(r) = r {
                out.push((s.kind, r));
            }
        }
    }
    out
}

/// Boxes to black out for the personal data in some lines, counting what was found.
fn boxes_for(lines: &[BoxedLine], red: &Redactor, counts: &mut Counts) -> Vec<Rect> {
    hits(lines, red, false)
        .into_iter()
        .map(|(k, r)| {
            *counts.entry(k).or_default() += 1;
            r
        })
        .collect()
}

/// Did OCR read real text (some word of 3+ letters/digits), not just specks?
fn readable(lines: &[BoxedLine]) -> bool {
    lines.iter().any(|l| l.text.split_whitespace().any(|w| w.chars().filter(|c| c.is_alphanumeric()).count() >= 3))
}

/// OCR in all four orientations (text turned sideways or upside down is read too). Returns the
/// hits mapped back onto the picture, and whether any orientation read real text.
fn ocr_hits(img: &image::RgbaImage, red: &Redactor, fo: &FileOptions) -> Result<(Vec<(String, Rect)>, bool)> {
    use image::imageops::{rotate180, rotate270, rotate90};
    let (w, h) = (img.width() as f32, img.height() as f32);
    let mut all = vec![];
    let mut any_readable = false;
    for turn in [0u16, 90, 180, 270] {
        fo.check_stop()?;
        let lines = match turn {
            0 => ocr_lines(img, &fo.ocr_language)?,
            90 => ocr_lines(&rotate90(img), &fo.ocr_language)?,
            180 => ocr_lines(&rotate180(img), &fo.ocr_language)?,
            _ => ocr_lines(&rotate270(img), &fo.ocr_language)?,
        };
        any_readable |= readable(&lines);
        for (k, r) in hits(&lines, red, true) {
            // Back to the picture's own coordinates.
            let r = match turn {
                0 => r,
                90 => Rect { x0: r.y0, x1: r.y1, y0: h - r.x1, y1: h - r.x0 },
                180 => Rect { x0: w - r.x1, x1: w - r.x0, y0: h - r.y1, y1: h - r.y0 },
                _ => Rect { x0: w - r.y1, x1: w - r.y0, y0: r.x0, y1: r.x1 },
            };
            all.push((k, r));
        }
        if !fo.ocr_rotated {
            break;
        }
    }
    Ok((all, any_readable))
}

fn ocr_lines(img: &image::RgbaImage, language: &str) -> Result<Vec<BoxedLine>> {
    let out = crate::ocr::recognize(img, language, 1.5)?;
    let mut lines = vec![];
    for l in out.lines {
        let mut bl = BoxedLine::new();
        for (i, w) in l.words.iter().enumerate() {
            if i > 0 {
                bl.push(" ", None);
            }
            // Each character gets its share of the word's box.
            let n = w.text.chars().count().max(1) as f32;
            for (j, ch) in w.text.chars().enumerate() {
                let x0 = w.x + w.w * j as f32 / n;
                let x1 = w.x + w.w * (j + 1) as f32 / n;
                let mut buf = [0u8; 4];
                bl.push(ch.encode_utf8(&mut buf), Some(Rect { x0, y0: w.y, x1, y1: w.y + w.h }));
            }
        }
        lines.push(bl);
    }
    Ok(lines)
}

fn black_out(img: &mut image::RgbaImage, rects: &[Rect]) {
    let (w, h) = (img.width() as i64, img.height() as i64);
    for r in rects {
        let pad = ((r.y1 - r.y0).min(r.x1 - r.x0) * 0.15).clamp(2.0, 12.0);
        let x0 = ((r.x0 - pad).floor() as i64).clamp(0, w);
        let x1 = ((r.x1 + pad).ceil() as i64).clamp(0, w);
        let y0 = ((r.y0 - pad).floor() as i64).clamp(0, h);
        let y1 = ((r.y1 + pad).ceil() as i64).clamp(0, h);
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x as u32, y as u32, image::Rgba([0, 0, 0, 255]));
            }
        }
    }
}

/// Is a picture (nearly) empty: almost no dark pixels? Then finding no text is expected.
fn mostly_blank(img: &image::RgbaImage) -> bool {
    let (mut dark, mut total) = (0u64, 0u64);
    for (i, p) in img.pixels().enumerate() {
        if i % 3 != 0 {
            continue;
        }
        total += 1;
        let l = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
        if p[3] > 32 && l < 170.0 {
            dark += 1;
        }
    }
    total == 0 || (dark as f64) / (total as f64) < 0.002
}

/// OCR a picture: the boxes to black out, and whether the picture could be checked (real text
/// was read, or the picture is blank or a tiny icon).
fn check_picture(img: &image::RgbaImage, red: &Redactor, fo: &FileOptions, counts: &mut Counts) -> Result<(Vec<Rect>, bool)> {
    if img.width() < 40 || img.height() < 16 {
        return Ok((vec![], true)); // too small to hold readable text
    }
    let (found, readable) = ocr_hits(img, red, fo)?;
    let verified = readable || mostly_blank(img);
    Ok((
        found
            .into_iter()
            .map(|(k, r)| {
                *counts.entry(k).or_default() += 1;
                r
            })
            .collect(),
        verified,
    ))
}

/// A picture encoded in the format of the extension (png, jpg, bmp, tiff).
fn encode_like(img: image::RgbaImage, ext: &str) -> Result<Vec<u8>> {
    let fmt = image::ImageFormat::from_extension(ext).ok_or_else(|| anyhow!("cannot write .{ext} pictures"))?;
    let mut buf = std::io::Cursor::new(vec![]);
    let dynimg = image::DynamicImage::ImageRgba8(img);
    if fmt == image::ImageFormat::Jpeg {
        let rgb = dynimg.to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 92).encode_image(&rgb)?;
    } else if fmt == image::ImageFormat::Bmp {
        image::DynamicImage::ImageRgb8(dynimg.to_rgb8()).write_to(&mut buf, fmt)?;
    } else {
        dynimg.write_to(&mut buf, fmt)?;
    }
    Ok(buf.into_inner())
}

fn redact_image(input: &Path, output: &Path, red: &Redactor, fo: &FileOptions) -> Result<Outcome> {
    if fo.ocr == OcrMode::Never {
        bail!("pictures need OCR (ocr is set to never)");
    }
    let bytes = std::fs::read(input).with_context(|| format!("cannot read {}", input.display()))?;
    let mut img = load_picture(&bytes)?;
    let mut o = Outcome { pages: 1, ..Default::default() };
    let (rects, verified) = check_picture(&img, red, fo, &mut o.counts)?;
    if !verified {
        o.unverified = 1;
        o.notes.push("no text could be read in the picture (blurred, rotated, handwritten or another language?)".into());
    }
    if !fo.dry_run && o.unverified == 0 {
        black_out(&mut img, &rects);
        let bytes = encode_like(img, &ext_of(output))?;
        write_atomic(output, &bytes)?;
    }
    Ok(o)
}

/// Lines of a text PDF page with each character's box, in PDF points from the top-left corner.
fn pdf_page_lines(glyphs: &[crate::documents::PageGlyph]) -> Vec<BoxedLine> {
    let mut gs: Vec<&crate::documents::PageGlyph> = glyphs.iter().collect();
    gs.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    let mut rows: Vec<Vec<&crate::documents::PageGlyph>> = vec![];
    for g in gs {
        match rows.last_mut() {
            Some(r) if (r[0].y - g.y).abs() <= r[0].size.max(g.size) * 0.35 => r.push(g),
            _ => rows.push(vec![g]),
        }
    }
    let mut lines = vec![];
    for mut r in rows {
        r.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut bl = BoxedLine::new();
        let mut last_end: Option<f64> = None;
        for g in r {
            if let Some(e) = last_end {
                let gap = g.x - e;
                if gap > g.size * 1.2 {
                    bl.push("  ", None);
                } else if gap > g.size * 0.18 && !bl.text.ends_with(' ') {
                    bl.push(" ", None);
                }
            }
            let rect = Rect { x0: g.x as f32, y0: (g.y - g.size * 0.85) as f32, x1: (g.x + g.w.max(g.size * 0.3)) as f32, y1: (g.y + g.size * 0.25) as f32 };
            bl.push(&g.ch, if g.ch.trim().is_empty() { None } else { Some(rect) });
            last_end = Some(g.x + g.w);
        }
        lines.push(bl);
    }
    lines
}

/// Which pages hold pictures (embedded scans, photos of IDs). Unknown = assume yes.
fn pdf_pages_with_pictures(path: &Path, pages: usize) -> Vec<bool> {
    let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<Vec<bool>> {
        let doc = lopdf::Document::load(path).ok()?;
        Some(
            doc.get_pages()
                .values()
                .map(|id| doc.get_page_images(*id).map(|imgs| imgs.iter().any(|i| i.width * i.height >= 64 * 64)).unwrap_or(true))
                .collect(),
        )
    }))
    .ok()
    .flatten();
    match found {
        Some(v) if v.len() == pages => v,
        _ => vec![true; pages],
    }
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

#[cfg(windows)]
fn redact_pdf(input: &Path, output: &Path, red: &Redactor, fo: &FileOptions) -> Result<Outcome> {
    // Text and character positions from the PDF itself (some PDFs make the reader panic: then OCR).
    let glyph_pages = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::documents::pdf_glyph_pages(input))).ok().and_then(|r| r.ok());
    let renderer = pdfrender::Pdf::open(input)?;
    let n = renderer.page_count();
    if n > MAX_PDF_PAGES {
        bail!("{n} pages, over the limit of {MAX_PDF_PAGES}");
    }
    let with_pictures = pdf_pages_with_pictures(input, n);
    let mut o = Outcome { pages: n, ..Default::default() };
    let mut pdf = PdfBuilder::new();
    let mut unread_pages = vec![];
    for (i, &has_pictures) in with_pictures.iter().enumerate().take(n) {
        fo.check_stop()?;
        let (mut img, w_pt, h_pt) = renderer.render(i, fo.dpi)?;
        let sx = img.width() as f32 / w_pt.max(1.0) as f32;
        let sy = img.height() as f32 / h_pt.max(1.0) as f32;
        let page = glyph_pages.as_ref().and_then(|p| p.get(i));
        let glyph_count = page.map(|p| p.glyphs.iter().filter(|g| !g.ch.trim().is_empty()).count()).unwrap_or(0);
        let mut rects = vec![];
        if let Some(p) = page {
            for r in boxes_for(&pdf_page_lines(&p.glyphs), red, &mut o.counts) {
                rects.push(Rect { x0: r.x0 * sx, y0: r.y0 * sy, x1: r.x1 * sx, y1: r.y1 * sy });
            }
        }
        let use_ocr = match fo.ocr {
            OcrMode::Always => true,
            OcrMode::Auto => glyph_count < 20 || has_pictures,
            OcrMode::Never => false,
        };
        if use_ocr {
            fo.check_stop()?;
            let (found, readable) = ocr_hits(&img, red, fo)?;
            // Count only what the PDF's own text did not already cover.
            for (kind, r) in &found {
                if !rects.iter().any(|g| overlaps(g, r)) {
                    *o.counts.entry(kind.clone()).or_default() += 1;
                }
            }
            rects.extend(found.into_iter().map(|(_, r)| r));
            if glyph_count == 0 && !readable && !mostly_blank(&img) {
                unread_pages.push(i + 1);
            }
        } else if glyph_count == 0 && !mostly_blank(&img) {
            unread_pages.push(i + 1);
        }
        if !fo.dry_run && unread_pages.is_empty() {
            black_out(&mut img, &rects);
            pdf.add_page(img, w_pt, h_pt)?;
        }
    }
    if !unread_pages.is_empty() {
        o.unverified = unread_pages.len();
        let list = unread_pages.iter().take(10).map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        o.notes.push(if fo.ocr == OcrMode::Never {
            format!("page(s) {list} have no text and ocr is off")
        } else {
            format!("no text could be read on page(s) {list} (blurred, rotated, handwritten or another language?)")
        });
    }
    if !fo.dry_run && o.unverified == 0 {
        pdf.save(output)?;
    }
    Ok(o)
}

#[cfg(not(windows))]
fn redact_pdf(_input: &Path, _output: &Path, _red: &Redactor, _fo: &FileOptions) -> Result<Outcome> {
    bail!("PDF redaction is only available on Windows")
}

/// A PDF made of page pictures (no text layer, so nothing redacted can be copied out). Pages are
/// compressed as they are added, so only one page picture is in memory at a time.
struct PdfBuilder {
    doc: lopdf::Document,
    pages_id: lopdf::ObjectId,
    kids: Vec<lopdf::Object>,
}

impl PdfBuilder {
    fn new() -> Self {
        let mut doc = lopdf::Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        Self { doc, pages_id, kids: vec![] }
    }

    fn add_page(&mut self, img: image::RgbaImage, w: f64, h: f64) -> Result<()> {
        use lopdf::{dictionary, Object, Stream};
        let (pw, ph) = (img.width() as i64, img.height() as i64);
        let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
        let mut stream = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => pw,
                "Height" => ph,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
            },
            rgb.into_raw(),
        );
        stream.compress().map_err(|e| anyhow!("PDF: {e}"))?;
        let image_id = self.doc.add_object(stream);
        let content = format!("q {w:.2} 0 0 {h:.2} 0 0 cm /Im0 Do Q");
        let content_id = self.doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = self.doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(w as f32), Object::Real(h as f32)],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
            "Contents" => content_id,
        });
        self.kids.push(Object::Reference(page_id));
        Ok(())
    }

    fn save(mut self, output: &Path) -> Result<()> {
        use lopdf::{dictionary, Object};
        let count = self.kids.len() as i64;
        let kids = std::mem::take(&mut self.kids);
        self.doc.objects.insert(self.pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
        let catalog_id = self.doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => self.pages_id });
        self.doc.trailer.set("Root", catalog_id);
        let tmp = temp_for(output);
        let r = self.doc.save(&tmp).map_err(|e| anyhow!("cannot write {}: {e}", output.display())).and_then(|_| replace_file(&tmp, output));
        if r.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        r
    }
}

#[cfg(windows)]
mod pdfrender {
    //! PDF pages to pictures with the PDF renderer built into Windows (Windows.Data.Pdf).
    use anyhow::{anyhow, Result};
    use std::path::Path;
    use windows::core::HSTRING;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};

    pub struct Pdf {
        doc: PdfDocument,
    }

    impl Pdf {
        pub fn open(path: &Path) -> Result<Self> {
            unsafe {
                let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
            }
            let abs = std::path::absolute(path)?;
            let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(abs.to_string_lossy().as_ref()))?
                .join()
                .map_err(|e| anyhow!("cannot open {}: {e}", path.display()))?;
            let doc = PdfDocument::LoadFromFileAsync(&file)?
                .join()
                .map_err(|e| anyhow!("cannot open PDF {} (password-protected or damaged?): {e}", path.display()))?;
            Ok(Self { doc })
        }

        pub fn page_count(&self) -> usize {
            self.doc.PageCount().unwrap_or(0) as usize
        }

        /// Page `i` as a picture at `dpi`, with the page size in points.
        pub fn render(&self, i: usize, dpi: u32) -> Result<(image::RgbaImage, f64, f64)> {
            let page = self.doc.GetPage(i as u32)?;
            let size = page.Size()?; // device-independent pixels, 1/96 inch
            let (w_pt, h_pt) = (size.Width as f64 * 72.0 / 96.0, size.Height as f64 * 72.0 / 96.0);
            let opts = PdfPageRenderOptions::new()?;
            let dpi = dpi.clamp(72, 400) as f64;
            opts.SetDestinationWidth((size.Width as f64 * dpi / 96.0).round() as u32)?;
            opts.SetDestinationHeight((size.Height as f64 * dpi / 96.0).round() as u32)?;
            let stream = InMemoryRandomAccessStream::new()?;
            page.RenderWithOptionsToStreamAsync(&stream, &opts)?.join().map_err(|e| anyhow!("cannot render page {}: {e}", i + 1))?;
            let len = stream.Size()? as u32;
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
            reader.LoadAsync(len)?.join()?;
            let mut buf = vec![0u8; len as usize];
            reader.ReadBytes(&mut buf)?;
            let img = image::load_from_memory(&buf).map_err(|e| anyhow!("page {}: {e}", i + 1))?.to_rgba8();
            Ok((img, w_pt, h_pt))
        }
    }
}

// ---------------------------------------------------------------- one file, many files

/// What happened to one file.
#[derive(Debug, Clone)]
pub struct FileResult {
    /// Path relative to the input folder (or the file name).
    pub file: String,
    /// redacted, clean (read completely, nothing found), unchanged (the existing output is proven
    /// to come from this input with these settings), unverified (some content could not be read:
    /// not written), skipped (type not supported: not copied), failed (error: not written).
    pub status: &'static str,
    pub counts: Counts,
    pub pages: usize,
    pub note: String,
}

/// Redact one file into `output` (same type). Nothing is written when part of the file could not
/// be checked (`Outcome::unverified`).
pub fn redact_file(input: &Path, output: &Path, red: &Redactor, fo: &FileOptions) -> Result<Outcome> {
    if same_file(input, output) {
        bail!("the output must be a different file than the input");
    }
    let norm = |e: String| match e.as_str() {
        "jpeg" => "jpg".to_string(),
        "tif" => "tiff".to_string(),
        _ => e,
    };
    if norm(ext_of(input)) != norm(ext_of(output)) {
        bail!("the redacted copy keeps the file type: name the output .{} (or give a folder)", ext_of(input));
    }
    if !fo.dry_run {
        if let Some(d) = output.parent() {
            std::fs::create_dir_all(d).with_context(|| format!("cannot create {}", d.display()))?;
        }
    }
    let ext = ext_of(input);
    let size = std::fs::metadata(input).with_context(|| format!("cannot read {}", input.display()))?.len();
    if size > MAX_FILE_BYTES {
        bail!("{} MB, over the limit of {} MB", size >> 20, MAX_FILE_BYTES >> 20);
    }
    if TEXT_EXT.contains(&ext.as_str()) {
        let text = read_text_any(input)?;
        let (new, counts) = red.redact(&text);
        if !fo.dry_run {
            write_atomic(output, new.as_bytes())?;
        }
        Ok(Outcome { counts, ..Default::default() })
    } else if ext == "docx" || ext == "xlsx" {
        redact_office(input, output, red, fo)
    } else if ext == "pdf" {
        redact_pdf(input, output, red, fo)
    } else if IMAGE_EXT.contains(&ext.as_str()) {
        redact_image(input, output, red, fo)
    } else {
        bail!("cannot redact .{ext} files")
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => std::path::absolute(a).ok() == std::path::absolute(b).ok(),
    }
}

/// Files to redact: a single file, or the matching files of a folder (`include`: patterns such as
/// "*.pdf;*.docx", empty = every supported file). Files inside `exclude_dir` are left out.
pub fn list_inputs(input: &Path, include: &str, recursive: bool, exclude_dir: Option<&Path>) -> Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    if !input.is_dir() {
        bail!("{} does not exist", input.display());
    }
    let pats: Vec<glob::Pattern> = include
        .split([';', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|p| glob::Pattern::new(&p.to_ascii_lowercase()).map_err(|e| anyhow!("bad pattern `{p}`: {e}")))
        .collect::<Result<_>>()?;
    let exclude = exclude_dir.and_then(|d| std::fs::canonicalize(d).ok());
    let mut out = vec![];
    let mut stack = vec![input.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let (Some(ex), Ok(c)) = (&exclude, std::fs::canonicalize(&dir)) {
            if &c == ex {
                continue;
            }
        }
        let mut entries: Vec<_> = std::fs::read_dir(&dir).with_context(|| format!("cannot list {}", dir.display()))?.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if recursive {
                    stack.push(p);
                }
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if name.starts_with("~$") {
                continue; // Office lock files
            }
            if pats.is_empty() || pats.iter().any(|g| g.matches(&name)) {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Name of the manifest kept in the output folder.
pub const MANIFEST: &str = ".autoflowmate-redaction.json";

/// What the manifest remembers for one output file: proof that it was made from this exact input
/// with these exact settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub input_sha256: String,
    pub output_sha256: String,
    pub policy: String,
    pub tool: String,
    pub at: String,
}

fn sha256_file(p: &Path) -> Result<String> {
    use sha2::Digest as _;
    let mut f = std::fs::File::open(p).with_context(|| format!("cannot read {}", p.display()))?;
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The settings that decide the result, as one hash.
fn policy_hash(red: &Redactor, fo: &FileOptions) -> String {
    use sha2::Digest as _;
    let text = format!("{}|{}|{:?}|{}|{}|{}|{}", env!("CARGO_PKG_VERSION"), red.policy, fo.ocr, fo.ocr_language, fo.ocr_rotated, fo.dpi, fo.clear_metadata);
    sha2::Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn manifest_path(input: &Path, output: &Path) -> PathBuf {
    if input.is_file() && output.extension().is_some() && !output.is_dir() {
        output.parent().map(|d| d.join(MANIFEST)).unwrap_or_else(|| PathBuf::from(MANIFEST))
    } else {
        output.join(MANIFEST)
    }
}

fn read_manifest(p: &Path) -> BTreeMap<String, ManifestEntry> {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("files").cloned())
        .and_then(|f| serde_json::from_value(f).ok())
        .unwrap_or_default()
}

fn write_manifest(p: &Path, files: &BTreeMap<String, ManifestEntry>) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let v = serde_json::json!({
        "about": "AutoFlowMate redaction manifest: which output was made from which input with which settings. An output is reused only when all three still match.",
        "files": files,
    });
    write_atomic(p, serde_json::to_string_pretty(&v)?.as_bytes())
}

/// Redact many files with several threads. `progress` is called (on this thread) after each file.
/// `stop` is checked between files. An output that already exists is reused only when the
/// manifest proves it was made from the same input with the same settings (status `unchanged`);
/// otherwise the file is redone. `overwrite` redoes every file.
#[allow(clippy::too_many_arguments)]
pub fn redact_many(
    input: &Path,
    files: &[PathBuf],
    output: &Path,
    red: &Redactor,
    fo: &FileOptions,
    overwrite: bool,
    threads: usize,
    stop: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(&FileResult),
) -> Vec<FileResult> {
    let single = input.is_file();
    let target = |f: &Path| -> PathBuf {
        if single {
            if output.extension().is_some() && !output.is_dir() {
                output.to_path_buf()
            } else {
                output.join(f.file_name().unwrap_or_default())
            }
        } else {
            output.join(f.strip_prefix(input).unwrap_or(f))
        }
    };
    let rel = |f: &Path| -> String {
        if single {
            f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            f.strip_prefix(input).unwrap_or(f).to_string_lossy().into_owned()
        }
    };
    let threads = if threads == 0 { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(6) } else { threads.clamp(1, 32) };
    let mpath = manifest_path(input, output);
    let mut manifest = read_manifest(&mpath);
    let known = manifest.clone();
    let policy = policy_hash(red, fo);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (tx, rx) = std::sync::mpsc::channel::<(FileResult, Option<ManifestEntry>)>();
    let mut results = Vec::with_capacity(files.len());
    std::thread::scope(|s| {
        for _ in 0..threads.min(files.len().max(1)) {
            let tx = tx.clone();
            let next = &next;
            let (target, rel, known, policy) = (&target, &rel, &known, &policy);
            s.spawn(move || loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if i >= files.len() || stop() {
                    break;
                }
                let f = &files[i];
                let out = target(f);
                let mut r = FileResult { file: rel(f), status: "failed", counts: Counts::new(), pages: 0, note: String::new() };
                let mut entry: Option<ManifestEntry> = None;
                let existed = out.exists();
                // Reuse an existing output only with proof: same input, same output, same settings.
                let proven = existed && !overwrite && {
                    let e = known.get(&r.file);
                    e.is_some_and(|e| {
                        e.policy == *policy && sha256_file(f).is_ok_and(|h| h == e.input_sha256) && sha256_file(&out).is_ok_and(|h| h == e.output_sha256)
                    })
                };
                if !supported(f) {
                    r.status = "skipped";
                    r.note = "file type not supported (not copied)".into();
                } else if proven {
                    r.status = "unchanged";
                    r.note = "already redacted from this input with the same settings".into();
                    entry = known.get(&r.file).cloned();
                } else {
                    match redact_file(f, &out, red, fo) {
                        Ok(o) => {
                            r.status = if o.unverified > 0 {
                                "unverified"
                            } else if o.counts.is_empty() {
                                "clean"
                            } else {
                                "redacted"
                            };
                            r.counts = o.counts;
                            r.pages = o.pages;
                            r.note = o.notes.join("; ");
                            if existed && !overwrite {
                                r.note = [r.note.as_str(), "the earlier output could not be proven current, so it was redone"]
                                    .iter()
                                    .filter(|s| !s.is_empty())
                                    .copied()
                                    .collect::<Vec<_>>()
                                    .join("; ");
                            }
                        }
                        Err(e) => r.note = format!("{e:#}"),
                    }
                    if !fo.dry_run {
                        if matches!(r.status, "redacted" | "clean") {
                            if let (Ok(i), Ok(o)) = (sha256_file(f), sha256_file(&out)) {
                                entry = Some(ManifestEntry {
                                    input_sha256: i,
                                    output_sha256: o,
                                    policy: policy.clone(),
                                    tool: env!("CARGO_PKG_VERSION").into(),
                                    at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                                });
                            }
                        } else if out.exists() {
                            // Not completely redacted: nothing of it may stay in the output folder,
                            // including an output from an earlier run.
                            let _ = std::fs::remove_file(&out);
                        }
                    }
                }
                if tx.send((r, entry)).is_err() {
                    break;
                }
            });
        }
        drop(tx);
        let mut since_save = 0;
        for (r, entry) in rx {
            progress(&r);
            if !fo.dry_run && r.status != "skipped" {
                match entry {
                    Some(e) => manifest.insert(r.file.clone(), e),
                    None => manifest.remove(&r.file),
                };
                since_save += 1;
                // Saved as it goes, so an interrupted run can continue.
                if since_save >= 25 {
                    let _ = write_manifest(&mpath, &manifest);
                    since_save = 0;
                }
            }
            results.push(r);
        }
    });
    if !fo.dry_run && !results.is_empty() {
        let _ = write_manifest(&mpath, &manifest);
    }
    results.sort_by(|a, b| a.file.cmp(&b.file));
    results
}

/// Files in the output folder that this redaction cannot vouch for: not written (or proven
/// current) by this run, and not proven by the manifest. A later step that zips or sends the whole
/// folder would share them as they are, so they are reported with status `foreign`.
/// `ignore` lists files to leave out (such as the report). Only for folder outputs.
pub fn foreign_outputs(input: &Path, output: &Path, results: &[FileResult], ignore: &[PathBuf]) -> Vec<FileResult> {
    if input.is_file() && output.extension().is_some() && !output.is_dir() {
        return vec![]; // a single named output file: there is no folder of ours to vouch for
    }
    if !output.is_dir() {
        return vec![];
    }
    let manifest = read_manifest(&output.join(MANIFEST));
    let ours: std::collections::BTreeSet<PathBuf> = results
        .iter()
        .filter(|r| matches!(r.status, "redacted" | "clean" | "unchanged"))
        .map(|r| output.join(&r.file))
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect();
    let ignore: std::collections::BTreeSet<PathBuf> = ignore.iter().filter_map(|p| std::fs::canonicalize(p).ok()).collect();
    let mut out = vec![];
    let mut stack = vec![output.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let rel = p.strip_prefix(output).unwrap_or(&p).to_string_lossy().into_owned();
            if rel == MANIFEST {
                continue;
            }
            let canon = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
            if ours.contains(&canon) || ignore.contains(&canon) {
                continue;
            }
            // An output from an earlier run (for example with a narrower include) still counts
            // when the manifest proves its content.
            if manifest.get(&rel).is_some_and(|e| sha256_file(&p).is_ok_and(|h| h == e.output_sha256)) {
                continue;
            }
            out.push(FileResult {
                file: rel,
                status: "foreign",
                counts: Counts::new(),
                pages: 0,
                note: "in the output folder but not made by this redaction (not checked): remove it, or it may be shared unredacted".into(),
            });
        }
    }
    out.sort_by(|a, b| a.file.cmp(&b.file));
    out
}

/// Write the report: one line per file (no personal data in it).
pub fn write_report(path: &Path, results: &[FileResult]) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut w = csv::Writer::from_path(path).with_context(|| format!("cannot write {}", path.display()))?;
    w.write_record(["file", "status", "found", "details", "pages", "note"])?;
    for r in results {
        let total: usize = r.counts.values().sum();
        w.write_record([r.file.as_str(), r.status, &total.to_string(), &counts_text(&r.counts), &r.pages.to_string(), r.note.as_str()])?;
    }
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red(rules: &[&str]) -> Redactor {
        Redactor::new(&Options { rules: rules.iter().map(|s| s.to_string()).collect(), strict: true, ..Default::default() }).unwrap()
    }

    /// A 12-digit number starting with 2-9 that passes the Verhoeff check.
    fn aadhaar_like() -> String {
        for last in 0..10u8 {
            let mut d: Vec<u8> = vec![2, 3, 4, 5, 6, 7, 8, 9, 1, 2, 3];
            d.push(last);
            if verhoeff_ok(&d) {
                return d.iter().map(|x| (b'0' + x) as char).collect();
            }
        }
        unreachable!()
    }

    #[test]
    fn checksums() {
        assert!(luhn_ok(&[4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]));
        assert!(!luhn_ok(&[4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2]));
        let a = aadhaar_like();
        let digits: Vec<u8> = a.bytes().map(|b| b - b'0').collect();
        assert!(verhoeff_ok(&digits));
    }

    #[test]
    fn indian_identifiers() {
        let a = aadhaar_like();
        let spaced = format!("{} {} {}", &a[0..4], &a[4..8], &a[8..12]);
        let text = format!("PAN ABCPE1234F, Aadhaar {spaced}, mobile +91 98765 43210, mail ravi.k@example.com, IFSC SBIN0001234, A/c No: 001234567890");
        let (out, counts) = red(&[]).redact(&text);
        assert!(!out.contains("ABCPE1234F") && !out.contains(&spaced) && !out.contains("98765") && !out.contains("ravi.k@"), "{out}");
        assert!(out.contains("[PAN]") && out.contains("[AADHAAR]") && out.contains("[MOBILE]") && out.contains("[EMAIL]") && out.contains("[IFSC]"), "{out}");
        assert!(out.contains("A/c No: [BANK_ACCOUNT]"), "{out}");
        assert_eq!(counts.get("PAN"), Some(&1));
    }

    #[test]
    fn aadhaar_needs_checksum() {
        let (out, counts) = red(&["aadhaar"]).redact("Order 2345 6789 0123 shipped");
        // 234567890123 fails the Verhoeff check (unless it happens to pass).
        let ok = verhoeff_ok(&[2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3]);
        assert_eq!(counts.is_empty(), !ok, "{out}");
    }

    #[test]
    fn labelled_values_keep_label() {
        let text = "Patient Name: Ravi Kumar  Age: 45 yrs  Sex: M\nDOB: 12/03/1979\nUHID: MH2024/00123\nDiagnosis: Type 2 diabetes\nS/o Mr. Suresh Kumar";
        let (out, _) = red(&[]).redact(text);
        assert!(out.contains("Patient Name: [NAME]"), "{out}");
        assert!(out.contains("Age: [AGE]") && out.contains("Sex: [GENDER]") && out.contains("DOB: [DOB]"), "{out}");
        assert!(out.contains("UHID: [PATIENT_ID]") && out.contains("Diagnosis: [HEALTH]"), "{out}");
        assert!(!out.contains("Suresh"), "{out}");
    }

    #[test]
    fn custom_patterns_words_and_styles() {
        let r = Redactor::new(&Options {
            rules: vec!["pan".into()],
            custom: vec![("Employee ID".into(), r"EMP-(?P<value>\d{5})".into())],
            words: vec!["Lakshmi Narayanan".into()],
            style: MaskStyle::Partial,
            strict: true,
            ..Default::default()
        })
        .unwrap();
        let (out, counts) = r.redact("lakshmi  narayanan, EMP-12345, ABCPE1234F");
        assert!(out.starts_with("XXXXXXX  XXXXXanan,"), "{out}");
        assert!(out.contains("EMP-X2345"), "{out}");
        assert!(out.contains("XXXXXX234F"), "{out}");
        assert_eq!(counts.get("EMPLOYEE_ID"), Some(&1));
        assert_eq!(counts.get("WORD"), Some(&1));
        assert!(Redactor::new(&Options { custom: vec![("x".into(), "(".into())], ..Default::default() }).is_err());
        assert!(Redactor::new(&Options { rules: vec!["nope".into()], ..Default::default() }).is_err());
        // "none": only your own patterns.
        let only = Redactor::new(&Options { rules: vec!["none".into()], custom: vec![("X".into(), r"EMP-\d+".into())], ..Default::default() }).unwrap();
        assert_eq!(only.redact("EMP-1 ABCPE1234F").0, "[X] ABCPE1234F");
    }

    #[test]
    fn word_runs_split_by_formatting() {
        let xml = r#"<w:document><w:body><w:p><w:r><w:t>PAN: ABCPE</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>1234F and more</w:t></w:r></w:p><w:p><w:r><w:t>clean</w:t></w:r></w:p></w:body></w:document>"#;
        let mut counts = Counts::new();
        let out = redact_xml(xml, &word_tokens(), &red(&["pan"]), &mut counts).unwrap();
        assert!(out.contains("<w:t>PAN: [PAN]</w:t>") && out.contains("<w:t> and more</w:t>"), "{out}");
        assert!(out.contains("<w:rPr><w:b/></w:rPr>") && out.contains("clean"));
        assert_eq!(counts.get("PAN"), Some(&1));
    }

    #[test]
    fn excel_numbers_and_strings() {
        let sheet = r#"<sheetData><row r="1"><c r="A1" s="1"><v>9876543210</v></c><c r="B1"><v>42</v></c><c r="C1" t="s"><v>0</v></c></row></sheetData>"#;
        let mut counts = Counts::new();
        let out = redact_numeric_cells(sheet, &red(&["mobile"]), &mut counts).unwrap();
        assert!(out.contains(r#"<c r="A1" s="1" t="inlineStr"><is><t>[MOBILE]</t></is></c>"#), "{out}");
        assert!(out.contains("<v>42</v>") && out.contains(r#"t="s"><v>0</v>"#));
        let shared = r#"<sst><si><t>ravi@example.com</t></si><si><r><t>Mob </t></r><r><t>98765 43210</t></r></si></sst>"#;
        let out = redact_xml(shared, &excel_tokens(), &red(&["email", "mobile"]), &mut counts).unwrap();
        assert!(out.contains("<t>[EMAIL]</t>") && out.contains("<t>[MOBILE]</t>"), "{out}");
    }

    #[test]
    fn ocr_misreads_are_still_found() {
        let r = red(&[]);
        // PAN with I read as 1, mobile with O read as 0... and a surname starting with l.
        let text = "Name: Kavya lyer   PAN: DEFP14567J   Mobile: 99887766S5";
        let spans = r.find_ocr(text);
        let kinds: Vec<&str> = spans.iter().map(|s| s.kind.as_str()).collect();
        assert!(kinds.contains(&"PAN") && kinds.contains(&"MOBILE") && kinds.contains(&"NAME"), "{kinds:?}");
        let name = spans.iter().find(|s| s.kind == "NAME").unwrap();
        assert_eq!(&text[name.start..name.end], "Kavya lyer");
        // Plain text is not mistaken for identifiers just because it has O, I, S letters.
        assert!(r.find_ocr("SOLID BOSSES GO TO BIG ISSUES").is_empty());
        assert!(r.find("DEFP14567J").is_empty(), "strict matching stays strict");
        assert_eq!(canonical("DEFP14567J", "AAAAA9999A").as_deref(), Some("DEFPI4567J"));
        assert_eq!(canonical("SOSOSOSOSO", "9999999999"), None, "mostly guessed digits");
    }

    #[test]
    fn overlapping_matches_cover_their_union() {
        // Two own patterns overlapping: 0..5 and 3..10. Nothing in 0..10 may stay visible.
        let r = Redactor::new(&Options {
            rules: vec!["none".into()],
            custom: vec![("A".into(), r"AB123".into()), ("B".into(), r"123CDEFGH".into())],
            ..Default::default()
        })
        .unwrap();
        let (out, counts) = r.redact("xx AB123CDEFGH yy");
        assert_eq!(out, "xx [B] yy");
        assert_eq!(counts.values().sum::<usize>(), 1);
        let spans = r.find("AB123CDEFGH");
        assert_eq!((spans[0].start, spans[0].end), (0, 11));
    }

    #[test]
    fn document_properties_and_authors() {
        let mut counts = Counts::new();
        let core = r#"<cp:coreProperties><dc:title>KYC of Ravi, PAN ABCPE1234F</dc:title><dc:creator>Ravi Kumar</dc:creator><cp:lastModifiedBy>Ravi Kumar</cp:lastModifiedBy></cp:coreProperties>"#;
        let out = clean_props(core, &red(&["pan"]), &mut counts);
        assert!(!out.contains("Ravi Kumar") && !out.contains("ABCPE1234F") && out.contains("[PAN]"), "{out}");
        let comments = r#"<w:comment w:id="1" w:author="Ravi Kumar" w:initials="RK"><w:p/></w:comment><authors><author>Lakshmi</author></authors>"#;
        let out = clear_author_attributes(comments);
        assert!(!out.contains("Ravi") && !out.contains("RK") && !out.contains("Lakshmi"), "{out}");
    }

    #[test]
    fn blank_pictures_are_recognised() {
        let white = image::RgbaImage::from_pixel(200, 100, image::Rgba([255, 255, 255, 255]));
        assert!(mostly_blank(&white));
        let mut text_like = white.clone();
        for y in 40..60 {
            for x in 20..180 {
                text_like.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        assert!(!mostly_blank(&text_like));
    }

    #[test]
    fn unreadable_picture_is_unverified_and_not_written() {
        // Random dark noise: OCR finds no text, and the picture is not blank.
        let mut img = image::RgbaImage::new(300, 200);
        let mut seed = 12345u32;
        for p in img.pixels_mut() {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let v = (seed >> 16) as u8;
            *p = image::Rgba([v, v / 2, v / 3, 255]);
        }
        let dir = std::env::temp_dir().join(format!("afm_redact_noise_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (inp, out) = (dir.join("noise.png"), dir.join("out.png"));
        img.save(&inp).unwrap();
        match redact_file(&inp, &out, &red(&[]), &FileOptions::default()) {
            Ok(o) => {
                assert_eq!(o.unverified, 1, "{:?}", o.notes);
                assert!(!out.exists(), "an unverified picture must not be written");
            }
            // No OCR available on this machine: an error is also fail-closed.
            Err(_) => assert!(!out.exists()),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_of_text_files() {
        let dir = std::env::temp_dir().join(format!("afm_redact_{}", std::process::id()));
        let (inp, out) = (dir.join("in"), dir.join("out"));
        std::fs::create_dir_all(inp.join("sub")).unwrap();
        std::fs::write(inp.join("a.txt"), "Mobile 9876543210").unwrap();
        std::fs::write(inp.join("sub").join("b.csv"), "name,pan\nx,ABCPE1234F\n").unwrap();
        std::fs::write(inp.join("c.txt"), "nothing here").unwrap();
        std::fs::write(inp.join("d.bin"), [0u8, 1, 2]).unwrap();
        let files = list_inputs(&inp, "", true, Some(&out)).unwrap();
        assert_eq!(files.len(), 4);
        let mut seen = 0;
        let res = redact_many(&inp, &files, &out, &red(&[]), &FileOptions::default(), false, 2, &|| false, &mut |_| seen += 1);
        assert_eq!(seen, 4);
        let st: Vec<_> = res.iter().map(|r| (r.file.replace('\\', "/"), r.status)).collect();
        assert!(st.contains(&("a.txt".into(), "redacted")) && st.contains(&("c.txt".into(), "clean")) && st.contains(&("d.bin".into(), "skipped")), "{st:?}");
        assert_eq!(std::fs::read_to_string(out.join("sub").join("b.csv")).unwrap(), "name,pan\nx,[PAN]\n");
        assert!(!out.join("d.bin").exists());
        assert!(redact_file(&inp.join("a.txt"), &inp.join("a.txt"), &red(&[]), &FileOptions::default()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
