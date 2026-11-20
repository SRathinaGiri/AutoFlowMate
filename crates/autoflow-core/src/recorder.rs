//! Recorder: turn what the user does with the mouse and keyboard into flow steps.
//!
//! Low-level Windows hooks report clicks, wheel turns and keys. Each click is looked up with
//! UI Automation, so it becomes a `ui_click` on a named control when the app exposes one
//! (robust against moving windows), else a `mouse_click` at the position. Typing becomes
//! `type_text`, shortcuts `key_press`, switching windows `window_activate`, and pauses `wait`.
//! Keys typed into a password field are never recorded: the flow asks for the password at run
//! time (`input_box` with `password: true`). Ctrl+Alt+R stops the recording.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::flow::Step;

#[derive(Debug, Clone)]
pub struct RecordOptions {
    /// Insert `wait` steps for pauses longer than this (0 = no waits).
    pub min_wait_ms: u64,
    /// Ignore clicks and keys in windows whose title contains this (the recorder's own window).
    pub ignore_window: String,
}
impl Default for RecordOptions {
    fn default() -> Self {
        Self { min_wait_ms: 1500, ignore_window: "AutoFlowMate".into() }
    }
}

/// A raw input event from the hooks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Raw {
    MouseDown { button: u8, x: i32, y: i32 },
    MouseUp { button: u8, x: i32, y: i32 },
    Wheel { delta: i32, horizontal: bool, x: i32, y: i32 },
    Key { vk: u32, scan: u32, down: bool },
}

/// An element under the mouse: (window title, automation id, name, control type).
pub type Element = (String, String, String, String);

/// What the converter needs to know about the screen at an event (looked up on Windows,
/// supplied directly in tests).
pub trait Probe {
    /// Element under the point: (window title, automation id, name, control type).
    fn element_at(&mut self, x: i32, y: i32) -> Option<Element>;
    /// Title of the foreground window.
    fn foreground_title(&mut self) -> String;
    /// Whether the focused control is a password box.
    fn focused_is_password(&mut self) -> bool;
    /// Text a key produces with the current shift / caps state (None for non-character keys).
    fn key_text(&mut self, vk: u32, scan: u32, shift: bool) -> Option<String>;
}

/// Key names understood by `key_press`, by virtual-key code.
pub fn key_name(vk: u32) -> Option<String> {
    Some(
        match vk {
            0x08 => "backspace",
            0x09 => "tab",
            0x0D => "enter",
            0x1B => "esc",
            0x20 => "space",
            0x21 => "pageup",
            0x22 => "pagedown",
            0x23 => "end",
            0x24 => "home",
            0x25 => "left",
            0x26 => "up",
            0x27 => "right",
            0x28 => "down",
            0x2C => "printscreen",
            0x2D => "insert",
            0x2E => "delete",
            0x5D => "apps",
            0x70..=0x7B => return Some(format!("f{}", vk - 0x6F)),
            0x30..=0x39 | 0x41..=0x5A => return Some(((vk as u8) as char).to_ascii_lowercase().to_string()),
            0x60..=0x69 => return Some(((b'0' + (vk - 0x60) as u8) as char).to_string()),
            0xBA => ";",
            0xBB => "=",
            0xBC => ",",
            0xBD => "-",
            0xBE => ".",
            0xBF => "/",
            0xC0 => "`",
            0xDB => "[",
            0xDC => "\\",
            0xDD => "]",
            0xDE => "'",
            _ => return None,
        }
        .to_string(),
    )
}

fn is_modifier(vk: u32) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5 | 0x14)
}

/// Turns raw events into steps.
pub struct Converter<P: Probe> {
    probe: P,
    opts: RecordOptions,
    steps: Vec<Value>,
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
    text: String,
    password: bool,
    passwords: u32,
    window: String,
    down: Option<(u8, i32, i32, Instant, Option<Element>)>,
    last_click: Option<(u8, i32, i32, Instant)>,
    last_event: Option<Instant>,
    /// Set when Ctrl+Alt+R is pressed.
    pub stopped: bool,
}

impl<P: Probe> Converter<P> {
    pub fn new(probe: P, opts: RecordOptions) -> Self {
        Self {
            probe,
            opts,
            steps: vec![],
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            text: String::new(),
            password: false,
            passwords: 0,
            window: String::new(),
            down: None,
            last_click: None,
            last_event: None,
            stopped: false,
        }
    }

