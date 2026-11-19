//! Mouse and keyboard simulation (wraps the `enigo` crate).

use anyhow::{anyhow, bail, Result};
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::{thread::sleep, time::Duration};

use crate::actions::MouseButton;

/// Pause after pressing/before releasing modifiers so apps see them held.
const MOD_SETTLE: Duration = Duration::from_millis(15);

pub struct Input {
    enigo: Enigo,
}

impl Input {
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&Settings::default()).map_err(|e| anyhow!("input init failed: {e}"))?;
        Ok(Self { enigo })
    }

    pub fn position(&self) -> Result<(i32, i32)> {
        self.enigo.location().map_err(|e| anyhow!("{e}"))
    }

    pub fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        self.enigo.move_mouse(x, y, Coordinate::Abs).map_err(|e| anyhow!("{e}"))
    }

    pub fn move_by(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.enigo.move_mouse(dx, dy, Coordinate::Rel).map_err(|e| anyhow!("{e}"))
    }

    pub fn click(&mut self, button: MouseButton, clicks: u32) -> Result<()> {
        let b = map_button(button);
        for i in 0..clicks.max(1) {
            if i > 0 {
                sleep(Duration::from_millis(40));
            }
            self.enigo.button(b, Direction::Click).map_err(|e| anyhow!("{e}"))?;
        }
        Ok(())
    }

    pub fn drag(&mut self, from: (i32, i32), to: (i32, i32), button: MouseButton) -> Result<()> {
        let b = map_button(button);
        self.move_to(from.0, from.1)?;
        sleep(Duration::from_millis(50));
        self.enigo.button(b, Direction::Press).map_err(|e| anyhow!("{e}"))?;
        // Move in a few increments so applications register a drag.
        let steps = 10;
        for i in 1..=steps {
            let x = from.0 + (to.0 - from.0) * i / steps;
            let y = from.1 + (to.1 - from.1) * i / steps;
            self.move_to(x, y)?;
            sleep(Duration::from_millis(15));
        }
        self.enigo.button(b, Direction::Release).map_err(|e| anyhow!("{e}"))
    }

    pub fn scroll(&mut self, amount: i32, horizontal: bool) -> Result<()> {
        let axis = if horizontal { Axis::Horizontal } else { Axis::Vertical };
        self.enigo.scroll(amount, axis).map_err(|e| anyhow!("{e}"))
    }

    /// Type text. `keys` mode presses real virtual keys for characters on the
    /// current keyboard layout (most compatible) and falls back to Unicode
    /// injection for the rest; `unicode` injects everything as Unicode; `paste`
    /// goes through the clipboard (fastest for long text; clipboard is restored).
    pub fn type_text_mode(&mut self, text: &str, delay_ms: u64, mode: TypeMode) -> Result<()> {
        match mode {
            TypeMode::Paste => self.paste(text),
            TypeMode::Unicode if delay_ms == 0 => self.enigo.text(text).map_err(|e| anyhow!("{e}")),
            TypeMode::Unicode => {
                let mut buf = [0u8; 4];
                for c in text.chars() {
                    self.enigo.text(c.encode_utf8(&mut buf)).map_err(|e| anyhow!("{e}"))?;
                    sleep(Duration::from_millis(delay_ms));
                }
                Ok(())
            }
            TypeMode::Keys => {
                let delay = Duration::from_millis(delay_ms.max(5));
                let mut buf = [0u8; 4];
                for c in text.chars() {
                    if c == '\r' {
                        continue; // "\r\n" -> a single Enter
                    }
                    match vk_for_char(c) {
                        Some((vk, mods)) => {
                            let mod_keys: Vec<Key> = [(1, Key::Shift), (2, Key::Control), (4, Key::Alt)]
                                .into_iter()
                                .filter(|(bit, _)| mods & bit != 0)
                                .map(|(_, k)| k)
                                .collect();
                            for k in &mod_keys {
                                self.enigo.key(*k, Direction::Press).map_err(|e| anyhow!("{e}"))?;
                            }
                            if !mod_keys.is_empty() {
                                sleep(MOD_SETTLE);
                            }
                            let r = self.enigo.key(Key::Other(vk), Direction::Click).map_err(|e| anyhow!("{e}"));
                            if !mod_keys.is_empty() {
                                sleep(MOD_SETTLE);
                            }
                            for k in mod_keys.iter().rev() {
                                let _ = self.enigo.key(*k, Direction::Release);
                            }
                            r?;
                        }
                        None => self.enigo.text(c.encode_utf8(&mut buf)).map_err(|e| anyhow!("{e}"))?,
                    }
                    sleep(delay);
                }
                Ok(())
            }
        }
    }

    pub fn type_text(&mut self, text: &str, delay_ms: u64) -> Result<()> {
        self.type_text_mode(text, delay_ms, TypeMode::Keys)
    }

    fn paste(&mut self, text: &str) -> Result<()> {
        let mut cb = arboard::Clipboard::new()?;
        let previous = cb.get_text().ok();
        cb.set_text(text.to_string())?;
        sleep(Duration::from_millis(50));
        self.press_combo("ctrl+v")?;
        sleep(Duration::from_millis(150)); // let the target read the clipboard
        if let Some(p) = previous {
            let _ = cb.set_text(p);
        }
        Ok(())
    }

    /// Press a combination like `ctrl+shift+s`: modifiers down, last key clicked, modifiers up.
    pub fn press_combo(&mut self, combo: &str) -> Result<()> {
        let keys = parse_combo(combo)?;
        let (last, mods) = keys.split_last().expect("parse_combo returns at least one key");
        let mut pressed = Vec::new();
        let result = (|| -> Result<()> {
            for k in mods {
                self.enigo.key(*k, Direction::Press).map_err(|e| anyhow!("{e}"))?;
                pressed.push(*k);
                sleep(MOD_SETTLE);
            }
            self.enigo.key(*last, Direction::Click).map_err(|e| anyhow!("{e}"))
        })();
        if !pressed.is_empty() {
            sleep(MOD_SETTLE);
        }
        for k in pressed.iter().rev() {
            let _ = self.enigo.key(*k, Direction::Release);
        }
        // Give the target a moment to process the shortcut before the next input.
        sleep(Duration::from_millis(30));
        result
    }
}

