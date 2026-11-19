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
            let len = text[..h]
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()));
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
    v.get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("DevTools response has no webSocketDebuggerUrl"))
}

/// Find the browser executable: "edge", "chrome" or a path.
pub fn browser_path(which: &str) -> Result<PathBuf> {
    let w = which.trim();
    let candidates: Vec<PathBuf> = match w.to_ascii_lowercase().as_str() {
        "" | "edge" | "msedge" => vec![
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".into(),
        ],
        "chrome" | "google chrome" => {
            let mut v: Vec<PathBuf> = vec![
                r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".into(),
            ];
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                v.push(Path::new(&local).join(r"Google\Chrome\Application\chrome.exe"));
            }
            v
        }
        _ => vec![PathBuf::from(w)],
    };
    candidates
        .into_iter()
        .find(|p| p.exists())
        .ok_or_else(|| anyhow!("browser `{which}` not found (use \"edge\", \"chrome\" or the full path to the .exe)"))
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
            e.call_js_fn(
                "function() { if ('value' in this) { this.value = ''; this.dispatchEvent(new Event('input', {bubbles: true})); } }",
                vec![],
                false,
            )?;
        }
        e.focus().map_err(|err| anyhow!("cannot focus {css}: {err}"))?;
        // Insert the text as one input (pages still receive beforeinput/input events);
        // much faster than one key event per character.
        self.tab
            .call_method(headless_chrome::protocol::cdp::Input::InsertText { text: text.to_string() })
            .map_err(|err| anyhow!("cannot type into {css}: {err}"))?;
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

    pub fn screenshot(&self, path: &Path, css: Option<&str>, timeout_ms: u64) -> Result<()> {
        let png = match css {
            Some(css) => self.wait(css, timeout_ms)?.capture_screenshot(Page::CaptureScreenshotFormatOption::Png)?,
            None => self.tab.capture_screenshot(Page::CaptureScreenshotFormatOption::Png, None, None, true)?,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, png).with_context(|| format!("save {}", path.display()))?;
        Ok(())
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
