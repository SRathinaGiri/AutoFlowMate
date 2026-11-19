//! Web automation through the Chrome DevTools Protocol (Edge or Chrome),
//! using the `headless_chrome` crate.

use anyhow::{anyhow, bail, Context as _, Result};
use headless_chrome::protocol::cdp::Page;
use headless_chrome::{Browser, Element, Tab};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// An open browser and its active tab.
pub struct WebSession {
    browser: Option<Browser>,
    process: Option<std::process::Child>,
    profile: PathBuf,
    pub tab: Arc<Tab>,
    /// Leave the browser window open when the run ends.
    pub keep_open: bool,
}

impl Drop for WebSession {
    fn drop(&mut self) {
        if !self.keep_open {
            // Ask the browser to close itself (works even when the launcher process
            // already exited, as Edge's does), then make sure the launcher is gone.
            let _ = self.tab.call_method(headless_chrome::protocol::cdp::Browser::Close(None));
            if let Some(mut p) = self.process.take() {
                let _ = p.kill();
                let _ = p.wait();
            }
        }
        drop(self.browser.take());
        if !self.keep_open {
            // Remove the temporary browser profile once the browser has let go of it.
            for _ in 0..25 {
                if std::fs::remove_dir_all(&self.profile).is_ok() || !self.profile.exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

/// A free local TCP port for the DevTools endpoint.
fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

/// Ask the browser for its DevTools WebSocket URL (GET /json/version).
fn devtools_ws_url(port: u16) -> Result<String> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500))?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    write!(s, "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")?;
    // The DevTools server keeps the connection open, so read headers and then
    // exactly Content-Length bytes instead of waiting for EOF.
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let body = loop {
        let n = s.read(&mut chunk)?;
        if n == 0 {
            break None;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf).to_string();
        if let Some(h) = text.find("\r\n\r\n") {
            let len = text[..h].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()));
            let body = &buf[h + 4..];
            match len {
                Some(len) if body.len() >= len => break Some(String::from_utf8_lossy(&body[..len]).to_string()),
                Some(_) => continue,
                None => continue,
            }
        }
    };
    let body = body.ok_or_else(|| anyhow!("bad DevTools response"))?;
    let v: Value = serde_json::from_str(body.trim())?;
    v.get("webSocketDebuggerUrl").and_then(Value::as_str).map(str::to_string).ok_or_else(|| anyhow!("DevTools response has no webSocketDebuggerUrl"))
}

