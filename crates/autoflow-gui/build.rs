// Embed the application icon and version info into autoflow-studio.exe, and collect the
// translatable texts (string literals passed to `tr`) for language templates.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/autoflowmate.ico");
    println!("cargo:rerun-if-changed=src");
    collect_ui_strings();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/autoflowmate.ico");
        res.set("ProductName", "AutoFlowMate");
        res.set("FileDescription", "AutoFlowMate Studio - visual RPA flow editor");
        res.set("InternalName", "autoflow-studio");
        res.set("OriginalFilename", "autoflow-studio.exe");
        res.set("Comments", "https://github.com/SRathinaGiri/AutoFlowMate");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
}

/// Every string literal passed to `tr(` in src/*.rs, written as a Rust array to OUT_DIR.
fn collect_ui_strings() {
    let mut found: Vec<String> = vec![];
    if let Ok(rd) = std::fs::read_dir("src") {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&p) else { continue };
            let bytes = src.as_bytes();
            let mut i = 0;
            while let Some(pos) = src[i..].find("tr(\"") {
                let start = i + pos;
                i = start + 4;
                // `tr(` must not be the end of a longer name (e.g. `attr(`).
                if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
                    continue;
                }
                let mut s = String::new();
                let mut chars = src[i..].char_indices();
                let mut end = None;
                while let Some((off, c)) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some((_, 'n')) => s.push('\n'),
                            Some((_, 't')) => s.push('\t'),
                            Some((_, other)) => s.push(other),
                            None => break,
                        },
                        '"' => {
                            end = Some(i + off + 1);
                            break;
                        }
                        other => s.push(other),
                    }
                }
                if let Some(e) = end {
                    i = e;
                    if !found.contains(&s) {
                        found.push(s);
                    }
                }
            }
        }
    }
    found.sort();
    let body: Vec<String> = found.iter().map(|s| format!("{s:?}")).collect();
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ui_strings.rs");
    std::fs::write(out, format!("[{}]", body.join(",\n"))).unwrap();
}
