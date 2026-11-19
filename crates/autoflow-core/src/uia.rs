//! Windows UI Automation: finding windows and controls, reading/writing them,
//! and inspecting the element under the cursor.

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::time::{Duration, Instant};
use uiautomation::patterns::{UIInvokePattern, UITextPattern, UIValuePattern, UIWindowPattern};
use uiautomation::types::{ControlType, Point, PropertyConditionFlags, TreeScope, UIProperty, WindowVisualState};
use uiautomation::{UIAutomation, UIElement};

use crate::actions::{Selector, WindowState};

/// Returns true when the run was cancelled.
pub type Cancel<'a> = &'a dyn Fn() -> bool;

pub struct Uia {
    auto: UIAutomation,
}

impl Uia {
    pub fn new() -> Result<Self> {
        Ok(Self { auto: UIAutomation::new().map_err(|e| anyhow!("UI Automation init failed: {e}"))? })
    }

    /// Top-level windows whose title matches (no waiting). Dialogs owned by a
    /// top-level window (e.g. "Save as") are matched too.
    pub fn windows(&self, title: &str, exact: bool) -> Result<Vec<UIElement>> {
        let root = self.auto.get_root_element().map_err(|e| anyhow!("{e}"))?;
        let all = self.auto.create_true_condition().map_err(|e| anyhow!("{e}"))?;
        let kids = root.find_all(TreeScope::Children, &all).unwrap_or_default();
        let want = title.to_lowercase();
        let matches = |w: &UIElement| {
            let name = w.get_name().unwrap_or_default();
            if exact {
                name == title
            } else {
                name.to_lowercase().contains(&want)
            }
        };
        let mut out: Vec<UIElement> = kids.iter().filter(|w| matches(w)).cloned().collect();
        if out.is_empty() && !title.is_empty() {
            let is_window =
                self.auto.create_property_condition(UIProperty::ControlType, (ControlType::Window as i32).into(), None).map_err(|e| anyhow!("{e}"))?;
            for k in &kids {
                if let Ok(dialogs) = k.find_all(TreeScope::Children, &is_window) {
                    out.extend(dialogs.into_iter().filter(|d| matches(d)));
                }
            }
        }
        Ok(out)
    }