    /// Clicks / keys that are not part of the task: AutoFlowMate's own windows (Studio, the
    /// recording window) and the taskbar used to switch to the app.
    fn ignored(&self, title: &str) -> bool {
        let t = title.trim().to_lowercase();
        (!self.opts.ignore_window.is_empty() && t.contains(&self.opts.ignore_window.to_lowercase())) || t == "taskbar"
    }

    /// A `wait` for the pause since the last action, if long enough.
    fn pause(&mut self, now: Instant) {
        if let Some(last) = self.last_event {
            let ms = now.duration_since(last).as_millis() as u64;
            if self.opts.min_wait_ms > 0 && ms >= self.opts.min_wait_ms {
                let rounded = (ms / 100 * 100).min(15_000);
                let mut w = json!({"action": "wait", "ms": rounded});
                if ms > 15_000 {
                    w["comment"] = json!(format!("recorded pause was {} s", ms / 1000));
                }
                self.steps.push(w);
            }
        }
        self.last_event = Some(now);
    }

    fn push(&mut self, step: Value) {
        self.steps.push(step);
    }

    fn switch_window(&mut self, title: &str) {
        if !title.is_empty() && title != self.window {
            self.window = title.to_string();
            self.push(json!({"action": "window_activate", "title": title}));
        }
    }

    /// Typed text so far becomes a type_text step (or a password prompt).
    fn flush(&mut self) {
        if self.text.is_empty() && !self.password {
            return;
        }
        if self.password {
            self.passwords += 1;
            let var = if self.passwords == 1 { "password".to_string() } else { format!("password{}", self.passwords) };
            let window = if self.window.is_empty() { "the app".to_string() } else { self.window.clone() };
            self.push(json!({"action": "input_box", "title": "Password", "prompt": format!("Password for {window}"),
                             "password": true, "output_var": var, "comment": "recorded: the typed password itself is never saved"}));
            self.push(json!({"action": "type_text", "text": format!("${{{var}}}")}));
        } else {
            let text = std::mem::take(&mut self.text).replace("${", "$${");
            self.push(json!({"action": "type_text", "text": text}));
        }
        self.text.clear();
        self.password = false;
    }

    /// Add a key_press, merging repeats of the same keys into `times`.
    fn key(&mut self, keys: String) {
        if let Some(last) = self.steps.last_mut() {
            if last["action"] == "key_press" && last["keys"] == keys.as_str() {
                let n = last["times"].as_u64().unwrap_or(1) + 1;
                last["times"] = json!(n);
                return;
            }
        }
        self.push(json!({"action": "key_press", "keys": keys}));
    }

