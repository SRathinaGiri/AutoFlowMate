//! Action definitions. Every action is a plain struct with `#[serde(default)]`
//! so a template can be created from `{"action": "<name>"}` alone, which the
//! GUI palette and AI agents rely on.
//!
//! To add an action: add a struct below, register it in the `actions!` list,
//! and handle it in `engine::Context::exec_action`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::flow::Step;

/// An integer, or a string containing `${variables}` that evaluates to one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Int {
    Num(i64),
    Text(String),
}

impl Default for Int {
    fn default() -> Self {
        Int::Num(0)
    }
}

impl From<i64> for Int {
    fn from(v: i64) -> Self {
        Int::Num(v)
    }
}

// ---------------------------------------------------------------- flow control

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SetVariable {
    /// Variable name (without `${}`).
    pub name: String,
    /// Value. Strings are interpolated; a string that is exactly `${x}` copies
    /// x's value as-is (lists stay lists). Numbers, bools, arrays and objects are stored as given.
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Increment {
    /// Numeric variable to change (missing = 0).
    pub name: String,
    /// Amount to add (negative to subtract).
    pub by: Int,
}
impl Default for Increment {
    fn default() -> Self {
        Self { name: String::new(), by: Int::Num(1) }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    #[default]
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Log {
    /// Text to log (interpolated).
    pub message: String,
    /// info, warn or error.
    pub level: LogLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Wait {
    /// Milliseconds to wait.
    pub ms: Int,
}
impl Default for Wait {
    fn default() -> Self {
        Self { ms: Int::Num(1000) }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    #[default]
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    GreaterThan,
    LessThan,
    GreaterOrEqual,
    LessOrEqual,
    /// `left` is empty (`right` ignored).
    IsEmpty,
    /// `left` is not empty (`right` ignored).
    NotEmpty,
    /// `left` is "true", "yes" or "1" (`right` ignored).
    IsTrue,
    /// `left` is "false", "no", "0" or empty (`right` ignored).
    IsFalse,
    /// A file or folder exists at path `left` (`right` ignored).
    PathExists,
    /// `left` matches regex-free wildcard pattern `right` (`*` and `?`).
    Matches,
}

/// `left <op> right`, both sides interpolated. Numeric comparison is used when
/// both sides parse as numbers, otherwise text (case-sensitive unless `ignore_case`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Condition {
    /// Left-hand value, usually `${variable}`.
    pub left: String,
    /// Comparison operator.
    pub op: Op,
    /// Right-hand value (ignored by is_empty, not_empty, is_true, is_false, path_exists).
    pub right: String,
    /// Compare text case-insensitively.
    pub ignore_case: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct If {
    /// The test: {left, op, right}.
    pub condition: Condition,
    /// Steps run when the condition is true.
    pub then: Vec<Step>,
    /// Steps run when the condition is false.
    #[serde(rename = "else")]
    pub otherwise: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Repeat {
    /// Number of iterations.
    pub times: Int,
    /// Variable holding the 0-based iteration number.
    pub index_var: String,
    /// Steps run each iteration.
    pub steps: Vec<Step>,
}
impl Default for Repeat {
    fn default() -> Self {
        Self { times: Int::Num(3), index_var: "index".into(), steps: vec![] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct While {
    /// Loop while this is true (checked before each iteration).
    pub condition: Condition,
    /// Safety limit; the step fails when reached.
    pub max_iterations: u32,
    /// Steps run each iteration.
    pub steps: Vec<Step>,
}
impl Default for While {
    fn default() -> Self {
        Self { condition: Condition::default(), max_iterations: 1000, steps: vec![] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ForEach {
    /// A JSON array, or a string: `${list_var}` (an array variable) or text
    /// that is split into lines.
    pub items: Value,
    /// Variable holding the current item.
    pub item_var: String,
    /// Variable holding the 0-based position.
    pub index_var: String,
    /// Steps run for each item.
    pub steps: Vec<Step>,
}
impl Default for ForEach {
    fn default() -> Self {
        Self {
            items: Value::String(String::new()),
            item_var: "item".into(),
            index_var: "index".into(),
            steps: vec![],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Break {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Stop {
    /// Final message of the run.
    pub message: String,
    /// true = finish the flow successfully, false = fail it.
    pub success: bool,
}
impl Default for Stop {
    fn default() -> Self {
        Self { message: String::new(), success: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RunFlow {
    /// Path of another flow JSON file (relative paths resolve against this flow's folder).
    pub path: String,
    /// Variables passed to the sub-flow (values interpolated).
    pub variables: Map<String, Value>,
    /// If true, variables set by the sub-flow are copied back into this flow.
    pub share_variables: bool,
}

// ---------------------------------------------------------------------- files

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CopyMove {
    /// File (or folder for file_move) to copy/move; wildcards allowed (`C:\in\*.csv`).
    pub source: String,
    /// Destination file path, or a folder: an existing folder, or any path ending in `\` (created if missing).
    pub destination: String,
    /// Replace existing files instead of failing.
    pub overwrite: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FileDelete {
    /// File path; may contain wildcards (`C:\tmp\*.log`).
    pub path: String,
    /// Do not fail when nothing matches.
    pub missing_ok: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FileWrite {
    /// File to write; parent folders are created.
    pub path: String,
    /// Text to write (interpolated). Use ${newline} for line breaks.
    pub content: String,
    /// Append instead of overwriting.
    pub append: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PathToVar {
    /// File path (relative paths resolve against the flow folder).
    pub path: String,
    /// Variable that receives the result.
    pub output_var: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PathOnly {
    /// Folder path (relative paths resolve against the flow folder).
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DirDelete {
    /// Folder to delete.
    pub path: String,
    /// Delete contents too. Without it only empty folders are removed.
    pub recursive: bool,
    /// Do not fail when the folder does not exist.
    pub missing_ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DirList {
    /// Folder to list.
    pub path: String,
    /// Wildcard pattern, e.g. `*.txt`.
    pub pattern: String,
    /// Include sub-folders.
    pub recursive: bool,
    /// Include files in the result.
    pub include_files: bool,
    /// Include folders in the result.
    pub include_dirs: bool,
    /// Receives an array of full paths, sorted.
    pub output_var: String,
}
impl Default for DirList {
    fn default() -> Self {
        Self {
            path: String::new(),
            pattern: "*".into(),
            recursive: false,
            include_files: true,
            include_dirs: false,
            output_var: "files".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PathInfo {
    /// File or folder path.
    pub path: String,
    /// Receives {name, stem, extension, parent, size, is_dir, exists, modified}.
    pub output_var: String,
}

// -------------------------------------------------------------------- programs

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RunProgram {
    /// Executable path or name on PATH (e.g. `notepad.exe`).
    pub program: String,
    /// Command-line arguments, one per item (interpolated).
    pub args: Vec<String>,
    /// Folder to start in (empty = current).
    pub working_dir: String,
    /// Wait for the program to exit.
    pub wait: bool,
    /// When waiting: fail after this many ms (0 = no limit).
    pub timeout_ms: u64,
    /// When waiting: variable receiving stdout.
    pub output_var: String,
    /// When waiting: variable receiving the exit code.
    pub exit_code_var: String,
    /// When waiting: fail if the exit code is not 0.
    pub check_exit_code: bool,
}
impl Default for RunProgram {
    fn default() -> Self {
        Self {
            program: String::new(),
            args: vec![],
            working_dir: String::new(),
            wait: false,
            timeout_ms: 0,
            output_var: String::new(),
            exit_code_var: String::new(),
            check_exit_code: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct KillProgram {
    /// Process image name, e.g. `notepad.exe`.
    pub name: String,
    /// Do not fail when no such process is running.
    pub missing_ok: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Open {
    /// File, folder or URL to open with its default application.
    pub target: String,
}

// --------------------------------------------------------------------- windows

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WindowTarget {
    /// Window title; matches when the title contains this text (case-insensitive)
    /// unless `exact` is true.
    pub title: String,
    /// Require the whole title to match exactly.
    pub exact: bool,
    /// How long to wait for the window, in ms.
    pub timeout_ms: u64,
}
impl Default for WindowTarget {
    fn default() -> Self {
        Self { title: String::new(), exact: false, timeout_ms: 10_000 }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindowState {
    #[default]
    Normal,
    Maximized,
    Minimized,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WindowSetState {
    #[serde(flatten)]
    pub window: WindowTarget,
    /// normal, maximized or minimized.
    pub state: WindowState,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WindowExists {
    #[serde(flatten)]
    pub window: WindowTarget,
    /// Variable that receives "true" or "false".
    pub output_var: String,
}

// ----------------------------------------------------------- mouse & keyboard

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

/// Easing curve for mouse movement (PyAutoGUI "tween" functions).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tween {
    #[default]
    Linear,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInCubic,
    EaseOutCubic,
    EaseInOutCubic,
    EaseInOutSine,
    EaseOutBack,
    EaseOutBounce,
}

impl Tween {
    /// Map progress `t` in 0..=1 to eased progress.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Tween::Linear => t,
            Tween::EaseInQuad => t * t,
            Tween::EaseOutQuad => t * (2.0 - t),
            Tween::EaseInOutQuad => if t < 0.5 { 2.0 * t * t } else { -1.0 + (4.0 - 2.0 * t) * t },
            Tween::EaseInCubic => t * t * t,
            Tween::EaseOutCubic => { let u = t - 1.0; u * u * u + 1.0 }
            Tween::EaseInOutCubic => if t < 0.5 { 4.0 * t * t * t } else { let u = 2.0 * t - 2.0; 0.5 * u * u * u + 1.0 },
            Tween::EaseInOutSine => -(std::f64::consts::PI * t).cos() / 2.0 + 0.5,
            Tween::EaseOutBack => { let c1 = 1.70158; let c3 = c1 + 1.0; let u = t - 1.0; 1.0 + c3 * u * u * u + c1 * u * u }
            Tween::EaseOutBounce => {
                let (n1, d1) = (7.5625, 2.75);
                if t < 1.0 / d1 { n1 * t * t }
                else if t < 2.0 / d1 { let u = t - 1.5 / d1; n1 * u * u + 0.75 }
                else if t < 2.5 / d1 { let u = t - 2.25 / d1; n1 * u * u + 0.9375 }
                else { let u = t - 2.625 / d1; n1 * u * u + 0.984375 }
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MouseMove {
    /// Screen X in pixels (or offset when relative).
    pub x: Int,
    /// Screen Y in pixels (or offset when relative).
    pub y: Int,
    /// Move relative to the current position (PyAutoGUI moveRel).
    pub relative: bool,
    /// Movement time in ms; null = settings.mouse_move_ms (0 = instant).
    pub duration_ms: Option<u64>,
    /// Easing curve used when duration_ms > 0.
    pub tween: Tween,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MouseClick {
    /// Screen X; omit (null) to click at the current position.
    pub x: Option<Int>,
    /// Screen Y; omit (null) to click at the current position.
    pub y: Option<Int>,
    /// left, right or middle.
    pub button: MouseButton,
    /// 1 = single click, 2 = double click, 3 = triple click.
    pub clicks: u32,
    /// Pause between clicks in ms.
    pub interval_ms: u64,
    /// Time to move to (x, y) in ms; null = settings.mouse_move_ms.
    pub duration_ms: Option<u64>,
    /// Easing curve for the move.
    pub tween: Tween,
}
impl Default for MouseClick {
    fn default() -> Self {
        Self { x: None, y: None, button: MouseButton::Left, clicks: 1, interval_ms: 60, duration_ms: None, tween: Tween::Linear }
    }
}

/// Press (mouse_down) or release (mouse_up) a button, optionally at a position.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MouseButtonAt {
    /// Screen X; null = current position.
    pub x: Option<Int>,
    /// Screen Y; null = current position.
    pub y: Option<Int>,
    /// left, right or middle.
    pub button: MouseButton,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MouseScroll {
    /// Wheel notches; positive = down/right, negative = up/left.
    pub amount: Int,
    /// Scroll sideways instead of up/down (PyAutoGUI hscroll).
    pub horizontal: bool,
    /// Screen X to scroll at; null = current position.
    pub x: Option<Int>,
    /// Screen Y to scroll at; null = current position.
    pub y: Option<Int>,
}
impl Default for MouseScroll {
    fn default() -> Self {
        Self { amount: Int::Num(3), horizontal: false, x: None, y: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MouseDrag {
    /// Start X; null = current position.
    pub from_x: Option<Int>,
    /// Start Y; null = current position.
    pub from_y: Option<Int>,
    /// End X (or offset when relative).
    pub to_x: Int,
    /// End Y (or offset when relative).
    pub to_y: Int,
    /// Treat to_x/to_y as offsets from the start (PyAutoGUI dragRel).
    pub relative: bool,
    /// Button held during the drag.
    pub button: MouseButton,
    /// Drag time in ms (some apps need a slow drag to register it).
    pub duration_ms: u64,
    /// Easing curve for the drag.
    pub tween: Tween,
}
impl Default for MouseDrag {
    fn default() -> Self {
        Self {
            from_x: Some(Int::Num(0)),
            from_y: Some(Int::Num(0)),
            to_x: Int::Num(0),
            to_y: Int::Num(0),
            relative: false,
            button: MouseButton::Left,
            duration_ms: 500,
            tween: Tween::EaseInOutQuad,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MousePosition {
    /// Variable receiving X.
    pub x_var: String,
    /// Variable receiving Y.
    pub y_var: String,
}
impl Default for MousePosition {
    fn default() -> Self {
        Self { x_var: "mouse_x".into(), y_var: "mouse_y".into() }
    }
}

/// How `type_text` sends text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TypeMode {
    /// Press real keys per character (most compatible; default).
    #[default]
    Keys,
    /// Inject Unicode characters (any language, but some apps drop them).
    Unicode,
    /// Paste through the clipboard (fast for long text; clipboard is restored).
    Paste,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TypeText {
    /// Text to type (interpolated). `\n` presses Enter, `\t` presses Tab.
    pub text: String,
    /// Delay between keystrokes in ms; null = settings.key_delay_ms.
    pub delay_ms: Option<u64>,
    /// keys (default, most compatible), unicode, or paste (via clipboard).
    pub mode: TypeMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct KeyPress {
    /// Key combination, e.g. `enter`, `ctrl+s`, `ctrl+shift+esc`, `alt+f4`, `win+r`, `f5`, `a`.
    pub keys: String,
    /// How many times to press it.
    pub times: u32,
    /// Pause between presses in ms; null = settings.key_delay_ms.
    pub interval_ms: Option<u64>,
}
impl Default for KeyPress {
    fn default() -> Self {
        Self { keys: "enter".into(), times: 1, interval_ms: None }
    }
}

/// Hold (key_down) or release (key_up) keys, e.g. `shift` or `ctrl+alt`.
/// Keys still held at the end of the run are released automatically.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct KeyHold {
    /// Key or combination; key_down presses left to right, key_up releases right to left.
    pub keys: String,
}

// --------------------------------------------------------- UI Automation (UIA)

/// Finds a UI element through Windows UI Automation. At least one of
/// `name`, `automation_id`, `class_name`, `control_type` should be set.
/// Use `autoflow inspect` (or the GUI "Pick" button) to discover them.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Selector {
    /// Restrict the search to the top-level window whose title contains this.
    pub window: String,
    /// Element name (the text a screen reader would say).
    pub name: String,
    /// Match when the name contains `name` instead of equalling it.
    pub name_contains: bool,
    /// UIA AutomationId (most stable; prefer it when present).
    pub automation_id: String,
    /// Win32/UIA class name, e.g. `Edit`, `Button`.
    pub class_name: String,
    /// UIA control type, e.g. `Button`, `Edit`, `Document`, `MenuItem`, `ListItem`, `CheckBox`.
    pub control_type: String,
    /// How long to keep searching, in ms.
    pub timeout_ms: u64,
}
impl Default for Selector {
    fn default() -> Self {
        Self {
            window: String::new(),
            name: String::new(),
            name_contains: false,
            automation_id: String::new(),
            class_name: String::new(),
            control_type: String::new(),
            timeout_ms: 10_000,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UiClick {
    #[serde(flatten)]
    pub target: Selector,
    /// Prefer the UIA Invoke pattern (works on hidden/covered controls) and fall back to a real mouse click.
    pub invoke: bool,
    /// Double-click (always a real mouse double-click).
    pub double: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UiSetText {
    #[serde(flatten)]
    pub target: Selector,
    /// Text to put in the element (interpolated).
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UiToVar {
    #[serde(flatten)]
    pub target: Selector,
    /// Variable that receives the result.
    pub output_var: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UiWait {
    #[serde(flatten)]
    pub target: Selector,
}

// ------------------------------------------------------------ clipboard/screen

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ClipboardSet {
    /// Text to copy (interpolated).
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OutputVar {
    /// Variable that receives the result.
    pub output_var: String,
}

/// A screen rectangle in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Screenshot {
    /// PNG path to write.
    pub path: String,
    /// Monitor index (0 = primary).
    pub monitor: u32,
    /// Capture only the window whose title contains this text.
    pub window: String,
    /// Capture only this screen rectangle {x, y, width, height}; null = whole monitor.
    pub region: Option<Region>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ScreenSize {
    /// Variable receiving the primary screen width.
    pub width_var: String,
    /// Variable receiving the primary screen height.
    pub height_var: String,
}
impl Default for ScreenSize {
    fn default() -> Self {
        Self { width_var: "screen_width".into(), height_var: "screen_height".into() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PixelGet {
    /// Screen X.
    pub x: Int,
    /// Screen Y.
    pub y: Int,
    /// Variable receiving the colour as `#RRGGBB`.
    pub output_var: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PixelMatches {
    /// Screen X.
    pub x: Int,
    /// Screen Y.
    pub y: Int,
    /// Expected colour `#RRGGBB`.
    pub color: String,
    /// Allowed difference per channel (0-255).
    pub tolerance: u8,
    /// Variable receiving "true" or "false".
    pub output_var: String,
}
impl Default for PixelMatches {
    fn default() -> Self {
        Self { x: Int::Num(0), y: Int::Num(0), color: "#FFFFFF".into(), tolerance: 0, output_var: String::new() }
    }
}

/// Locate a picture on screen (PyAutoGUI locateOnScreen). The match is
/// tolerant to small rendering differences through `confidence`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ImageTarget {
    /// PNG/JPG file of what to look for (relative paths resolve against the flow folder).
    /// Capture it with Studio's "Snip image" button or Win+Shift+S.
    pub image: String,
    /// Required similarity 0.0-1.0 (1.0 = pixel-perfect). 0.9 suits most UI.
    pub confidence: f64,
    /// Search only this screen rectangle (much faster); null = all monitors.
    pub region: Option<Region>,
    /// Keep searching for up to this many ms.
    pub timeout_ms: u64,
}
impl Default for ImageTarget {
    fn default() -> Self {
        Self { image: String::new(), confidence: 0.9, region: None, timeout_ms: 5000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ImageFind {
    #[serde(flatten)]
    pub target: ImageTarget,
    /// Variable receiving "true"/"false" (the step never fails when not found).
    pub found_var: String,
    /// Variable receiving the centre X of the match.
    pub x_var: String,
    /// Variable receiving the centre Y of the match.
    pub y_var: String,
}
impl Default for ImageFind {
    fn default() -> Self {
        Self { target: ImageTarget { timeout_ms: 0, ..ImageTarget::default() }, found_var: "found".into(), x_var: "image_x".into(), y_var: "image_y".into() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ImageFindAll {
    #[serde(flatten)]
    pub target: ImageTarget,
    /// Variable receiving an array of matches [{x, y, left, top, width, height, score}].
    pub output_var: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ImageClick {
    #[serde(flatten)]
    pub target: ImageTarget,
    /// Click this many pixels right of the image centre (negative = left).
    pub offset_x: i32,
    /// Click this many pixels below the image centre (negative = above).
    pub offset_y: i32,
    /// left, right or middle.
    pub button: MouseButton,
    /// 1 = single, 2 = double click.
    pub clicks: u32,
}
impl Default for ImageClick {
    fn default() -> Self {
        Self { target: ImageTarget::default(), offset_x: 0, offset_y: 0, button: MouseButton::Left, clicks: 1 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ImageWait {
    #[serde(flatten)]
    pub target: ImageTarget,
}

// ---------------------------------------------------------------------- dialogs

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DialogButtons {
    #[default]
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
    RetryCancel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DialogIcon {
    #[default]
    Info,
    Warning,
    Error,
    Question,
    None,
}

/// A message box (PyAutoGUI alert / confirm). Blocks until the user answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MessageBox {
    /// Window title of the dialog.
    pub title: String,
    /// Message text (interpolated).
    pub text: String,
    /// ok, ok_cancel, yes_no, yes_no_cancel or retry_cancel.
    pub buttons: DialogButtons,
    /// info, warning, error, question or none.
    pub icon: DialogIcon,
    /// Variable receiving the clicked button: ok, cancel, yes, no, retry.
    pub output_var: String,
}
impl Default for MessageBox {
    fn default() -> Self {
        Self { title: "AutoFlowMate".into(), text: String::new(), buttons: DialogButtons::Ok, icon: DialogIcon::Info, output_var: "answer".into() }
    }
}

/// Ask the user to type a value (PyAutoGUI prompt / password).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct InputBox {
    /// Window title of the dialog.
    pub title: String,
    /// Question shown above the text box (interpolated).
    pub prompt: String,
    /// Initial text.
    pub default: String,
    /// Mask the input (for passwords). The value is never written to the log.
    pub password: bool,
    /// Variable receiving the text.
    pub output_var: String,
    /// Fail the step when the user presses Cancel (otherwise the variable is set to "").
    pub fail_on_cancel: bool,
}
impl Default for InputBox {
    fn default() -> Self {
        Self { title: "AutoFlowMate".into(), prompt: String::new(), default: String::new(), password: false, output_var: "input".into(), fail_on_cancel: true }
    }
}

// ---------------------------------------------------------------- registration

/// Metadata about an action, for the editor palette, `autoflow actions` and docs.
#[derive(Debug, Clone, Serialize)]
pub struct ActionInfo {
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    /// The action with default parameters, as JSON.
    pub template: Value,
}

macro_rules! actions {
    ($( $cat:literal, $name:literal, $Variant:ident($Ty:ty), $desc:literal; )*) => {
        /// The action performed by a step. Selected by the `"action"` field.
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(tag = "action")]
        pub enum Action {
            $( #[doc = $desc] #[serde(rename = $name)] $Variant($Ty), )*
        }

        impl Action {
            pub fn name(&self) -> &'static str {
                match self { $( Action::$Variant(_) => $name, )* }
            }
            pub fn category(&self) -> &'static str {
                match self { $( Action::$Variant(_) => $cat, )* }
            }
            /// A new action of the given name with default parameters.
            pub fn template(name: &str) -> Option<Action> {
                match name { $( $name => Some(Action::$Variant(<$Ty>::default())), )* _ => None }
            }
        }

        pub fn catalog() -> Vec<ActionInfo> {
            vec![ $( ActionInfo {
                name: $name,
                category: $cat,
                description: $desc,
                template: serde_json::to_value(Action::$Variant(<$Ty>::default())).unwrap(),
            }, )* ]
        }
    };
}

actions! {
    "Flow", "set_variable", SetVariable(SetVariable), "Set a variable (strings are interpolated).";
    "Flow", "increment", Increment(Increment), "Add a number to a numeric variable (counters).";
    "Flow", "log", Log(Log),"Write a message to the run log.";
    "Flow", "wait", Wait(Wait), "Pause for a number of milliseconds.";
    "Flow", "if", If(If), "Run `then` steps if the condition holds, otherwise `else` steps.";
    "Flow", "repeat", Repeat(Repeat), "Run `steps` a fixed number of times.";
    "Flow", "while", While(While), "Run `steps` while the condition holds (bounded by max_iterations).";
    "Flow", "for_each", ForEach(ForEach), "Run `steps` once per item of a list.";
    "Flow", "break", Break(Break), "Leave the innermost repeat / while / for_each loop.";
    "Flow", "stop", Stop(Stop), "End the flow now, successfully or as a failure.";
    "Flow", "run_flow", RunFlow(RunFlow), "Run another flow file as a sub-flow.";

    "File", "file_copy", FileCopy(CopyMove), "Copy a file (to a file path or into a folder).";
    "File", "file_move", FileMove(CopyMove), "Move or rename a file or folder.";
    "File", "file_delete", FileDelete(FileDelete), "Delete file(s); wildcards allowed.";
    "File", "file_write", FileWrite(FileWrite), "Write or append text to a file (creates folders).";
    "File", "file_read", FileRead(PathToVar), "Read a text file into a variable.";
    "File", "file_exists", FileExists(PathToVar), "Store \"true\"/\"false\" depending on whether a path exists.";
    "File", "file_info", FileInfo(PathInfo), "Store name, extension, size, dates etc. of a path in an object variable.";
    "File", "dir_create", DirCreate(PathOnly), "Create a folder (and parents).";
    "File", "dir_delete", DirDelete(DirDelete), "Delete a folder.";
    "File", "dir_list", DirList(DirList), "List files/folders matching a pattern into an array variable.";

    "Program", "run_program", RunProgram(RunProgram), "Start a program, optionally waiting and capturing its output.";
    "Program", "kill_program", KillProgram(KillProgram), "Terminate all processes with the given image name.";
    "Program", "open", Open(Open), "Open a file, folder or URL with its default application.";

    "Window", "window_activate", WindowActivate(WindowTarget), "Wait for a window and bring it to the foreground.";
    "Window", "window_wait", WindowWait(WindowTarget), "Wait until a window with the given title exists.";
    "Window", "window_wait_close", WindowWaitClose(WindowTarget), "Wait until no window with the given title exists.";
    "Window", "window_close", WindowClose(WindowTarget), "Close a window (like clicking X).";
    "Window", "window_set_state", WindowSetState(WindowSetState), "Maximize, minimize or restore a window.";
    "Window", "window_exists", WindowExists(WindowExists), "Store \"true\"/\"false\" depending on whether a window exists (no waiting).";

    "Mouse", "mouse_move", MouseMove(MouseMove), "Move the mouse pointer.";
    "Mouse", "mouse_click", MouseClick(MouseClick), "Click (optionally at a position).";
    "Mouse", "mouse_scroll", MouseScroll(MouseScroll), "Scroll the mouse wheel.";
    "Mouse", "mouse_drag", MouseDrag(MouseDrag), "Drag from one point to another.";
    "Mouse", "mouse_down", MouseDown(MouseButtonAt), "Press and hold a mouse button.";
    "Mouse", "mouse_up", MouseUp(MouseButtonAt), "Release a held mouse button.";
    "Mouse", "mouse_position", MousePosition(MousePosition), "Store the current pointer position.";

    "Keyboard", "type_text", TypeText(TypeText), "Type text into the focused control.";
    "Keyboard", "key_press", KeyPress(KeyPress), "Press a key or key combination such as ctrl+s.";
    "Keyboard", "key_down", KeyDown(KeyHold), "Press and hold key(s), e.g. shift while clicking.";
    "Keyboard", "key_up", KeyUp(KeyHold), "Release key(s) held with key_down.";

    "UI Element", "ui_click", UiClick(UiClick), "Find a UI element (UI Automation) and click it.";
    "UI Element", "ui_set_text", UiSetText(UiSetText), "Find an input element and set its text.";
    "UI Element", "ui_get_text", UiGetText(UiToVar), "Read an element's value/text into a variable.";
    "UI Element", "ui_wait", UiWait(UiWait), "Wait until a UI element exists.";
    "UI Element", "ui_exists", UiExists(UiToVar), "Store \"true\"/\"false\" depending on whether an element exists (waits up to timeout_ms).";

    "Clipboard", "clipboard_set", ClipboardSet(ClipboardSet), "Put text on the clipboard.";
    "Clipboard", "clipboard_get", ClipboardGet(OutputVar), "Read clipboard text into a variable.";

    "Screen", "screenshot", Screenshot(Screenshot), "Save a PNG screenshot of a monitor, window or region.";
    "Screen", "screen_size", ScreenSize(ScreenSize), "Store the primary screen width and height.";
    "Screen", "pixel_get", PixelGet(PixelGet), "Store the colour of a screen pixel as #RRGGBB.";
    "Screen", "pixel_matches", PixelMatches(PixelMatches), "Store \"true\"/\"false\": does a pixel have the expected colour?";

    "Image", "image_find", ImageFind(ImageFind), "Look for a picture on screen; store found/x/y (never fails).";
    "Image", "image_find_all", ImageFindAll(ImageFindAll), "Store every place a picture appears on screen.";
    "Image", "image_click", ImageClick(ImageClick), "Wait for a picture on screen and click it.";
    "Image", "image_wait", ImageWait(ImageWait), "Wait until a picture appears on screen.";
    "Image", "image_wait_vanish", ImageWaitVanish(ImageWait), "Wait until a picture is no longer on screen.";

    "Dialog", "message_box", MessageBox(MessageBox), "Show a message box and store which button was clicked.";
    "Dialog", "input_box", InputBox(InputBox), "Ask the user to type a value (optionally masked).";
}

impl Action {
    /// Nested step lists (for control-flow actions).
    pub fn children(&self) -> Vec<&Vec<Step>> {
        match self {
            Action::If(a) => vec![&a.then, &a.otherwise],
            Action::Repeat(a) => vec![&a.steps],
            Action::While(a) => vec![&a.steps],
            Action::ForEach(a) => vec![&a.steps],
            _ => vec![],
        }
    }

    pub fn children_mut(&mut self) -> Vec<&mut Vec<Step>> {
        match self {
            Action::If(a) => vec![&mut a.then, &mut a.otherwise],
            Action::Repeat(a) => vec![&mut a.steps],
            Action::While(a) => vec![&mut a.steps],
            Action::ForEach(a) => vec![&mut a.steps],
            _ => vec![],
        }
    }

    /// Names (JSON keys) of the nested step lists, matching `children()`.
    pub fn child_labels(&self) -> &'static [&'static str] {
        match self {
            Action::If(_) => &["then", "else"],
            Action::Repeat(_) | Action::While(_) | Action::ForEach(_) => &["steps"],
            _ => &[],
        }
    }

    /// True for actions that move the mouse or press keys (fail-safe applies).
    pub fn uses_input(&self) -> bool {
        (matches!(self.category(), "Mouse" | "Keyboard") && !matches!(self, Action::MousePosition(_)))
            || matches!(self, Action::UiClick(_) | Action::UiSetText(_) | Action::ImageClick(_))
    }
}