pub use crate::actions::TypeMode;

/// Virtual-key code and modifier bits (1 shift, 2 ctrl, 4 alt) that produce `c`
/// on the active keyboard layout. Newlines map to Enter, tabs to Tab.
#[cfg(windows)]
fn vk_for_char(c: char) -> Option<(u32, u8)> {
    #[link(name = "user32")]
    extern "system" {
        fn VkKeyScanW(ch: u16) -> i16;
    }
    match c {
        '\n' => return Some((0x0D, 0)),
        '\t' => return Some((0x09, 0)),
        _ => {}
    }
    let mut units = [0u16; 2];
    if c.encode_utf16(&mut units).len() != 1 {
        return None;
    }
    let r = unsafe { VkKeyScanW(units[0]) };
    if r == -1 {
        return None;
    }
    let vk = (r & 0xFF) as u32;
    let mods = ((r >> 8) & 0xFF) as u8;
    // Ctrl+Alt combinations (AltGr) are unreliable to synthesize; use Unicode instead.
    if mods & 0b110 != 0 {
        return None;
    }
    Some((vk, mods))
}

#[cfg(not(windows))]
fn vk_for_char(_: char) -> Option<(u32, u8)> {
    None
}

fn map_button(b: MouseButton) -> Button {
    match b {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
    }
}

/// Parse `ctrl+shift+s` into keys. Names are case-insensitive.
pub fn parse_combo(combo: &str) -> Result<Vec<Key>> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        // allow "ctrl++" meaning ctrl and plus
        if combo.trim().ends_with("++") {
            let mut keys = parse_combo(combo.trim().trim_end_matches('+'))?;
            keys.push(parse_key("plus")?);
            return Ok(keys);
        }
        bail!("invalid key combination `{combo}`");
    }
    parts.into_iter().map(parse_key).collect()
}

pub fn parse_key(name: &str) -> Result<Key> {
    let lower = name.to_ascii_lowercase();
    let key = match lower.as_str() {
        "ctrl" | "control" => Key::Control,
        "shift" => Key::Shift,
        "alt" => Key::Alt,
        "win" | "windows" | "meta" | "super" | "cmd" => Key::Meta,
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" | "back" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "capslock" => Key::CapsLock,
        "printscreen" | "prtsc" => Key::PrintScr,
        "menu" | "apps" => Key::Apps,
        "volumeup" => Key::VolumeUp,
        "volumedown" => Key::VolumeDown,
        "volumemute" | "mute" => Key::VolumeMute,
        "plus" => Key::Unicode('+'),
        "minus" => Key::Unicode('-'),
        f if f.len() >= 2 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
            match f[1..].parse::<u8>().unwrap() {
                1 => Key::F1, 2 => Key::F2, 3 => Key::F3, 4 => Key::F4, 5 => Key::F5, 6 => Key::F6,
                7 => Key::F7, 8 => Key::F8, 9 => Key::F9, 10 => Key::F10, 11 => Key::F11, 12 => Key::F12,
                _ => bail!("unsupported function key `{name}`"),
            }
        }
        _ => {
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => char_key(c),
                _ => bail!("unknown key `{name}`"),
            }
        }
    };
    Ok(key)
}

/// Letters and digits map to virtual-key codes on Windows so shortcuts such
/// as ctrl+s work regardless of keyboard layout state.
fn char_key(c: char) -> Key {
    #[cfg(windows)]
    {
        if c.is_ascii_alphanumeric() {
            return Key::Other(c.to_ascii_uppercase() as u32);
        }
    }
    Key::Unicode(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos() {
        assert_eq!(parse_combo("ctrl+s").unwrap().len(), 2);
        assert_eq!(parse_combo("Ctrl+Shift+Esc").unwrap().len(), 3);
        assert!(parse_combo("f5").is_ok());
        assert!(parse_combo("ctrl+").is_err());
        assert!(parse_combo("nosuchkey").is_err());
    }
}