    pub fn feed(&mut self, ev: Raw, now: Instant) {
        match ev {
            Raw::Key { vk, scan, down } => {
                match vk {
                    0x11 | 0xA2 | 0xA3 => self.ctrl = down,
                    0x12 | 0xA4 | 0xA5 => self.alt = down,
                    0x10 | 0xA0 | 0xA1 => self.shift = down,
                    0x5B | 0x5C => self.win = down,
                    _ => {}
                }
                if !down || is_modifier(vk) {
                    return;
                }
                if self.ctrl && self.alt && vk == 0x52 {
                    self.stopped = true;
                    return;
                }
                let title = self.probe.foreground_title();
                if self.ignored(&title) {
                    return;
                }
                // AltGr (Ctrl+Alt) produces characters on many keyboard layouts.
                let altgr_char = if self.ctrl && self.alt && !self.win { self.probe.key_text(vk, scan, self.shift) } else { None };
                if (self.ctrl || self.alt || self.win) && altgr_char.is_none() {
                    self.flush();
                    self.pause(now);
                    self.switch_window(&title);
                    let mut parts = vec![];
                    if self.ctrl {
                        parts.push("ctrl".to_string());
                    }
                    if self.alt {
                        parts.push("alt".into());
                    }
                    if self.shift {
                        parts.push("shift".into());
                    }
                    if self.win {
                        parts.push("win".into());
                    }
                    let Some(name) = key_name(vk) else { return };
                    parts.push(name);
                    self.key(parts.join("+"));
                    return;
                }
                let special = matches!(vk, 0x09 | 0x0D | 0x1B | 0x21..=0x28 | 0x2C..=0x2E | 0x5D | 0x70..=0x7B);
                if vk == 0x08 && !self.text.is_empty() && !self.password {
                    self.text.pop();
                    return;
                }
                if special || vk == 0x08 {
                    self.flush();
                    self.pause(now);
                    self.switch_window(&title);
                    if let Some(name) = key_name(vk) {
                        self.key(if self.shift { format!("shift+{name}") } else { name });
                    }
                    return;
                }
                let Some(ch) = altgr_char.or_else(|| self.probe.key_text(vk, scan, self.shift)) else { return };
                if ch.chars().any(char::is_control) {
                    return;
                }
                if self.text.is_empty() && !self.password {
                    self.pause(now);
                    self.switch_window(&title);
                    self.password = self.probe.focused_is_password();
                }
                if !self.password {
                    self.text.push_str(&ch);
                }
            }
            Raw::MouseDown { button, x, y } => {
                let info = self.probe.element_at(x, y);
                if info.as_ref().is_some_and(|i| self.ignored(&i.0)) {
                    self.down = None;
                    return;
                }
                self.flush();
                self.down = Some((button, x, y, now, info));
            }
            Raw::MouseUp { button, x, y } => {
                let Some((b, dx, dy, at, info)) = self.down.take() else { return };
                if b != button {
                    return;
                }
                let moved = ((x - dx).pow(2) + (y - dy).pow(2)) as f64;
                if moved.sqrt() > 8.0 {
                    self.pause(at);
                    if let Some(i) = &info {
                        self.switch_window(&i.0);
                    }
                    self.push(json!({"action": "mouse_drag", "from_x": dx, "from_y": dy, "to_x": x, "to_y": y, "button": button_name(b), "duration_ms": 300}));
                    self.last_click = None;
                    return;
                }
                // A second click at the same place soon after the first makes it a double click.
                if let Some((lb, lx, ly, lt)) = self.last_click {
                    if lb == b && (lx - dx).abs() <= 6 && (ly - dy).abs() <= 6 && at.duration_since(lt) <= Duration::from_millis(500) {
                        if let Some(last) = self.steps.last_mut() {
                            match last["action"].as_str() {
                                Some("ui_click") => last["double"] = json!(true),
                                Some("mouse_click") => last["clicks"] = json!(2),
                                _ => {}
                            }
                        }
                        self.last_click = None;
                        return;
                    }
                }
                self.pause(at);
                self.last_click = Some((b, dx, dy, at));
                if let Some(i) = &info {
                    self.switch_window(&i.0);
                }
                self.push(click_step(b, dx, dy, info.as_ref()));
            }
            Raw::Wheel { delta, horizontal, x, y } => {
                let steps = delta / 120;
                if steps == 0 {
                    return;
                }
                if let Some(last) = self.steps.last_mut() {
                    if last["action"] == "mouse_scroll" && last["x"] == x && last["y"] == y && last["horizontal"] == horizontal {
                        let n = last["amount"].as_i64().unwrap_or(0) + steps as i64;
                        last["amount"] = json!(n);
                        return;
                    }
                }
                self.flush();
                self.pause(now);
                self.push(json!({"action": "mouse_scroll", "amount": steps, "horizontal": horizontal, "x": x, "y": y}));
            }
        }
    }

    /// The recorded steps (typing still in progress is included).
    pub fn finish(mut self) -> Result<Vec<Step>> {
        self.flush();
        self.steps.into_iter().map(|v| serde_json::from_value(v).map_err(|e| anyhow!("recorder made an invalid step: {e}"))).collect()
    }
}

fn button_name(b: u8) -> &'static str {
    match b {
        1 => "right",
        2 => "middle",
        _ => "left",
    }
}

/// Control types worth addressing by name (others are layout containers).
const CLICKABLE: &[&str] = &[
    "Button",
    "MenuItem",
    "Hyperlink",
    "CheckBox",
    "RadioButton",
    "TabItem",
    "ListItem",
    "TreeItem",
    "ComboBox",
    "Edit",
    "SplitButton",
    "DataItem",
    "Document",
    "Text",
    "Image",
];