/// Find the browser executable: "edge", "chrome" or a path.
pub fn browser_path(which: &str) -> Result<PathBuf> {
    let w = which.trim();
    let candidates: Vec<PathBuf> = match w.to_ascii_lowercase().as_str() {
        "" | "edge" | "msedge" => {
            vec![r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(), r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".into()]
        }
        "chrome" | "google chrome" => {
            let mut v: Vec<PathBuf> =
                vec![r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(), r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".into()];
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                v.push(Path::new(&local).join(r"Google\Chrome\Application\chrome.exe"));
            }
            v
        }
        _ => vec![PathBuf::from(w)],
    };
    candidates.into_iter().find(|p| p.exists()).ok_or_else(|| anyhow!("browser `{which}` not found (use \"edge\", \"chrome\" or the full path to the .exe)"))
}

/// Turn a local path into a file:// URL; leave real URLs alone.
pub fn to_url(target: &str, base: &Path) -> String {
    let t = target.trim();
    if t.contains("://") || t.starts_with("about:") || t.starts_with("data:") {
        return t.to_string();
    }
    let p = PathBuf::from(t);
    let p = if p.is_absolute() { p } else { base.join(p) };
    format!("file:///{}", p.display().to_string().replace('\\', "/").replace(' ', "%20"))
}

impl WebSession {
    /// Start the browser ourselves with a DevTools port, then attach to it.
    /// (Launching through headless_chrome waits for a "DevTools listening" line
    /// on stderr that Edge on Windows does not print.)
    pub fn open(which: &str, headless: bool, size: (u32, u32), keep_open: bool) -> Result<Self> {
        let path = browser_path(which)?;
        let port = free_port()?;
        // Separate profile so automation never touches the user's own browser profile,
        // and a fresh one per port so two runs never collide.
        let profile = std::env::temp_dir().join("autoflowmate-browser-profile").join(port.to_string());
        let mut cmd = std::process::Command::new(&path);
        cmd.arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("--window-size={},{}", size.0, size.1))
            .args(["--no-first-run", "--no-default-browser-check", "--disable-sync", "--disable-features=msEdgeSidebarV2,Translate", "--new-window"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if headless {
            cmd.arg("--headless=new");
        }
        cmd.arg("about:blank");
        let mut child = cmd.spawn().with_context(|| format!("could not start {}", path.display()))?;

        // Wait for the DevTools endpoint (normally < 2 s).
        let start = std::time::Instant::now();
        let ws = loop {
            match devtools_ws_url(port) {
                Ok(u) => break u,
                Err(_) if start.elapsed() < Duration::from_secs(20) => {
                    // Edge's msedge.exe is a launcher that exits with 0 after starting the
                    // real browser, so only a failing exit code is an error.
                    if let Ok(Some(status)) = child.try_wait() {
                        if !status.success() {
                            bail!("{} exited immediately ({status})", path.display());
                        }
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => {
                    let _ = child.kill();
                    bail!("browser did not open its DevTools port: {e}");
                }
            }
        };
        let browser = match Browser::connect_with_timeout(ws, Duration::from_secs(24 * 3600)) {
            Ok(b) => b,
            Err(e) => {
                let _ = child.kill();
                return Err(e).context("could not connect to the browser");
            }
        };
        let tab = browser.new_tab().context("could not open a tab")?;
        // Close the blank start tab(s) so only ours is visible.
        if let Ok(tabs) = browser.get_tabs().lock() {
            for t in tabs.iter() {
                if !Arc::ptr_eq(t, &tab) {
                    let _ = t.close(false);
                }
            }
        }
        let _ = tab.bring_to_front();
        Ok(Self { browser: Some(browser), process: Some(child), profile, tab, keep_open })
    }

    pub fn navigate(&self, url: &str, timeout_ms: u64) -> Result<()> {
        self.tab.set_default_timeout(Duration::from_millis(timeout_ms.max(1000)));
        self.tab.navigate_to(url)?.wait_until_navigated().with_context(|| format!("page did not finish loading: {url}"))?;
        Ok(())
    }

    pub fn wait(&self, css: &str, timeout_ms: u64) -> Result<Element<'_>> {
        self.tab
            .wait_for_element_with_custom_timeout(css, Duration::from_millis(timeout_ms))
            .map_err(|_| anyhow!("element {css} not found on the page within {timeout_ms} ms"))
    }

    pub fn exists(&self, css: &str) -> bool {
        self.tab.find_element(css).is_ok()
    }

    pub fn click(&self, css: &str, timeout_ms: u64) -> Result<()> {
        let e = self.wait(css, timeout_ms)?;
        e.scroll_into_view().ok();
        e.click().map_err(|err| anyhow!("cannot click {css}: {err}"))?;
        Ok(())
    }

    pub fn type_text(&self, css: &str, text: &str, clear: bool, enter: bool, timeout_ms: u64) -> Result<()> {
        let e = self.wait(css, timeout_ms)?;
        e.scroll_into_view().ok();
        if clear {
            e.call_js_fn("function() { if ('value' in this) { this.value = ''; this.dispatchEvent(new Event('input', {bubbles: true})); } }", vec![], false)?;
        }
        e.focus().map_err(|err| anyhow!("cannot focus {css}: {err}"))?;
        // Insert the text as one input (pages still receive beforeinput/input events);
        // much faster than one key event per character.
        self.tab
            .call_method(headless_chrome::protocol::cdp::Input::InsertText { text: text.to_string() })
            .map_err(|err| anyhow!("cannot type into {css}: {err}"))?;
        // Frameworks (Angular, React, …) often react to key and change events, not just input.
        e.call_js_fn(
            "function() { for (const t of ['keydown', 'keypress', 'keyup']) { this.dispatchEvent(new KeyboardEvent(t, {bubbles: true, key: 'Unidentified'})); } this.dispatchEvent(new Event('change', {bubbles: true})); }",
            vec![],
            false,
        )
        .ok();
        if enter {
            self.tab.press_key("Enter")?;
        }
        Ok(())
    }

    /// Visible text (`attribute` empty), the input value (`value`), the HTML (`html`)
    /// or any attribute.
    pub fn get(&self, css: &str, attribute: &str, timeout_ms: u64) -> Result<String> {
        let e = self.wait(css, timeout_ms)?;
        let a = attribute.trim();
        Ok(match a {
            "" | "text" => e.get_inner_text()?,
            "value" => {
                let r = e.call_js_fn("function() { return ('value' in this) ? String(this.value) : this.innerText; }", vec![], false)?;
                r.value.map(|v| crate::vars::to_text(&v)).unwrap_or_default()
            }
            "html" => e.get_content()?,
            name => e.get_attribute_value(name)?.unwrap_or_default(),
        })
    }

    pub fn screenshot(&self, path: &Path, css: Option<&str>, full_page: bool, timeout_ms: u64) -> Result<()> {
        let png = match css {
            Some(css) => self.wait(css, timeout_ms)?.capture_screenshot(Page::CaptureScreenshotFormatOption::Png)?,
            None if full_page => self.full_page_png()?,
            None => self.tab.capture_screenshot(Page::CaptureScreenshotFormatOption::Png, None, None, true)?,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, png).with_context(|| format!("save {}", path.display()))?;
        Ok(())
    }

    /// PNG of the whole document, including the parts scrolled out of view.
    fn full_page_png(&self) -> Result<Vec<u8>> {
        use base64::Engine as _;
        use headless_chrome::protocol::cdp::Emulation;
        // Height of everything: the document, or (for apps that scroll inside a
        // container instead of the page) the window plus the container's hidden part.
        let size = self.eval(
            r#"(function () {
                const de = document.documentElement, b = document.body;
                const docH = Math.max(de.scrollHeight, b ? b.scrollHeight : 0);
                let extra = 0;
                for (const el of document.querySelectorAll('body *')) {
                  const oy = getComputedStyle(el).overflowY;
                  if ((oy === 'auto' || oy === 'scroll') && el.scrollHeight > el.clientHeight + 10 && el.clientHeight > 100)
                    extra = Math.max(extra, el.scrollHeight - el.clientHeight);
                }
                return JSON.stringify([Math.max(de.scrollWidth, innerWidth), Math.max(docH, innerHeight + extra)]);
            })()"#,
        )?;
        // Objects/arrays only come back by value as JSON text.
        let size: Value = size.as_str().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(size);
        let num = |i: usize, d: f64| size.get(i).and_then(Value::as_f64).unwrap_or(d);
        let (w, h) = (num(0, 1280.0).min(16_000.0), num(1, 900.0).min(16_000.0));
        // Make the viewport as tall as the content so inner scroll containers expand too.
        self.tab.call_method(Emulation::SetDeviceMetricsOverride {
            width: w as u32,
            height: h as u32,
            device_scale_factor: 0.0,
            mobile: false,
            scale: None,
            screen_width: None,
            screen_height: None,
            position_x: None,
            position_y: None,
            dont_set_visible_size: None,
            screen_orientation: None,
            viewport: None,
            display_feature: None,
            device_posture: None,
        })?;
        std::thread::sleep(Duration::from_millis(400));
        let shot = self.tab.call_method(Page::CaptureScreenshot {
            format: Some(Page::CaptureScreenshotFormatOption::Png),
            quality: None,
            clip: Some(Page::Viewport { x: 0.0, y: 0.0, width: w, height: h, scale: 1.0 }),
            from_surface: Some(true),
            capture_beyond_viewport: Some(true),
            optimize_for_speed: None,
        });
        let _ = self.tab.call_method(Emulation::ClearDeviceMetricsOverride(None));
        Ok(base64::engine::general_purpose::STANDARD.decode(shot?.data)?)
    }

    /// Wait until `text` appears in the page (or in one element). Returns the
    /// element's / page's text at that moment.
    pub fn wait_text(&self, text: &str, css: Option<&str>, ignore_case: bool, timeout_ms: u64, cancel: &dyn Fn() -> bool) -> Result<String> {
        let target = match css {
            Some(c) => format!("document.querySelector({})", serde_json::to_string(c)?),
            None => "document.body".to_string(),
        };
        let script = format!("(function(){{ const e = {target}; return e ? (e.innerText || e.value || '') : ''; }})()");
        let want = if ignore_case { text.to_lowercase() } else { text.to_string() };
        let start = std::time::Instant::now();
        loop {
            let got = self.eval(&script).map(|v| crate::vars::to_text(&v)).unwrap_or_default();
            let hay = if ignore_case { got.to_lowercase() } else { got.clone() };
            if hay.contains(&want) {
                return Ok(got);
            }
            if start.elapsed() >= Duration::from_millis(timeout_ms) {
                bail!("text \"{text}\" did not appear{} within {timeout_ms} ms", css.map(|c| format!(" in {c}")).unwrap_or_default());
            }
            if cancel() {
                bail!("cancelled");
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    pub fn eval(&self, script: &str) -> Result<Value> {
        let r = self.tab.evaluate(script, true).context("script failed")?;
        Ok(r.value.unwrap_or(Value::Null))
    }

    pub fn url(&self) -> String {
        self.tab.get_url()
    }

    pub fn title(&self) -> String {
        self.tab.get_title().unwrap_or_default()
    }

    pub fn close(mut self) {
        self.keep_open = false;
    }
}

/// An element chosen with the web picker.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct PickedElement {
    /// The element's id attribute ("" when it has none).
    pub id: String,
    /// A CSS selector that finds it (uses the id when there is one).
    pub selector: String,
    pub tag: String,
    pub name: String,
    pub text: String,
    /// Page the element was picked on.
    pub url: String,
}

const PICKER_JS: &str = r#"(function () {
  if (window.__afmPick) return 'ok';
  window.__afmPick = { result: null };
  function cssPath(el) {
    if (el.id) return '#' + CSS.escape(el.id);
    const parts = [];
    while (el && el.nodeType === 1 && el !== document.documentElement) {
      if (el.id) { parts.unshift('#' + CSS.escape(el.id)); break; }
      let sel = el.tagName.toLowerCase();
      const same = el.parentElement ? Array.from(el.parentElement.children).filter(c => c.tagName === el.tagName) : [];
      if (same.length > 1) sel += ':nth-of-type(' + (same.indexOf(el) + 1) + ')';
      parts.unshift(sel);
      el = el.parentElement;
    }
    return parts.join(' > ');
  }
  function describe(el) {
    return { id: el.id || '', selector: cssPath(el), tag: el.tagName.toLowerCase(), name: el.getAttribute('name') || '',
             text: (el.innerText || el.value || el.alt || el.placeholder || '').trim().slice(0, 80), url: location.href };
  }
  const box = document.createElement('div');
  box.style.cssText = 'position:fixed;pointer-events:none;z-index:2147483647;border:2px solid #7c3aed;background:rgba(124,58,237,.12);border-radius:3px;display:none';
  const tip = document.createElement('div');
  tip.style.cssText = 'position:fixed;left:50%;top:8px;transform:translateX(-50%);z-index:2147483647;background:#3b2fb0;color:#fff;font:14px Segoe UI,sans-serif;padding:8px 14px;border-radius:8px;box-shadow:0 4px 14px rgba(0,0,0,.3);pointer-events:none;max-width:90vw;white-space:nowrap;overflow:hidden;text-overflow:ellipsis';
  tip.textContent = 'AutoFlowMate: hold Ctrl and click the element you want (normal clicks still work)';
  document.documentElement.appendChild(box); document.documentElement.appendChild(tip);
  document.addEventListener('mouseover', function (e) {
    const r = e.target.getBoundingClientRect();
    box.style.display = 'block'; box.style.left = r.left + 'px'; box.style.top = r.top + 'px';
    box.style.width = r.width + 'px'; box.style.height = r.height + 'px';
    const d = describe(e.target);
    tip.textContent = 'Ctrl+click to pick: <' + d.tag + '> ' + (d.id ? 'id="' + d.id + '"' : d.selector);
  }, true);
  document.addEventListener('click', function (e) {
    if (!e.ctrlKey) return;
    e.preventDefault(); e.stopPropagation();
    window.__afmPick.result = describe(e.target);
    tip.textContent = 'Picked - you can close this window';
    tip.style.background = '#15803d';
  }, true);
  return 'installed';
})()"#;

/// Open `url` in a visible browser and let the user Ctrl+click an element.
pub fn pick_element(url: &str, which: &str, timeout_ms: u64, cancel: &dyn Fn() -> bool) -> Result<PickedElement> {
    let session = WebSession::open(which, false, (1280, 900), false)?;
    if !url.trim().is_empty() {
        session.navigate(url, 60_000)?;
    }
    let start = std::time::Instant::now();
    loop {
        let state =
            session.eval("window.__afmPick ? (window.__afmPick.result ? JSON.stringify(window.__afmPick.result) : '') : 'NOINSTALL'").unwrap_or(Value::Null);
        match state.as_str() {
            Some("NOINSTALL") | None => {
                // New page (navigation) or not ready yet: (re)install the picker.
                let _ = session.eval(PICKER_JS);
            }
            Some("") => {}
            Some(json) => return Ok(serde_json::from_str(json)?),
        }
        if start.elapsed() >= Duration::from_millis(timeout_ms) {
            bail!("no element picked within {} s", timeout_ms / 1000);
        }
        if cancel() {
            bail!("cancelled");
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// CSS selector for an element given by id or selector.
pub fn css(id: &str, selector: &str) -> Result<String> {
    if !selector.trim().is_empty() {
        return Ok(selector.trim().to_string());
    }
    if id.trim().is_empty() {
        bail!("give the `element_id` (or a CSS `selector`)");
    }
    // Attribute form works for any id, including ones with dots or colons.
    Ok(format!("[id=\"{}\"]", id.trim().replace('\\', "\\\\").replace('"', "\\\"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_and_urls() {
        assert_eq!(css("email", "").unwrap(), "[id=\"email\"]");
        assert_eq!(css("a.b:c", "").unwrap(), "[id=\"a.b:c\"]");
        assert_eq!(css("x", "form > input").unwrap(), "form > input");
        assert!(css("", "").is_err());
        assert_eq!(to_url("https://example.com", Path::new("C:\\")), "https://example.com");
        assert_eq!(to_url("web\\form.html", Path::new("D:\\flows")), "file:///D:/flows/web/form.html");
    }
}
