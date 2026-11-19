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

// ------------------------------------------------------------------------ OCR

/// Where OCR reads from: an image file, a window, a screen region or a monitor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OcrSource {
    /// Read this picture file (PNG/JPG/BMP) instead of the screen.
    pub image: String,
    /// Read this window (part of its title), e.g. "Tally" or "Notepad".
    pub window: String,
    /// Read only this screen rectangle {x, y, width, height}; null = whole window / monitor.
    pub region: Option<Region>,
    /// Monitor to read when no window or region is given (0 = primary).
    pub monitor: u32,
    /// Language tag such as en-US or hi (empty = the Windows display languages).
    pub language: String,
    /// Enlarge the picture before reading; 2 helps with small screen fonts.
    pub scale: f32,
}
impl Default for OcrSource {
    fn default() -> Self {
        Self { image: String::new(), window: String::new(), region: None, monitor: 0, language: String::new(), scale: 2.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OcrRead {
    #[serde(flatten)]
    pub source: OcrSource,
    /// Variable receiving the recognised text (one line per text line).
    pub output_var: String,
    /// Variable receiving the list of lines (optional).
    pub lines_var: String,
    /// Variable receiving every word with its screen position: {text, x, y, width, height, center_x, center_y} (optional).
    pub words_var: String,
}
impl Default for OcrRead {
    fn default() -> Self {
        Self { source: OcrSource::default(), output_var: "ocr_text".into(), lines_var: String::new(), words_var: String::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OcrFind {
    #[serde(flatten)]
    pub source: OcrSource,
    /// Text to look for (interpolated); may be several words, e.g. "Save As".
    pub text: String,
    /// Match regardless of upper/lower case.
    pub ignore_case: bool,
    /// Which match to use when the text appears more than once (1 = first, top to bottom).
    pub occurrence: u32,
    /// Keep looking for up to this many ms (the screen may still be changing).
    pub timeout_ms: u64,
    /// Variable receiving "true"/"false" (ocr_find never fails when the text is missing).
    pub found_var: String,
    /// Variable receiving the centre X of the text on screen.
    pub x_var: String,
    /// Variable receiving the centre Y of the text on screen.
    pub y_var: String,
}
impl Default for OcrFind {
    fn default() -> Self {
        Self {
            source: OcrSource::default(),
            text: String::new(),
            ignore_case: true,
            occurrence: 1,
            timeout_ms: 5000,
            found_var: "found".into(),
            x_var: String::new(),
            y_var: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OcrClick {
    #[serde(flatten)]
    pub find: OcrFind,
    /// Click this many pixels right of the text centre (negative = left), e.g. into the box next to a label.
    pub offset_x: i32,
    /// Click this many pixels below the text centre (negative = above).
    pub offset_y: i32,
    /// left, right or middle.
    pub button: MouseButton,
    /// 1 = single, 2 = double click.
    pub clicks: u32,
}
impl Default for OcrClick {
    fn default() -> Self {
        Self { find: OcrFind { found_var: String::new(), ..OcrFind::default() }, offset_x: 0, offset_y: 0, button: MouseButton::Left, clicks: 1 }
    }
}

// ------------------------------------------------------------------------- data

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReadExcel {
    /// Workbook file (.xlsx, .xlsm, .xls, .xlsb, .ods).
    pub path: String,
    /// Sheet name; empty = the first sheet.
    pub sheet: String,
    /// Only this cell range, e.g. `A1:D50`; empty = the whole used area.
    pub range: String,
    /// The first row holds column names.
    pub has_header: bool,
    /// Return only this column as a list: header name (e.g. `GSTIN`), letter (`B`) or number (`2`). Empty = all columns.
    pub column: String,
    /// Leave out empty rows (and empty cells when reading one column).
    pub skip_empty: bool,
    /// Variable receiving the data: a list of values (column), of records {header: value}, or of rows.
    pub output_var: String,
    /// Variable receiving the number of rows (optional).
    pub count_var: String,
}
impl Default for ReadExcel {
    fn default() -> Self {
        Self {
            path: String::new(),
            sheet: String::new(),
            range: String::new(),
            has_header: true,
            column: String::new(),
            skip_empty: true,
            output_var: "rows".into(),
            count_var: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReadCsv {
    /// CSV / TSV / text table file.
    pub path: String,
    /// Separator character: `,` `;` `|` or `tab`; empty = detect automatically.
    pub delimiter: String,
    /// The first row holds column names.
    pub has_header: bool,
    /// Return only this column as a list: header name, letter (`B`) or number (`2`). Empty = all columns.
    pub column: String,
    /// Leave out empty rows (and empty cells when reading one column).
    pub skip_empty: bool,
    /// Variable receiving the data: a list of values (column), of records {header: value}, or of rows.
    pub output_var: String,
    /// Variable receiving the number of rows (optional).
    pub count_var: String,
}
impl Default for ReadCsv {
    fn default() -> Self {
        Self {
            path: String::new(),
            delimiter: String::new(),
            has_header: true,
            column: String::new(),
            skip_empty: true,
            output_var: "rows".into(),
            count_var: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReadPdf {
    /// PDF file.
    pub path: String,
    /// Pages to read, e.g. `1`, `2-4`, `1,3,5-`; empty = all pages.
    pub pages: String,
    /// Variable receiving the text (pages separated by a blank line).
    pub output_var: String,
    /// Variable receiving the number of pages in the PDF (optional).
    pub page_count_var: String,
}
impl Default for ReadPdf {
    fn default() -> Self {
        Self { path: String::new(), pages: String::new(), output_var: "pdf_text".into(), page_count_var: String::new() }
    }
}

// ------------------------------------------------------------------------ email

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SendEmail {
    /// `gmail`, `outlook365`, `outlook.com`, `yahoo`, `zoho` (fills server/port), or `custom`.
    pub provider: String,
    /// SMTP server (custom provider), e.g. `smtp.yourcompany.com`.
    pub smtp_host: String,
    /// SMTP port; 0 = the provider's default (587).
    pub smtp_port: u16,
    /// `starttls` (port 587), `tls` (port 465) or `none` (internal relays only).
    pub security: String,
    /// Login name, usually your e-mail address. Empty = no login (internal relays).
    pub username: String,
    /// Name of the password stored in Windows Credential Manager (`autoflow secret set NAME`).
    /// For Gmail use an App Password. Never put the password itself in the flow.
    pub password_secret: String,
    /// Sender, e.g. `Accounts <me@gmail.com>`; empty = the user name.
    pub from: String,
    /// Recipients, separated by ; or ,
    pub to: String,
    /// Copy recipients, separated by ; or ,
    pub cc: String,
    /// Blind-copy recipients, separated by ; or ,
    pub bcc: String,
    /// Subject line (interpolated).
    pub subject: String,
    /// Message text (interpolated).
    pub body: String,
    /// The body is HTML.
    pub html: bool,
    /// Files to attach, one per item (interpolated; relative to the flow folder).
    pub attachments: Vec<String>,
    /// Give up after this many ms.
    pub timeout_ms: u64,
}
impl Default for SendEmail {
    fn default() -> Self {
        Self {
            provider: "gmail".into(),
            smtp_host: String::new(),
            smtp_port: 0,
            security: "starttls".into(),
            username: String::new(),
            password_secret: "gmail".into(),
            from: String::new(),
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            subject: String::new(),
            body: String::new(),
            html: false,
            attachments: vec![],
            timeout_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OutlookSend {
    /// Recipients, separated by ; or ,
    pub to: String,
    /// Copy recipients, separated by ; or ,
    pub cc: String,
    /// Blind-copy recipients, separated by ; or ,
    pub bcc: String,
    /// Subject line (interpolated).
    pub subject: String,
    /// Message text (interpolated).
    pub body: String,
    /// The body is HTML.
    pub html: bool,
    /// Files to attach, one per item (interpolated; relative to the flow folder).
    pub attachments: Vec<String>,
    /// Send immediately. Off = open the mail in Outlook as a draft for you to check and send.
    pub send_now: bool,
    /// Send from this Outlook account (its e-mail address); empty = the default account.
    pub account: String,
}

// -------------------------------------------------------------------------- web

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserOpen {
    /// Page to open: a URL (https://…) or a local .html file path.
    pub url: String,
    /// "edge" (default), "chrome", or the full path to a Chromium-based browser .exe.
    pub browser: String,
    /// Run without a visible window.
    pub headless: bool,
    /// Window width in pixels.
    pub width: u32,
    /// Window height in pixels.
    pub height: u32,
    /// Leave the browser open when the flow ends.
    pub keep_open: bool,
    /// How long to wait for the page to load, in ms.
    pub timeout_ms: u64,
}
impl Default for BrowserOpen {
    fn default() -> Self {
        Self { url: String::new(), browser: "edge".into(), headless: false, width: 1280, height: 900, keep_open: false, timeout_ms: 30_000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserNavigate {
    /// URL or local .html file path to load in the open browser.
    pub url: String,
    /// How long to wait for the page to load, in ms.
    pub timeout_ms: u64,
}
impl Default for BrowserNavigate {
    fn default() -> Self {
        Self { url: String::new(), timeout_ms: 30_000 }
    }
}

/// Which element on the page: by `element_id` (simplest) or by CSS `selector`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WebTarget {
    /// The element's id attribute, e.g. `email` for <input id="email"> (without #).
    /// (Named element_id because every step already has its own `id`.)
    pub element_id: String,
    /// CSS selector instead of an id, e.g. `form button[type=submit]`, `.result`.
    pub selector: String,
    /// How long to wait for the element to appear, in ms.
    pub timeout_ms: u64,
}
impl Default for WebTarget {
    fn default() -> Self {
        Self { element_id: String::new(), selector: String::new(), timeout_ms: 10_000 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserElement {
    #[serde(flatten)]
    pub target: WebTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserType {
    #[serde(flatten)]
    pub target: WebTarget,
    /// Text to type (interpolated).
    pub text: String,
    /// Empty the field first.
    pub clear: bool,
    /// Press Enter after typing (submits most forms).
    pub press_enter: bool,
}
impl Default for BrowserType {
    fn default() -> Self {
        Self { target: WebTarget::default(), text: String::new(), clear: true, press_enter: false }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserGetText {
    #[serde(flatten)]
    pub target: WebTarget,
    /// What to read: empty = visible text, `value` = input value, `html`, or any attribute name (e.g. `href`).
    pub attribute: String,
    /// Variable that receives the result.
    pub output_var: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserScreenshot {
    /// PNG path to write.
    pub path: String,
    /// Only this element (by its id attribute); empty = the visible page.
    pub element_id: String,
    /// Only this element (by CSS selector); empty = the visible page.
    pub selector: String,
    /// Capture the whole page including the parts below the fold (ignored for an element).
    pub full_page: bool,
    /// How long to wait for the element, in ms.
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserWaitText {
    /// Text to wait for (interpolated), e.g. a heading that only the result page shows.
    pub text: String,
    /// Look only inside this element (by id); empty = the whole page.
    pub element_id: String,
    /// Look only inside this element (by CSS selector); empty = the whole page.
    pub selector: String,
    /// Compare case-insensitively.
    pub ignore_case: bool,
    /// How long to wait, in ms.
    pub timeout_ms: u64,
    /// Variable receiving the page/element text once found (optional).
    pub output_var: String,
}
impl Default for BrowserWaitText {
    fn default() -> Self {
        Self { text: String::new(), element_id: String::new(), selector: String::new(), ignore_case: true, timeout_ms: 30_000, output_var: String::new() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserEval {
    /// JavaScript expression evaluated in the page; its value is returned (promises are awaited).
    pub script: String,
    /// Variable that receives the result.
    pub output_var: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserClose {}

/// Which messages to read, shared by read_email and outlook_read.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MailFilter {
    /// Folder to read, e.g. INBOX (IMAP) or Inbox/Invoices (Outlook).
    pub folder: String,
    /// Only messages not read yet.
    pub unread_only: bool,
    /// Only messages from this sender (address or part of it).
    pub from: String,
    /// Only messages whose subject contains this text.
    pub subject_contains: String,
    /// Only messages received on or after this date (e.g. 01-11-2026, "today"; empty = any date).
    pub since: String,
    /// Read at most this many messages.
    pub max: u32,
    /// Newest messages first.
    pub newest_first: bool,
    /// Mark the messages read afterwards.
    pub mark_read: bool,
    /// Move the messages to this folder afterwards (e.g. Processed); empty = leave them.
    pub move_to: String,
    /// Save attachments into this folder (empty = only list their names).
    pub save_attachments_to: String,
    /// Only attachments matching this pattern, e.g. *.pdf or *.xlsx.
    pub attachment_filter: String,
    /// Variable receiving the list of messages:
    /// {from, from_name, to, subject, date, body, attachments (saved paths), attachment_names}.
    pub output_var: String,
    /// Variable receiving how many messages were read (optional).
    pub count_var: String,
}
impl Default for MailFilter {
    fn default() -> Self {
        Self {
            folder: String::new(),
            unread_only: true,
            from: String::new(),
            subject_contains: String::new(),
            since: String::new(),
            max: 20,
            newest_first: true,
            mark_read: false,
            move_to: String::new(),
            save_attachments_to: String::new(),
            attachment_filter: String::new(),
            output_var: "emails".into(),
            count_var: String::new(),
        }
    }
}

/// Read e-mail from an IMAP mailbox (Gmail, Zoho, Yahoo, any IMAP server).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReadEmail {
    /// gmail, outlook365, yahoo, zoho, zoho.in, or custom (then set imap_host).
    pub provider: String,
    /// IMAP server; empty = the provider's.
    pub imap_host: String,
    /// IMAP port (0 = 993, TLS).
    pub imap_port: u16,
    /// Mailbox login, usually the e-mail address.
    pub username: String,
    /// Name of the password stored with `autoflow secret set NAME` (Gmail: an App Password).
    pub password_secret: String,
    #[serde(flatten)]
    pub filter: MailFilter,
}
impl Default for ReadEmail {
    fn default() -> Self {
        Self {
            provider: "gmail".into(),
            imap_host: String::new(),
            imap_port: 0,
            username: String::new(),
            password_secret: String::new(),
            filter: MailFilter { folder: "INBOX".into(), ..MailFilter::default() },
        }
    }
}

/// Read e-mail from a folder of the Outlook desktop app (classic Outlook).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OutlookRead {
    /// Outlook account / mailbox name to read (empty = the default one).
    pub account: String,
    #[serde(flatten)]
    pub filter: MailFilter,
}
impl Default for OutlookRead {
    fn default() -> Self {
        Self { account: String::new(), filter: MailFilter { folder: "Inbox".into(), ..MailFilter::default() } }
    }
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
    /// Picture to show above the text box (PNG/JPG path), e.g. a CAPTCHA captured with browser_screenshot.
    pub image: String,
    /// Variable receiving the text.
    pub output_var: String,
    /// Fail the step when the user presses Cancel (otherwise the variable is set to "").
    pub fail_on_cancel: bool,
}
impl Default for InputBox {
    fn default() -> Self {
        Self {
            title: "AutoFlowMate".into(),
            prompt: String::new(),
            default: String::new(),
            password: false,
            image: String::new(),
            output_var: "input".into(),
            fail_on_cancel: true,
        }
    }
}

// --------------------------------------------------------- error handling

/// Run steps; if one fails, run `catch` instead of stopping the flow; always run `finally`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Try {
    /// Steps to try.
    pub steps: Vec<Step>,
    /// Steps run when one of `steps` fails (the error text is in error_var).
    pub catch: Vec<Step>,
    /// Steps run afterwards in every case (e.g. close the browser or the program).
    pub finally: Vec<Step>,
    /// Variable receiving the error message (it names the step that failed).
    pub error_var: String,
}
impl Default for Try {
    fn default() -> Self {
        Self { steps: vec![], catch: vec![], finally: vec![], error_var: "error".into() }
    }
}

// ----------------------------------------------------------------------- text

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextOperation {
    #[default]
    Trim,
    Upper,
    Lower,
    Title,
    Length,
    UrlEncode,
    RemoveSpaces,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TextTransform {
    /// The text (interpolated), e.g. "${row.Name}".
    pub input: String,
    /// trim, upper, lower, title (Each Word Capitalised), length (number of characters),
    /// url_encode (for a URL query value) or remove_spaces.
    pub operation: TextOperation,
    /// Variable receiving the result.
    pub output_var: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TextReplace {
    /// The text (interpolated).
    pub input: String,
    /// Text to look for (a regular expression when `regex` is true).
    pub find: String,
    /// Replacement. With regex, $1 / ${name} insert captured groups.
    pub replace_with: String,
    /// Treat `find` as a regular expression.
    pub regex: bool,
    /// Match regardless of upper/lower case.
    pub ignore_case: bool,
    /// Variable receiving the result.
    pub output_var: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TextExtract {
    /// The text to search (interpolated), e.g. "${pdf_text}".
    pub input: String,
    /// Regular expression, e.g. "Invoice No[.:]\\s*(\\S+)" or "\\d{2}[A-Z]{5}\\d{4}[A-Z]\\d[Z][A-Z\\d]" (GSTIN).
    pub pattern: String,
    /// Capture group to return: 0 = the whole match, 1 = the first ( ) group...
    pub group: u32,
    /// Return every match as a list instead of the first one.
    pub all: bool,
    /// Match regardless of upper/lower case.
    pub ignore_case: bool,
    /// Variable receiving the match ("" when nothing matches) or the list of matches.
    pub output_var: String,
    /// Variable receiving "true"/"false": was anything found? (optional)
    pub found_var: String,
}
impl Default for TextExtract {
    fn default() -> Self {
        Self { input: String::new(), pattern: String::new(), group: 0, all: false, ignore_case: false, output_var: "match".into(), found_var: String::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TextSplit {
    /// The text to split (interpolated).
    pub input: String,
    /// Separator, e.g. "," or "\n" for lines or "\t" for tabs (a regular expression when `regex` is true).
    pub separator: String,
    /// Treat `separator` as a regular expression, e.g. "[,;]\\s*".
    pub regex: bool,
    /// Trim spaces around each part.
    pub trim: bool,
    /// Leave out empty parts.
    pub skip_empty: bool,
    /// Variable receiving the list of parts.
    pub output_var: String,
}
impl Default for TextSplit {
    fn default() -> Self {
        Self { input: String::new(), separator: ",".into(), regex: false, trim: true, skip_empty: true, output_var: "parts".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TextJoin {
    /// The list to join, e.g. "${names}".
    pub items: Value,
    /// Put between the items, e.g. ", " or "\n".
    pub separator: String,
    /// Variable receiving the text.
    pub output_var: String,
}
impl Default for TextJoin {
    fn default() -> Self {
        Self { items: Value::String(String::new()), separator: ", ".into(), output_var: "text".into() }
    }
}

// ------------------------------------------------------------ dates and maths

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DateOperation {
    /// Re-write the date in `format`.
    #[default]
    Format,
    /// Add `amount` `unit`s (negative to subtract), then format.
    Add,
    /// Whole `unit`s from `date` to `other_date` (a number).
    Diff,
    /// An object with year, month, day, hour, minute, second, weekday (1 = Monday), weekday_name, month_name, day_of_year.
    Parts,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DateCalc {
    /// format, add, diff or parts.
    pub operation: DateOperation,
    /// The date (interpolated); empty = now, "today" = today 00:00. Excel date numbers work too.
    pub date: String,
    /// How `date` is written, e.g. dd/MM/yyyy (empty = recognise common formats, day first).
    pub input_format: String,
    /// add: how many units (negative to go back).
    pub amount: Int,
    /// add / diff: seconds, minutes, hours, days, weeks, months or years.
    pub unit: String,
    /// diff: the later date (empty = now).
    pub other_date: String,
    /// Output format (tokens as in ${now:...}): yyyy MM dd HH hh mm ss tt MMM dddd.
    pub format: String,
    /// Variable receiving the result.
    pub output_var: String,
}
impl Default for DateCalc {
    fn default() -> Self {
        Self {
            operation: DateOperation::Format,
            date: String::new(),
            input_format: String::new(),
            amount: Int::Num(0),
            unit: "days".into(),
            other_date: String::new(),
            format: "dd-MM-yyyy".into(),
            output_var: "date".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Calculate {
    /// Expression (interpolated), e.g. "${amount} * 18 / 100". Supports + - * / % ^ ( ),
    /// comparisons (result 1 or 0) and round(x), round(0.01, x), int, floor, ceil, abs, min, max, sqrt.
    pub expression: String,
    /// Round the result to this many decimal places (empty = no rounding).
    pub decimals: Option<u32>,
    /// Variable receiving the number.
    pub output_var: String,
}
impl Default for Calculate {
    fn default() -> Self {
        Self { expression: String::new(), decimals: None, output_var: "result".into() }
    }
}

// ------------------------------------------------------------- data writing

/// Append a value (text, number or record) to a list variable, creating it if needed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ListAppend {
    /// List variable to add to (missing = a new empty list).
    pub name: String,
    /// Value to add. Strings are interpolated; an object such as
    /// {"Comment": "${row.Comment}", "Label": "${label}"} adds a record (a row for write_csv / write_excel).
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WriteCsv {
    /// CSV file to write (folders are created).
    pub path: String,
    /// The rows: usually "${results}" - a list of records, a list of lists or a list of values.
    pub data: Value,
    /// Columns to write, in order (empty = every key of the records, in first-seen order).
    pub columns: Vec<String>,
    /// Field separator: "," (default), ";" or "\t" for tab.
    pub delimiter: String,
    /// Write a header row with the column names.
    pub header: bool,
    /// Add to the end of an existing file (the header is not repeated).
    pub append: bool,
    /// Start the file with a UTF-8 marker so Excel shows non-English text correctly.
    pub excel_bom: bool,
    /// Variable receiving the number of rows written (optional).
    pub count_var: String,
}
impl Default for WriteCsv {
    fn default() -> Self {
        Self {
            path: String::new(),
            data: Value::String(String::new()),
            columns: vec![],
            delimiter: ",".into(),
            header: true,
            append: false,
            excel_bom: true,
            count_var: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WriteExcel {
    /// .xlsx workbook to create (an existing file is replaced; close it in Excel first).
    pub path: String,
    /// Sheet name.
    pub sheet: String,
    /// The rows: usually "${results}" - a list of records, a list of lists or a list of values.
    /// Numbers stay numbers; text starting with = is written as a formula.
    pub data: Value,
    /// Columns to write, in order (empty = every key of the records, in first-seen order).
    pub columns: Vec<String>,
    /// Write a bold, frozen header row with the column names.
    pub header: bool,
    /// Size the columns to fit their contents.
    pub autofit: bool,
    /// Variable receiving the number of rows written (optional).
    pub count_var: String,
}
impl Default for WriteExcel {
    fn default() -> Self {
        Self {
            path: String::new(),
            sheet: "Sheet1".into(),
            data: Value::String(String::new()),
            columns: vec![],
            header: true,
            autofit: true,
            count_var: String::new(),
        }
    }
}

// ------------------------------------------------------------------ database

/// Where the data is. See the db_query description for connection examples.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DbConnection {
    /// A SQLite file (sqlite:data/app.db, or a path ending in .db) or an ODBC connection string, e.g.
    /// DSN=TallyODBC64_9000 or Driver={ODBC Driver 18 for SQL Server};Server=.;Database=Sales;Trusted_Connection=yes;TrustServerCertificate=yes.
    /// Write {password} where the password goes; it is taken from password_secret.
    pub connection: String,
    /// Name of the database password stored with `autoflow secret set NAME` (replaces {password}).
    pub password_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DbQuery {
    #[serde(flatten)]
    pub db: DbConnection,
    /// SELECT statement. Use ? for values and list them in params, e.g. "SELECT * FROM Ledger WHERE Parent = ?".
    pub sql: String,
    /// Values for the ? placeholders, in order (interpolated), e.g. ["${group}"].
    pub params: Vec<Value>,
    /// Return at most this many rows (0 = all).
    pub max_rows: u32,
    /// Variable receiving the rows as a list of records {column: value}.
    pub output_var: String,
    /// Variable receiving the number of rows (optional).
    pub count_var: String,
}
impl Default for DbQuery {
    fn default() -> Self {
        Self { db: DbConnection::default(), sql: String::new(), params: vec![], max_rows: 0, output_var: "rows".into(), count_var: String::new() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DbExecute {
    #[serde(flatten)]
    pub db: DbConnection,
    /// INSERT / UPDATE / DELETE / CREATE statement with ? placeholders, e.g.
    /// "INSERT INTO Labels (Comment, Label) VALUES (?, ?)". Without params, SQLite runs a whole script.
    pub sql: String,
    /// Values for the ? placeholders, in order (interpolated), e.g. ["${row.Comment}", "${label}"].
    pub params: Vec<Value>,
    /// Variable receiving the number of rows changed (optional).
    pub affected_var: String,
}

// ------------------------------------------------------------------ internet

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HttpRequest {
    /// GET, POST, PUT, PATCH, DELETE or HEAD.
    pub method: String,
    /// Address (interpolated), e.g. https://api.example.com/items?q=${query}.
    pub url: String,
    /// Extra request headers, e.g. {"Accept": "application/json"} (values are interpolated).
    pub headers: Map<String, Value>,
    /// Request body. A string is sent as-is; an object or list is sent as JSON.
    pub body: Value,
    /// Name of a token stored with `autoflow secret set NAME`, sent as "Authorization: Bearer <token>" (optional).
    pub bearer_secret: String,
    /// Save the response body to this file (for downloads); output_var then receives the path.
    pub save_to: String,
    /// Fail the step when the server answers with an error status (400 and above).
    pub fail_on_error_status: bool,
    /// Store a JSON response as an object/list (so ${response.field} works) instead of text.
    pub parse_json: bool,
    /// How long to wait for the whole request, in ms.
    pub timeout_ms: u64,
    /// Variable receiving the response body.
    pub output_var: String,
    /// Variable receiving the HTTP status code (optional).
    pub status_var: String,
}
impl Default for HttpRequest {
    fn default() -> Self {
        Self {
            method: "GET".into(),
            url: String::new(),
            headers: Map::new(),
            body: Value::String(String::new()),
            bearer_secret: String::new(),
            save_to: String::new(),
            fail_on_error_status: true,
            parse_json: true,
            timeout_ms: 60_000,
            output_var: "response".into(),
            status_var: String::new(),
        }
    }
}

// ------------------------------------------------------------------------ AI

/// Send one prompt to a language model and store its reply.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AiPrompt {
    /// lmstudio (local, default), ollama (local), openai_compatible, openai, gemini or anthropic.
    pub provider: String,
    /// Model name. Empty = the model loaded in LM Studio/Ollama, or a default for cloud providers.
    pub model: String,
    /// Server address; empty = the provider's default (LM Studio: http://localhost:1234/v1).
    pub base_url: String,
    /// Name of the API key stored with `autoflow secret set NAME` (not needed for local servers).
    pub api_key_secret: String,
    /// Instructions for the model (interpolated), e.g. "You classify customer comments."
    pub system: String,
    /// The prompt (interpolated), e.g. "Classify this comment: ${row.Comment}".
    pub prompt: String,
    /// Allowed answers, e.g. ["Fav", "UnFav", "Neutral"]. The model is told to reply with one,
    /// and the stored value is always exactly one of them (the step fails otherwise).
    pub labels: Vec<String>,
    /// Ask for JSON and store the parsed object (so ${reply.field} works).
    pub json: bool,
    /// Randomness, 0 = most predictable (empty = provider default; ignored by Claude).
    pub temperature: Option<f64>,
    /// Longest reply, in tokens (thinking models need room to think).
    pub max_tokens: u32,
    /// How long to wait for the reply, in ms.
    pub timeout_ms: u64,
    /// Variable receiving the reply (or the label / parsed JSON).
    pub output_var: String,
}
impl Default for AiPrompt {
    fn default() -> Self {
        Self {
            provider: "lmstudio".into(),
            model: String::new(),
            base_url: String::new(),
            api_key_secret: String::new(),
            system: String::new(),
            prompt: String::new(),
            labels: vec![],
            json: false,
            temperature: None,
            max_tokens: 4096,
            timeout_ms: 120_000,
            output_var: "reply".into(),
        }
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
    "Flow", "try", Try(Try), "Run steps; on failure run `catch` instead of stopping; always run `finally`.";

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

    "Data", "read_excel", ReadExcel(ReadExcel), "Read a sheet (or one column) of an Excel workbook into a list variable.";
    "Data", "read_csv", ReadCsv(ReadCsv), "Read a CSV/TSV table (or one column) into a list variable.";
    "Data", "read_pdf", ReadPdf(ReadPdf), "Read the text of a PDF (all or some pages) into a variable.";
    "Data", "list_append", ListAppend(ListAppend), "Add a value or record to a list variable (collect results).";
    "Data", "write_csv", WriteCsv(WriteCsv), "Write a list of records to a CSV file (Excel-friendly).";
    "Data", "write_excel", WriteExcel(WriteExcel), "Write a list of records to a new Excel workbook.";

    "Text", "text_transform", TextTransform(TextTransform), "Trim, change case, count or URL-encode text.";
    "Text", "text_replace", TextReplace(TextReplace), "Replace text (plain or regular expression).";
    "Text", "text_extract", TextExtract(TextExtract), "Pick out text with a regular expression (first match or all).";
    "Text", "text_split", TextSplit(TextSplit), "Split text into a list (by comma, line, tab or pattern).";
    "Text", "text_join", TextJoin(TextJoin), "Join a list into one text.";

    "Date & Math", "date_calc", DateCalc(DateCalc), "Format, add to, subtract or compare dates.";
    "Date & Math", "calculate", Calculate(Calculate), "Evaluate arithmetic such as ${amount} * 18 / 100.";

    "Database", "db_query", DbQuery(DbQuery), "Run a SELECT on SQLite or any ODBC database (SQL Server, MySQL, Access, Tally) into a list of records.";
    "Database", "db_execute", DbExecute(DbExecute), "Run INSERT / UPDATE / DELETE / CREATE on SQLite or an ODBC database.";

    "Internet", "http_request", HttpRequest(HttpRequest), "Call a web API or download a file (GET/POST/...).";

    "AI", "ai_prompt", AiPrompt(AiPrompt), "Ask a local (LM Studio, Ollama) or cloud (OpenAI, Gemini, Claude) AI model; optionally force one of given labels.";

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

    "OCR", "ocr_read", OcrRead(OcrRead), "Read the text on screen, in a window or in a picture (Windows OCR, offline).";
    "OCR", "ocr_find", OcrFind(OcrFind), "Look for text on screen; store found/x/y (never fails).";
    "OCR", "ocr_click", OcrClick(OcrClick), "Wait for text on screen and click it (e.g. a button or menu without UI Automation).";

    "Web", "browser_open", BrowserOpen(BrowserOpen), "Start Edge/Chrome and open a page (URL or local .html file).";
    "Web", "browser_navigate", BrowserNavigate(BrowserNavigate), "Load another page in the open browser.";
    "Web", "browser_type", BrowserType(BrowserType), "Type text into a page element given by id or selector.";
    "Web", "browser_click", BrowserClick(BrowserElement), "Click a page element given by id or selector.";
    "Web", "browser_wait", BrowserWait(BrowserElement), "Wait until a page element exists.";
    "Web", "browser_wait_text", BrowserWaitText(BrowserWaitText), "Wait until some text appears on the page (e.g. after a search).";
    "Web", "browser_get_text", BrowserGetText(BrowserGetText), "Read a page element's text, value or attribute into a variable.";
    "Web", "browser_screenshot", BrowserScreenshot(BrowserScreenshot), "Save a PNG of the page or of one element.";
    "Web", "browser_eval", BrowserEval(BrowserEval), "Run JavaScript in the page and store the result.";
    "Web", "browser_close", BrowserClose(BrowserClose), "Close the browser.";

    "Email", "send_email", SendEmail(SendEmail), "Send an e-mail with attachments through SMTP (Gmail, Microsoft 365, any server).";
    "Email", "outlook_send", OutlookSend(OutlookSend), "Create an e-mail in the Outlook desktop app and send it (or show it as a draft).";
    "Email", "read_email", ReadEmail(ReadEmail), "Read messages from an IMAP mailbox (Gmail...) and save their attachments.";
    "Email", "outlook_read", OutlookRead(OutlookRead), "Read messages from an Outlook desktop folder and save their attachments.";

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
            Action::Try(a) => vec![&a.steps, &a.catch, &a.finally],
            _ => vec![],
        }
    }

    pub fn children_mut(&mut self) -> Vec<&mut Vec<Step>> {
        match self {
            Action::If(a) => vec![&mut a.then, &mut a.otherwise],
            Action::Repeat(a) => vec![&mut a.steps],
            Action::While(a) => vec![&mut a.steps],
            Action::ForEach(a) => vec![&mut a.steps],
            Action::Try(a) => vec![&mut a.steps, &mut a.catch, &mut a.finally],
            _ => vec![],
        }
    }

    /// Names (JSON keys) of the nested step lists, matching `children()`.
    pub fn child_labels(&self) -> &'static [&'static str] {
        match self {
            Action::If(_) => &["then", "else"],
            Action::Repeat(_) | Action::While(_) | Action::ForEach(_) => &["steps"],
            Action::Try(_) => &["steps", "catch", "finally"],
            _ => &[],
        }
    }

    /// True for actions that move the mouse or press keys (fail-safe applies).
    pub fn uses_input(&self) -> bool {
        (matches!(self.category(), "Mouse" | "Keyboard") && !matches!(self, Action::MousePosition(_)))
            || matches!(self, Action::UiClick(_) | Action::UiSetText(_) | Action::ImageClick(_) | Action::OcrClick(_))
    }
}