fn click_step(b: u8, x: i32, y: i32, info: Option<&Element>) -> Value {
    if let Some((window, aid, name, ctype)) = info {
        let named = !aid.is_empty() || (!name.trim().is_empty() && name.chars().count() <= 80 && CLICKABLE.contains(&ctype.as_str()));
        if b == 0 && named {
            let mut s = json!({"action": "ui_click", "window": window, "control_type": ctype, "comment": format!("recorded at ({x}, {y})")});
            if aid.is_empty() {
                s["name"] = json!(name);
            } else {
                s["automation_id"] = json!(aid);
            }
            return s;
        }
        let what = if name.trim().is_empty() { ctype.clone() } else { format!("{ctype} \"{name}\"") };
        return json!({"action": "mouse_click", "x": x, "y": y, "button": button_name(b), "comment": format!("recorded on {what} in \"{window}\"")});
    }
    json!({"action": "mouse_click", "x": x, "y": y, "button": button_name(b)})
}

// --------------------------------------------------------------------- hooks

static SINK: Mutex<Option<Sender<Raw>>> = Mutex::new(None);
/// Also record input sent by programs (set from AFM_RECORD_INJECTED=1, for automated tests only).
static INCLUDE_INJECTED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
mod hooks {
    use super::*;
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const LLKHF_INJECTED: u32 = 0x10;
    const LLMHF_INJECTED: u32 = 0x01;