    pub fn wait_window(&self, title: &str, exact: bool, timeout_ms: u64, cancel: Cancel) -> Result<UIElement> {
        if title.trim().is_empty() {
            bail!("window title is empty");
        }
        let start = Instant::now();
        loop {
            if let Some(w) = self.windows(title, exact)?.into_iter().next() {
                return Ok(w);
            }
            if start.elapsed() >= Duration::from_millis(timeout_ms) {
                bail!("window \"{title}\" not found within {timeout_ms} ms");
            }
            if cancel() {
                bail!("cancelled");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn wait_window_closed(&self, title: &str, exact: bool, timeout_ms: u64, cancel: Cancel) -> Result<()> {
        let start = Instant::now();
        while !self.windows(title, exact)?.is_empty() {
            if start.elapsed() >= Duration::from_millis(timeout_ms) {
                bail!("window \"{title}\" still open after {timeout_ms} ms");
            }
            if cancel() {
                bail!("cancelled");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    }

    pub fn activate(&self, w: &UIElement) -> Result<()> {
        if let Ok(p) = w.get_pattern::<UIWindowPattern>() {
            if matches!(p.get_window_visual_state(), Ok(WindowVisualState::Minimized)) {
                let _ = p.set_window_visual_state(WindowVisualState::Normal);
            }
        }
        w.set_focus().map_err(|e| anyhow!("cannot focus window: {e}"))
    }

    pub fn close(&self, w: &UIElement) -> Result<()> {
        w.get_pattern::<UIWindowPattern>().and_then(|p| p.close()).map_err(|e| anyhow!("cannot close window: {e}"))
    }

    pub fn set_state(&self, w: &UIElement, state: WindowState) -> Result<()> {
        let s = match state {
            WindowState::Normal => WindowVisualState::Normal,
            WindowState::Maximized => WindowVisualState::Maximized,
            WindowState::Minimized => WindowVisualState::Minimized,
        };
        w.get_pattern::<UIWindowPattern>().and_then(|p| p.set_window_visual_state(s)).map_err(|e| anyhow!("cannot change window state: {e}"))
    }

    /// Find an element matching the selector, retrying until `timeout_ms`.
    pub fn find(&self, sel: &Selector, cancel: Cancel) -> Result<UIElement> {
        if sel.name.is_empty() && sel.automation_id.is_empty() && sel.class_name.is_empty() && sel.control_type.is_empty() {
            bail!("selector needs at least one of name, automation_id, class_name, control_type");
        }
        let start = Instant::now();
        let deadline = Duration::from_millis(sel.timeout_ms);
        let cond = self.condition(sel)?;
        loop {
            let scopes: Vec<UIElement> =
                if sel.window.is_empty() { vec![self.auto.get_root_element().map_err(|e| anyhow!("{e}"))?] } else { self.windows(&sel.window, false)? };
            for scope in &scopes {
                if let Ok(e) = scope.find_first(TreeScope::Descendants, &cond) {
                    return Ok(e);
                }
            }
            if start.elapsed() >= deadline {
                if !sel.window.is_empty() && scopes.is_empty() {
                    bail!("window \"{}\" not found", sel.window);
                }
                bail!("element not found: {}", describe(sel));
            }
            if cancel() {
                bail!("cancelled");
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn condition(&self, sel: &Selector) -> Result<uiautomation::core::UICondition> {
        let a = &self.auto;
        let mut conds = Vec::new();
        if !sel.name.is_empty() {
            let flags = if sel.name_contains { PropertyConditionFlags::All } else { PropertyConditionFlags::None };
            conds.push(a.create_property_condition(UIProperty::Name, sel.name.as_str().into(), Some(flags)));
        }
        if !sel.automation_id.is_empty() {
            conds.push(a.create_property_condition(UIProperty::AutomationId, sel.automation_id.as_str().into(), None));
        }
        if !sel.class_name.is_empty() {
            conds.push(a.create_property_condition(UIProperty::ClassName, sel.class_name.as_str().into(), None));
        }
        if !sel.control_type.is_empty() {
            let ct = parse_control_type(&sel.control_type)?;
            conds.push(a.create_property_condition(UIProperty::ControlType, (ct as i32).into(), None));
        }
        let mut iter = conds.into_iter();
        let mut cond = iter.next().expect("at least one condition").map_err(|e| anyhow!("{e}"))?;
        for c in iter {
            cond = a.create_and_condition(cond, c.map_err(|e| anyhow!("{e}"))?).map_err(|e| anyhow!("{e}"))?;
        }
        Ok(cond)
    }

    pub fn click(&self, e: &UIElement, invoke: bool, double: bool) -> Result<()> {
        if invoke && !double {
            if let Ok(p) = e.get_pattern::<UIInvokePattern>() {
                return p.invoke().map_err(|err| anyhow!("invoke failed: {err}"));
            }
        }
        let r = if double { e.double_click() } else { e.click() };
        r.map_err(|err| anyhow!("click failed: {err}"))
    }

    /// Set text through the Value pattern. Returns false when the element has no
    /// writable Value pattern (caller falls back to typing).
    pub fn set_value(&self, e: &UIElement, text: &str) -> Result<bool> {
        match e.get_pattern::<UIValuePattern>() {
            Ok(p) if !p.is_readonly().unwrap_or(true) => {
                p.set_value(text).map_err(|err| anyhow!("set value failed: {err}"))?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub fn get_text(&self, e: &UIElement) -> String {
        if let Ok(p) = e.get_pattern::<UIValuePattern>() {
            if let Ok(v) = p.get_value() {
                return v;
            }
        }
        if let Ok(p) = e.get_pattern::<UITextPattern>() {
            if let Ok(t) = p.get_document_range().and_then(|r| r.get_text(-1)) {
                return t;
            }
        }
        e.get_name().unwrap_or_default()
    }

    /// Describe the element at a screen position (for building selectors).
    /// Whether the control that has the keyboard focus is a password box.
    pub fn focused_is_password(&self) -> bool {
        self.auto.get_focused_element().ok().and_then(|e| e.is_password().ok()).unwrap_or(false)
    }

    pub fn inspect_point(&self, x: i32, y: i32) -> Result<ElementInfo> {
        let e = self.auto.element_from_point(Point::new(x, y)).map_err(|e| anyhow!("{e}"))?;
        let walker = self.auto.get_control_view_walker().map_err(|e| anyhow!("{e}"))?;
        let root = self.auto.get_root_element().map_err(|e| anyhow!("{e}"))?;
        // Walk up to the top-level window.
        let mut window = String::new();
        let mut cur = e.clone();
        for _ in 0..50 {
            match walker.get_parent(&cur) {
                Ok(p) => {
                    if self.auto.compare_elements(&p, &root).unwrap_or(false) {
                        window = cur.get_name().unwrap_or_default();
                        break;
                    }
                    cur = p;
                }
                Err(_) => break,
            }
        }
        let rect = e.get_bounding_rectangle().ok();
        Ok(ElementInfo {
            window,
            name: e.get_name().unwrap_or_default(),
            automation_id: e.get_automation_id().unwrap_or_default(),
            class_name: e.get_classname().unwrap_or_default(),
            control_type: e.get_control_type().map(|c| format!("{c:?}")).unwrap_or_default(),
            x,
            y,
            rect: rect.map(|r| [r.get_left(), r.get_top(), r.get_right(), r.get_bottom()]),
        })
    }
}

/// What `inspect` reports about an element.
#[derive(Debug, Clone, Serialize)]
pub struct ElementInfo {
    pub window: String,
    pub name: String,
    pub automation_id: String,
    pub class_name: String,
    pub control_type: String,
    pub x: i32,
    pub y: i32,
    /// [left, top, right, bottom]
    pub rect: Option<[i32; 4]>,
}

impl ElementInfo {
    /// A selector that should find this element again.
    pub fn to_selector(&self) -> Selector {
        Selector {
            window: self.window.clone(),
            name: if self.automation_id.is_empty() { self.name.clone() } else { String::new() },
            automation_id: self.automation_id.clone(),
            control_type: self.control_type.clone(),
            ..Selector::default()
        }
    }
}

fn describe(sel: &Selector) -> String {
    let mut parts = Vec::new();
    if !sel.window.is_empty() {
        parts.push(format!("window~\"{}\"", sel.window));
    }
    if !sel.name.is_empty() {
        parts.push(format!("name{}\"{}\"", if sel.name_contains { "~" } else { "=" }, sel.name));
    }
    if !sel.automation_id.is_empty() {
        parts.push(format!("automation_id=\"{}\"", sel.automation_id));
    }
    if !sel.class_name.is_empty() {
        parts.push(format!("class_name=\"{}\"", sel.class_name));
    }
    if !sel.control_type.is_empty() {
        parts.push(format!("control_type={}", sel.control_type));
    }
    parts.join(" ")
}

/// Parse a control type name (`Button`, `edit`, ...) by matching the enum's Debug names.
pub fn parse_control_type(name: &str) -> Result<ControlType> {
    let want = name.trim().to_ascii_lowercase();
    for id in 50000..=50040 {
        if let Ok(ct) = ControlType::try_from(id) {
            if format!("{ct:?}").to_ascii_lowercase() == want {
                return Ok(ct);
            }
        }
    }
    bail!("unknown control type `{name}` (examples: Button, Edit, Document, MenuItem, ListItem, CheckBox, ComboBox, TabItem, Window)")
}
