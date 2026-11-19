//! Fail-safe (like PyAutoGUI's FAILSAFE): a background watcher aborts the run
//! when the mouse is pushed into a screen corner or the abort hotkey is held.
//! It also works while the engine is busy typing, dragging or waiting.

use anyhow::{bail, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Shared "why did we stop" message.
pub type Reason = Arc<Mutex<Option<String>>>;

pub struct Guard {
    done: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Start watching. Sets `stop` (and `reason`) when triggered.
pub fn start(stop: Arc<AtomicBool>, reason: Reason, corners: bool, hotkey: &str) -> Result<Guard> {
    let keys = if hotkey.trim().is_empty() { vec![] } else { parse_hotkey(hotkey)? };
    let done = Arc::new(AtomicBool::new(false));
    let d = done.clone();
    let label = hotkey.trim().to_string();
    let handle = std::thread::spawn(move || {
        // Ignore a hotkey that is already held when the run starts (e.g. the user's shortcut).
        let mut armed = keys.is_empty() || !all_down(&keys);
        // Likewise, a mouse resting in a corner at start only arms the fail-safe once it leaves.
        let mut corner_armed = cursor().map(|(x, y)| !corner_hit(x, y)).unwrap_or(true);
        while !d.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
            if corners {
                if let Some((x, y)) = cursor() {
                    let hit = corner_hit(x, y);
                    if hit && corner_armed {
                        trigger(&stop, &reason, format!("fail-safe triggered: mouse moved to a screen corner ({x}, {y})"));
                        break;
                    }
                    if !hit {
                        corner_armed = true;
                    }
                }
            }
            if !keys.is_empty() {
                let down = all_down(&keys);
                if armed && down {
                    trigger(&stop, &reason, format!("aborted with hotkey {label}"));
                    break;
                }
                if !down {
                    armed = true;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    Ok(Guard { done, handle: Some(handle) })
}

fn trigger(stop: &AtomicBool, reason: &Reason, msg: String) {
    if let Ok(mut r) = reason.lock() {
        r.get_or_insert(msg);
    }
    stop.store(true, Ordering::Relaxed);
}

/// True when (x, y) is a corner of the primary screen or the whole virtual desktop.
pub fn corner_hit(x: i32, y: i32) -> bool {
    let near = |a: i32, b: i32| (a - b).abs() <= 1;
    let (w, h) = screen_size();
    let (vx, vy, vw, vh) = virtual_screen();
    let rects = [(0, 0, w, h), (vx, vy, vw, vh)];
    rects.iter().any(|&(rx, ry, rw, rh)| {
        rw > 0 && rh > 0 && {
            let (l, t, r, b) = (rx, ry, rx + rw - 1, ry + rh - 1);
            (near(x, l) || near(x, r)) && (near(y, t) || near(y, b))
        }
    })
}

/// Parse `ctrl+alt+q` into Windows virtual-key codes.
pub fn parse_hotkey(s: &str) -> Result<Vec<i32>> {
    s.split('+')
        .map(|p| {
            let p = p.trim().to_ascii_lowercase();
            Ok(match p.as_str() {
                "ctrl" | "control" => 0x11,
                "alt" => 0x12,
                "shift" => 0x10,
                "win" | "windows" => 0x5B,
                "esc" | "escape" => 0x1B,
                "pause" | "break" => 0x13,
                "end" => 0x23,
                "home" => 0x24,
                "insert" | "ins" => 0x2D,
                "delete" | "del" => 0x2E,
                "space" => 0x20,
                "scrolllock" => 0x91,
                f if f.len() >= 2 && f.starts_with('f') && f[1..].parse::<u8>().map(|n| (1..=24).contains(&n)).unwrap_or(false) => {
                    0x70 + f[1..].parse::<i32>().unwrap() - 1
                }
                c if c.len() == 1 && c.chars().all(|c| c.is_ascii_alphanumeric()) => c.to_ascii_uppercase().as_bytes()[0] as i32,
                _ => bail!("unknown key `{p}` in abort hotkey `{s}`"),
            })
        })
        .collect()
}

#[cfg(windows)]
mod sys {
    #[repr(C)]
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        pub fn GetCursorPos(p: *mut Point) -> i32;
        pub fn GetSystemMetrics(index: i32) -> i32;
        pub fn GetAsyncKeyState(vk: i32) -> i16;
    }
}

#[cfg(windows)]
pub fn cursor() -> Option<(i32, i32)> {
    let mut p = sys::Point { x: 0, y: 0 };
    (unsafe { sys::GetCursorPos(&mut p) } != 0).then_some((p.x, p.y))
}

#[cfg(windows)]
pub fn screen_size() -> (i32, i32) {
    unsafe { (sys::GetSystemMetrics(0), sys::GetSystemMetrics(1)) }
}

#[cfg(windows)]
fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe { (sys::GetSystemMetrics(76), sys::GetSystemMetrics(77), sys::GetSystemMetrics(78), sys::GetSystemMetrics(79)) }
}

#[cfg(windows)]
fn all_down(keys: &[i32]) -> bool {
    keys.iter().all(|&k| unsafe { sys::GetAsyncKeyState(k) } as u16 & 0x8000 != 0)
}

#[cfg(not(windows))]
pub fn cursor() -> Option<(i32, i32)> {
    None
}
#[cfg(not(windows))]
pub fn screen_size() -> (i32, i32) {
    (0, 0)
}
#[cfg(not(windows))]
fn virtual_screen() -> (i32, i32, i32, i32) {
    (0, 0, 0, 0)
}
#[cfg(not(windows))]
fn all_down(_: &[i32]) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys() {
        assert_eq!(parse_hotkey("ctrl+alt+q").unwrap(), vec![0x11, 0x12, 0x51]);
        assert_eq!(parse_hotkey("F12").unwrap(), vec![0x7B]);
        assert!(parse_hotkey("ctrl+nope").is_err());
    }
}