    fn send(r: Raw) {
        if let Ok(g) = SINK.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(r);
            }
        }
    }

    unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code >= 0 {
            let k = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
            // Keys sent by programs (including AutoFlowMate's own playback) are not recorded.
            if k.flags.0 & LLKHF_INJECTED == 0 || INCLUDE_INJECTED.load(Ordering::Relaxed) {
                let down = matches!(w.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                send(Raw::Key { vk: k.vkCode, scan: k.scanCode, down });
            }
        }
        unsafe { CallNextHookEx(None, code, w, l) }
    }

    unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code >= 0 {
            let m = unsafe { &*(l.0 as *const MSLLHOOKSTRUCT) };
            if m.flags & LLMHF_INJECTED == 0 || INCLUDE_INJECTED.load(Ordering::Relaxed) {
                let (x, y) = (m.pt.x, m.pt.y);
                let ev = match w.0 as u32 {
                    WM_LBUTTONDOWN => Some(Raw::MouseDown { button: 0, x, y }),
                    WM_RBUTTONDOWN => Some(Raw::MouseDown { button: 1, x, y }),
                    WM_MBUTTONDOWN => Some(Raw::MouseDown { button: 2, x, y }),
                    WM_LBUTTONUP => Some(Raw::MouseUp { button: 0, x, y }),
                    WM_RBUTTONUP => Some(Raw::MouseUp { button: 1, x, y }),
                    WM_MBUTTONUP => Some(Raw::MouseUp { button: 2, x, y }),
                    WM_MOUSEWHEEL => Some(Raw::Wheel { delta: ((m.mouseData >> 16) as i16) as i32, horizontal: false, x, y }),
                    WM_MOUSEHWHEEL => Some(Raw::Wheel { delta: ((m.mouseData >> 16) as i16) as i32, horizontal: true, x, y }),
                    _ => None,
                };
                if let Some(e) = ev {
                    send(e);
                }
            }
        }
        unsafe { CallNextHookEx(None, code, w, l) }
    }

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{GetStockObject, DEFAULT_GUI_FONT};

    /// Set by the Stop button of the recording window.
    static STOP_CLICKED: AtomicBool = AtomicBool::new(false);
    /// The step-count label of the recording window (as an integer handle).
    pub static LABEL: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
    const STOP_ID: usize = 1001;

    unsafe extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_COMMAND if (w.0 & 0xFFFF) == STOP_ID => {
                STOP_CLICKED.store(true, Ordering::Relaxed);
                LRESULT(0)
            }
            WM_CLOSE => {
                STOP_CLICKED.store(true, Ordering::Relaxed);
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(h, msg, w, l) },
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Show the current number of recorded steps in the recording window.
    pub fn set_label(text: &str) {
        let h = LABEL.load(Ordering::Relaxed);
        if h != 0 {
            let t = wide(text);
            unsafe {
                let _ = SetWindowTextW(HWND(h as *mut _), PCWSTR(t.as_ptr()));
            }
        }
    }

    /// A small always-on-top window (top-right of the screen) with a Stop button.
    unsafe fn stop_window(module: windows::Win32::Foundation::HMODULE) -> Option<HWND> {
        unsafe {
            let class = w!("AutoFlowMateRecorder");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: module.into(),
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let (sw, _) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
            let (w, h) = (380, 140);
            let win = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class,
                w!("AutoFlowMate recording"),
                WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                sw - w - 24,
                24,
                w,
                h,
                None,
                None,
                Some(module.into()),
                None,
            )
            .ok()?;
            let mut rc = RECT::default();
            let _ = GetClientRect(win, &mut rc);
            let font = GetStockObject(DEFAULT_GUI_FONT);
            let label = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("● Recording: 0 steps\r\nCtrl+Alt+R also stops"),
                WS_CHILD | WS_VISIBLE,
                12,
                8,
                rc.right - 24,
                36,
                Some(win),
                None,
                Some(module.into()),
                None,
            )
            .ok()?;
            let button = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("■  Stop recording"),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0000_0001), // BS_DEFPUSHBUTTON
                12,
                50,
                rc.right - 24,
                rc.bottom - 60,
                Some(win),
                Some(HMENU(STOP_ID as *mut _)),
                Some(module.into()),
                None,
            )
            .ok()?;
            for c in [label, button] {
                SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
            LABEL.store(label.0 as isize, Ordering::Relaxed);
            Some(win)
        }
    }

    /// The mouse pointer is a cross while recording; `restore` puts the user's pointers back.
    struct CrossCursor;
    impl CrossCursor {
        fn set() -> CrossCursor {
            unsafe {
                if let Ok(cross) = LoadCursorW(None, IDC_CROSS) {
                    if let Ok(copy) = CopyIcon(HICON(cross.0)) {
                        let _ = SetSystemCursor(HCURSOR(copy.0), OCR_NORMAL);
                    }
                }
            }
            CrossCursor
        }
    }
    impl Drop for CrossCursor {
        fn drop(&mut self) {
            unsafe {
                let _ = SystemParametersInfoW(SPI_SETCURSORS, 0, None, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
            }
        }
    }

    /// Install the hooks on this thread, show the recording window, and pump messages until
    /// `stop` is set or the window's Stop button is clicked.
    pub fn run(stop: Arc<AtomicBool>) -> Result<()> {
        STOP_CLICKED.store(false, Ordering::Relaxed);
        unsafe {
            let module = GetModuleHandleW(None).map_err(|e| anyhow!("{e}"))?;
            let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), Some(module.into()), 0).map_err(|e| anyhow!("cannot hook the keyboard: {e}"))?;
            let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), Some(module.into()), 0).map_err(|e| anyhow!("cannot hook the mouse: {e}"))?;
            let win = stop_window(module);
            let cursor = CrossCursor::set();
            let mut msg = MSG::default();
            while !stop.load(Ordering::Relaxed) {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if STOP_CLICKED.load(Ordering::Relaxed) {
                    stop.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            drop(cursor);
            LABEL.store(0, Ordering::Relaxed);
            if let Some(w) = win {
                let _ = DestroyWindow(w);
            }
            let _ = UnhookWindowsHookEx(kb);
            let _ = UnhookWindowsHookEx(ms);
        }
        Ok(())
    }
}

/// The real screen: UI Automation for elements, Win32 for windows and keyboard layout.
#[cfg(windows)]
pub struct WindowsProbe {
    uia: Option<crate::uia::Uia>,
}

#[cfg(windows)]
impl WindowsProbe {
    pub fn new() -> Self {
        Self { uia: crate::uia::Uia::new().ok() }
    }
}

#[cfg(windows)]
impl Default for WindowsProbe {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
impl Probe for WindowsProbe {
    fn element_at(&mut self, x: i32, y: i32) -> Option<Element> {
        let i = self.uia.as_ref()?.inspect_point(x, y).ok()?;
        Some((i.window, i.automation_id, i.name, i.control_type))
    }

    fn foreground_title(&mut self) -> String {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};
        unsafe {
            let h = GetForegroundWindow();
            let mut buf = [0u16; 512];
            let n = GetWindowTextW(h, &mut buf);
            String::from_utf16_lossy(&buf[..n.max(0) as usize])
        }
    }

    fn focused_is_password(&mut self) -> bool {
        self.uia.as_ref().map(|u| u.focused_is_password()).unwrap_or(false)
    }

    fn key_text(&mut self, vk: u32, scan: u32, shift: bool) -> Option<String> {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, GetKeyboardLayout, ToUnicodeEx};
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
        unsafe {
            let mut state = [0u8; 256];
            if shift {
                state[0x10] = 0x80;
            }
            if GetKeyState(0x14) & 1 != 0 {
                state[0x14] = 0x01;
            }
            let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let layout = GetKeyboardLayout(thread);
            let mut buf = [0u16; 8];
            // Flag 4: do not change the keyboard state (dead keys keep working for the user).
            let n = ToUnicodeEx(vk, scan, &state, &mut buf, 4, Some(layout));
            (n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
        }
    }
}

/// Record until Ctrl+Alt+R (or `stop` is set). `progress` gets a short note per new step.
#[cfg(windows)]
pub fn record(opts: RecordOptions, stop: Arc<AtomicBool>, progress: &mut dyn FnMut(&str)) -> Result<Vec<Step>> {
    INCLUDE_INJECTED.store(std::env::var("AFM_RECORD_INJECTED").is_ok_and(|v| v == "1"), Ordering::Relaxed);
    let (tx, rx): (Sender<Raw>, Receiver<Raw>) = channel();
    *SINK.lock().unwrap() = Some(tx);
    let hook_stop = stop.clone();
    let hook = std::thread::spawn(move || hooks::run(hook_stop));
    let mut conv = Converter::new(WindowsProbe::new(), opts);
    let mut shown = 0;
    while !stop.load(Ordering::Relaxed) && !conv.stopped {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(ev) => conv.feed(ev, Instant::now()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        if conv.steps.len() > shown {
            for s in &conv.steps[shown..] {
                progress(&summary(s));
            }
            shown = conv.steps.len();
            hooks::set_label(&format!("● Recording: {shown} step(s)\r\nCtrl+Alt+R also stops"));
        }
    }
    stop.store(true, Ordering::Relaxed);
    *SINK.lock().unwrap() = None;
    hook.join().map_err(|_| anyhow!("recorder thread failed"))??;
    conv.finish()
}

#[cfg(not(windows))]
pub fn record(_opts: RecordOptions, _stop: Arc<AtomicBool>, _progress: &mut dyn FnMut(&str)) -> Result<Vec<Step>> {
    anyhow::bail!("recording is only available on Windows")
}

fn summary(s: &Value) -> String {
    let a = s["action"].as_str().unwrap_or("");
    let detail = match a {
        "type_text" => s["text"].as_str().unwrap_or("").chars().take(40).collect::<String>(),
        "key_press" => s["keys"].as_str().unwrap_or("").to_string(),
        "ui_click" => format!("{} {}", s["control_type"].as_str().unwrap_or(""), s["name"].as_str().or(s["automation_id"].as_str()).unwrap_or("")),
        "window_activate" => s["title"].as_str().unwrap_or("").to_string(),
        "wait" => format!("{} ms", s["ms"]),
        _ => format!("{}, {}", s["x"], s["y"]),
    };
    format!("{a} {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        password_focus: bool,
    }
    impl Probe for Fake {
        fn element_at(&mut self, x: i32, _y: i32) -> Option<Element> {
            Some(match x {
                10 => ("Untitled - Notepad".into(), "".into(), "File".into(), "MenuItem".into()),
                20 => ("Login".into(), "pwd".into(), "Password".into(), "Edit".into()),
                30 => ("AutoFlowMate Studio".into(), "".into(), "Run".into(), "Button".into()),
                _ => ("Paint".into(), "".into(), "".into(), "Pane".into()),
            })
        }
        fn foreground_title(&mut self) -> String {
            "Untitled - Notepad".into()
        }
        fn focused_is_password(&mut self) -> bool {
            self.password_focus
        }
        fn key_text(&mut self, vk: u32, _scan: u32, shift: bool) -> Option<String> {
            let c = (vk as u8) as char;
            match vk {
                0x41..=0x5A => Some(if shift { c.to_string() } else { c.to_ascii_lowercase().to_string() }),
                0x20 => Some(" ".into()),
                0x30..=0x39 => Some(c.to_string()),
                _ => None,
            }
        }
    }

    fn keys(c: &mut Converter<Fake>, t: Instant, s: &str) {
        for ch in s.chars() {
            let up = ch.is_ascii_uppercase();
            let vk = ch.to_ascii_uppercase() as u32;
            if up {
                c.feed(Raw::Key { vk: 0x10, scan: 0, down: true }, t);
            }
            c.feed(Raw::Key { vk, scan: 0, down: true }, t);
            c.feed(Raw::Key { vk, scan: 0, down: false }, t);
            if up {
                c.feed(Raw::Key { vk: 0x10, scan: 0, down: false }, t);
            }
        }
    }

    fn actions(steps: &[Step]) -> Vec<String> {
        steps.iter().map(|s| serde_json::to_value(&s.action).unwrap()["action"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn clicks_typing_shortcuts_and_waits() {
        let mut c = Converter::new(Fake { password_focus: false }, RecordOptions::default());
        let t0 = Instant::now();
        c.feed(Raw::MouseDown { button: 0, x: 10, y: 5 }, t0);
        c.feed(Raw::MouseUp { button: 0, x: 10, y: 5 }, t0);
        keys(&mut c, t0 + Duration::from_millis(100), "Hi there");
        c.feed(Raw::Key { vk: 0x08, scan: 0, down: true }, t0); // backspace removes the last letter
        for _ in 0..3 {
            c.feed(Raw::Key { vk: 0x28, scan: 0, down: true }, t0 + Duration::from_secs(3));
        }
        c.feed(Raw::Key { vk: 0x11, scan: 0, down: true }, t0 + Duration::from_secs(3));
        c.feed(Raw::Key { vk: 0x53, scan: 0, down: true }, t0 + Duration::from_secs(3));
        c.feed(Raw::Key { vk: 0x11, scan: 0, down: false }, t0 + Duration::from_secs(3));
        // a click in Studio is ignored; a double click on a plain pane becomes mouse_click clicks 2
        c.feed(Raw::MouseDown { button: 0, x: 30, y: 5 }, t0 + Duration::from_secs(4));
        c.feed(Raw::MouseUp { button: 0, x: 30, y: 5 }, t0 + Duration::from_secs(4));
        let t5 = t0 + Duration::from_secs(5);
        for _ in 0..2 {
            c.feed(Raw::MouseDown { button: 0, x: 400, y: 300 }, t5);
            c.feed(Raw::MouseUp { button: 0, x: 400, y: 300 }, t5);
        }
        let steps = c.finish().unwrap();
        assert_eq!(actions(&steps), ["window_activate", "ui_click", "type_text", "wait", "key_press", "key_press", "wait", "window_activate", "mouse_click"]);
        let v: Vec<Value> = steps.iter().map(|s| serde_json::to_value(s).unwrap()).collect();
        assert_eq!(v[1]["name"], "File");
        assert_eq!(v[2]["text"], "Hi ther");
        assert_eq!(v[4]["keys"], "down");
        assert_eq!(v[4]["times"], 3);
        assert_eq!(v[5]["keys"], "ctrl+s");
        assert_eq!(v[7]["title"], "Paint");
        assert_eq!(v[8]["clicks"], 2);
    }

    #[test]
    fn password_is_never_recorded() {
        let mut c = Converter::new(Fake { password_focus: true }, RecordOptions { min_wait_ms: 0, ..RecordOptions::default() });
        let t = Instant::now();
        c.feed(Raw::MouseDown { button: 0, x: 20, y: 5 }, t);
        c.feed(Raw::MouseUp { button: 0, x: 20, y: 5 }, t);
        keys(&mut c, t, "Secret123");
        c.feed(Raw::Key { vk: 0x0D, scan: 0, down: true }, t);
        let steps = c.finish().unwrap();
        let json = serde_json::to_string(&steps).unwrap();
        assert!(!json.contains("Secret") && !json.contains("ecret123"), "{json}");
        assert!(json.contains("\"password\":true"));
        assert!(json.contains("${password}"));
        assert!(json.contains("\"automation_id\":\"pwd\""));
    }

    #[test]
    fn stop_hotkey() {
        let mut c = Converter::new(Fake { password_focus: false }, RecordOptions::default());
        let t = Instant::now();
        for vk in [0x11, 0x12, 0x52] {
            c.feed(Raw::Key { vk, scan: 0, down: true }, t);
        }
        assert!(c.stopped);
        assert!(c.finish().unwrap().is_empty(), "the stop keys are not recorded");
    }
}
