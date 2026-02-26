/// Button mapper: translates UnifiedInput → keyboard/mouse events via SendInput.
///
/// Always active (both profiles):
///   D-pad Up/Down/Left/Right → Arrow keys (two-frame confirm + repeat)
///   Cross    → Enter
///   Circle   → Escape
///   Triangle → Tab
///   Left stick  → Mouse cursor (velocity-based, configurable sensitivity)
///   Right stick → Mouse scroll wheel (vertical + horizontal)
///   PS       → Cycle profiles (Default ↔ Tmux)
///
/// Default profile (Windows Terminal shortcuts, auto-detected from settings.json):
///   Square   → new tab / profile 1   (newTab,  default ctrl+shift+1)
///   L1       → previous tab          (prevTab, default ctrl+shift+tab)
///   R1       → next tab              (nextTab, default ctrl+tab)
///   L2       → Ctrl+Win (hold)
///   R2       → Ctrl+C
///   L3       → Ctrl+T
///   R3       → Ctrl+P
///
/// Tmux profile (auto-detected from tmux config):
///   L1       → tmux prefix + previous-window key
///   R1       → tmux prefix + next-window key
///   L2       → Ctrl+Win (hold)
///   R2       → tmux prefix + kill-window key
///   R3       → Ctrl+P
///   Square   → tmux prefix + new-window key
///   L3       → Ctrl+T
///
/// Combos are sent atomically in a single SendInput call.

use crate::config::{OpenCodeConfig, ScrollConfig, StickMouseConfig, TouchpadConfig, TmuxConfig, WtConfig};
use crate::input::{ButtonState, DPad, UnifiedInput};
use crate::opencode_detect::{ActionBinding, OpenCodeDetected};
use crate::tmux_detect::TmuxDetected;
use crate::wt_detect::WtDetected;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Instant;

#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, MOUSEINPUT,
    KEYEVENTF_KEYUP,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    VK_RETURN, VK_ESCAPE, VK_TAB, VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT,
    VK_MENU, VK_SHIFT, VK_CONTROL,
};

/// Virtual key codes we use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VKey {
    Return,
    Escape,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Alt,
    Shift,
    Control,
    Win,
    // Letter keys
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    // Digit keys
    D0, D1, D2, D3, D4, D5, D6, D7, D8, D9,
    // Punctuation / symbols
    Semicolon,    // VK_OEM_1 (;:)
    LeftBracket,  // VK_OEM_4 ([{)
    RightBracket, // VK_OEM_6 (]})
    Backslash,    // VK_OEM_5 (\|)
    Quote,        // VK_OEM_7 ('")
    Slash,        // VK_OEM_2 (/?)
    Minus,        // VK_OEM_MINUS (-_)
    Equals,       // VK_OEM_PLUS (=+)
    Comma,        // VK_OEM_COMMA (,<)
    Period,       // VK_OEM_PERIOD (.>)
    Backtick,     // VK_OEM_3 (`~)
    Space,        // VK_SPACE
    // Function keys
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
}

#[cfg(windows)]
impl VKey {
    fn code(self) -> u16 {
        match self {
            VKey::Return => VK_RETURN,
            VKey::Escape => VK_ESCAPE,
            VKey::Tab => VK_TAB,
            VKey::Up => VK_UP,
            VKey::Down => VK_DOWN,
            VKey::Left => VK_LEFT,
            VKey::Right => VK_RIGHT,
            VKey::Alt => VK_MENU,
            VKey::Shift => VK_SHIFT,
            VKey::Control => VK_CONTROL,
            VKey::Win => 0x5B,  // VK_LWIN
            VKey::A => 0x41, VKey::B => 0x42, VKey::C => 0x43, VKey::D => 0x44,
            VKey::E => 0x45, VKey::F => 0x46, VKey::G => 0x47, VKey::H => 0x48,
            VKey::I => 0x49, VKey::J => 0x4A, VKey::K => 0x4B, VKey::L => 0x4C,
            VKey::M => 0x4D, VKey::N => 0x4E, VKey::O => 0x4F, VKey::P => 0x50,
            VKey::Q => 0x51, VKey::R => 0x52, VKey::S => 0x53, VKey::T => 0x54,
            VKey::U => 0x55, VKey::V => 0x56, VKey::W => 0x57, VKey::X => 0x58,
            VKey::Y => 0x59, VKey::Z => 0x5A,
            VKey::D0 => 0x30, VKey::D1 => 0x31, VKey::D2 => 0x32, VKey::D3 => 0x33,
            VKey::D4 => 0x34, VKey::D5 => 0x35, VKey::D6 => 0x36, VKey::D7 => 0x37,
            VKey::D8 => 0x38, VKey::D9 => 0x39,
            VKey::Semicolon => 0xBA,      // VK_OEM_1
            VKey::LeftBracket => 0xDB,    // VK_OEM_4
            VKey::RightBracket => 0xDD,   // VK_OEM_6
            VKey::Backslash => 0xDC,      // VK_OEM_5
            VKey::Quote => 0xDE,          // VK_OEM_7
            VKey::Slash => 0xBF,          // VK_OEM_2
            VKey::Minus => 0xBD,          // VK_OEM_MINUS
            VKey::Equals => 0xBB,         // VK_OEM_PLUS (unshifted =)
            VKey::Comma => 0xBC,          // VK_OEM_COMMA
            VKey::Period => 0xBE,         // VK_OEM_PERIOD
            VKey::Backtick => 0xC0,       // VK_OEM_3
            VKey::Space => 0x20,          // VK_SPACE
            VKey::F1  => 0x70, VKey::F2  => 0x71, VKey::F3  => 0x72, VKey::F4  => 0x73,
            VKey::F5  => 0x74, VKey::F6  => 0x75, VKey::F7  => 0x76, VKey::F8  => 0x77,
            VKey::F9  => 0x78, VKey::F10 => 0x79, VKey::F11 => 0x7A, VKey::F12 => 0x7B,
        }
    }
}

impl VKey {
    /// Parse a key name string into a VKey.
    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "return" | "enter" => Some(VKey::Return),
            "escape" | "esc" => Some(VKey::Escape),
            "tab" => Some(VKey::Tab),
            "up" => Some(VKey::Up),
            "down" => Some(VKey::Down),
            "left" => Some(VKey::Left),
            "right" => Some(VKey::Right),
            "alt" => Some(VKey::Alt),
            "shift" => Some(VKey::Shift),
            "ctrl" | "control" => Some(VKey::Control),
            "win" | "windows" | "super" | "meta" => Some(VKey::Win),
            "a" => Some(VKey::A), "b" => Some(VKey::B), "c" => Some(VKey::C),
            "d" => Some(VKey::D), "e" => Some(VKey::E), "f" => Some(VKey::F),
            "g" => Some(VKey::G), "h" => Some(VKey::H), "i" => Some(VKey::I),
            "j" => Some(VKey::J), "k" => Some(VKey::K), "l" => Some(VKey::L),
            "m" => Some(VKey::M), "n" => Some(VKey::N), "o" => Some(VKey::O),
            "p" => Some(VKey::P), "q" => Some(VKey::Q), "r" => Some(VKey::R),
            "s" => Some(VKey::S), "t" => Some(VKey::T), "u" => Some(VKey::U),
            "v" => Some(VKey::V), "w" => Some(VKey::W), "x" => Some(VKey::X),
            "y" => Some(VKey::Y), "z" => Some(VKey::Z),
            "0" => Some(VKey::D0), "1" => Some(VKey::D1), "2" => Some(VKey::D2),
            "3" => Some(VKey::D3), "4" => Some(VKey::D4), "5" => Some(VKey::D5),
            "6" => Some(VKey::D6), "7" => Some(VKey::D7), "8" => Some(VKey::D8),
            "9" => Some(VKey::D9),
            ";" | "semicolon" => Some(VKey::Semicolon),
            "[" | "leftbracket" => Some(VKey::LeftBracket),
            "]" | "rightbracket" => Some(VKey::RightBracket),
            "\\" | "backslash" => Some(VKey::Backslash),
            "'" | "quote" => Some(VKey::Quote),
            "/" | "slash" => Some(VKey::Slash),
            "-" | "minus" => Some(VKey::Minus),
            "=" | "equals" => Some(VKey::Equals),
            "," | "comma" => Some(VKey::Comma),
            "." | "period" => Some(VKey::Period),
            "`" | "backtick" => Some(VKey::Backtick),
            "space" => Some(VKey::Space),
            "f1"  => Some(VKey::F1),  "f2"  => Some(VKey::F2),  "f3"  => Some(VKey::F3),
            "f4"  => Some(VKey::F4),  "f5"  => Some(VKey::F5),  "f6"  => Some(VKey::F6),
            "f7"  => Some(VKey::F7),  "f8"  => Some(VKey::F8),  "f9"  => Some(VKey::F9),
            "f10" => Some(VKey::F10), "f11" => Some(VKey::F11), "f12" => Some(VKey::F12),
            _ => None,
        }
    }
}

/// Parse a key combo string like "Ctrl+B" or "p" into a Vec<VKey>.
pub fn parse_key_combo(s: &str) -> Option<Vec<VKey>> {
    s.split('+').map(|part| VKey::from_name(part.trim())).collect()
}

/// Active input profile. PS button cycles between Default and Tmux.
///
/// TODO: Add a third "Agent" profile that merges OpenCode + tmux shortcuts onto
/// buttons that make sense for AI-assisted coding sessions — e.g. session nav on
/// L1/R1 (OpenCode sessions), window nav on shoulder combos (tmux windows), new
/// session/window on Square (OpenCode) / Triangle (tmux), etc. The resolved
/// `OpenCodeState` and `TmuxState` are both already computed inside `MapperState`
/// and ready to dispatch; only the profile variant, PS cycling, LED slot, and
/// tray color need to be wired up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// L2-Touchpad unmapped.
    Default,
    /// L2-Touchpad send tmux prefix + key sequences.
    Tmux,
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Profile::Default => f.write_str("default"),
            Profile::Tmux    => f.write_str("tmux"),
        }
    }
}

/// An action the mapper can produce.
#[derive(Debug, Clone)]
pub enum Action {
    /// Press and release a key combo (modifiers held, main key pressed+released, modifiers released).
    KeyCombo(Vec<VKey>),
    /// Hold keys down (sent on button press, released on button release).
    KeyDown(Vec<VKey>),
    /// Release held keys.
    KeyUp(Vec<VKey>),
    /// Sequence of key combos with a delay between each (for tmux prefix+key).
    KeySequence(Vec<Vec<VKey>>),
    /// Mouse scroll event. Values in wheel-delta units (positive = up/right).
    Scroll { horizontal: i32, vertical: i32 },
    /// Relative mouse cursor movement (screen pixels). Emitted by touchpad touch.
    MouseMove { dx: i32, dy: i32 },
    /// Left mouse button click (press + release). Emitted by touchpad physical click.
    MouseClick,
    /// Custom action identifier (e.g., "new_session").
    Custom(String),
}

/// Key repeat timing.
const REPEAT_DELAY_MS: u64 = 300;  // hold before repeating
const REPEAT_RATE_MS: u64 = 100;   // interval between repeats

/// Scroll timing.
const SCROLL_MIN_INTERVAL_MS: u64 = 30;  // fastest scroll at full deflection
const SCROLL_MAX_INTERVAL_MS: u64 = 200; // slowest scroll near dead zone edge
const WHEEL_DELTA: i32 = 120;            // Windows standard per notch

/// Per-button repeat tracking with two-frame confirmation.
/// First frame of a new press is "pending" — only fires if still held next frame.
/// Filters single-frame hat switch glitches (~8ms latency, unnoticeable).
#[derive(Clone, Default)]
struct RepeatTimer {
    pending_since: Option<Instant>,
    pressed_at: Option<Instant>,
    last_fired: Option<Instant>,
}

impl RepeatTimer {
    fn on_press(&mut self, now: Instant) {
        self.pending_since = Some(now);
    }

    fn on_hold(&mut self, now: Instant) -> bool {
        if let Some(pending) = self.pending_since.take() {
            self.pressed_at = Some(pending);
            self.last_fired = Some(now);
            return true;
        }
        let pressed_at = match self.pressed_at {
            Some(t) => t,
            None => return false,
        };
        let held_ms = now.duration_since(pressed_at).as_millis() as u64;
        if held_ms < REPEAT_DELAY_MS {
            return false;
        }
        let last = self.last_fired.unwrap_or(pressed_at);
        if now.duration_since(last).as_millis() as u64 >= REPEAT_RATE_MS {
            self.last_fired = Some(now);
            return true;
        }
        false
    }

    fn on_release(&mut self) {
        self.pressed_at = None;
        self.pending_since = None;
    }
}

/// Resolved tmux button mappings (parsed once from config strings).
/// None = unmapped in tmux profile; Some = sends prefix + keys.
#[derive(Clone)]
struct TmuxState {
    prefix: Vec<VKey>,
    l1: Option<Vec<VKey>>,
    r1: Option<Vec<VKey>>,
    l2: Option<Vec<VKey>>,
    r2: Option<Vec<VKey>>,
    l3: Option<Vec<VKey>>,
    r3: Option<Vec<VKey>>,
    square: Option<Vec<VKey>>,
    share: Option<Vec<VKey>>,
    options: Option<Vec<VKey>>,
    touchpad: Option<Vec<VKey>>,
}

impl Default for TmuxState {
    fn default() -> Self {
        Self {
            prefix: vec![VKey::Control, VKey::B],
            l1: Some(vec![VKey::P]),                   // prev window
            r1: Some(vec![VKey::N]),                   // next window
            l2: None,
            r2: Some(vec![VKey::Shift, VKey::D7]),     // kill window (&)
            l3: None,
            r3: None,
            square: Some(vec![VKey::C]),               // new window
            share: None,
            options: None,
            touchpad: None,
        }
    }
}

/// Well-known tmux action → default key mapping (tmux defaults).
/// Used as fallback when auto-detection is unavailable.
fn default_key_for_action(action: &str) -> Option<Vec<VKey>> {
    match action {
        "previous-window" => Some(vec![VKey::P]),
        "next-window" => Some(vec![VKey::N]),
        "new-window" => Some(vec![VKey::C]),
        "kill-window" => Some(vec![VKey::Shift, VKey::D7]),   // &
        "copy-mode" => Some(vec![VKey::LeftBracket]),
        "resize-pane -Z" => Some(vec![VKey::Z]),              // zoom toggle
        "last-pane" => Some(vec![VKey::Semicolon]),
        "select-pane" => Some(vec![VKey::O]),                 // next pane
        "last-window" => Some(vec![VKey::L]),
        "detach-client" => Some(vec![VKey::D]),
        "split-window -h" => Some(vec![VKey::Shift, VKey::D5]), // %
        "split-window -v" => Some(vec![VKey::Shift, VKey::Quote]), // "
        _ => None,
    }
}

/// Resolve a button config value to VKey combo.
///
/// Resolution order:
/// 1. If empty → None (unmapped)
/// 2. Look up in auto-detected tmux bindings (action name → key)
/// 3. Look up in hardcoded tmux defaults (action name → key)
/// 4. Parse as direct key combo string (backward compatible)
fn resolve_button(value: &str, detected: Option<&TmuxDetected>) -> Option<Vec<VKey>> {
    if value.is_empty() {
        return None;
    }

    // Try auto-detected bindings first
    if let Some(det) = detected {
        if let Some(keys) = det.key_for_action(value) {
            log::debug!("Resolved tmux action '{value}' from detected bindings");
            return Some(keys.clone());
        }
    }

    // Try hardcoded defaults for well-known tmux actions
    if let Some(keys) = default_key_for_action(value) {
        log::debug!("Resolved tmux action '{value}' from hardcoded defaults");
        return Some(keys);
    }

    // Try parsing as direct key combo (backward compatible with manual config)
    parse_key_combo(value)
}

impl TmuxState {
    fn from_config(cfg: &TmuxConfig, detected: Option<&TmuxDetected>) -> Self {
        // Prefix: prefer detected, fall back to config, then hardcoded default
        let prefix = if cfg.auto_detect {
            detected
                .and_then(|d| d.prefix.clone())
                .unwrap_or_else(|| {
                    parse_key_combo(&cfg.prefix).unwrap_or_else(|| vec![VKey::Control, VKey::B])
                })
        } else {
            parse_key_combo(&cfg.prefix).unwrap_or_else(|| vec![VKey::Control, VKey::B])
        };

        // Resolve buttons: action name → detected key → default key → direct key combo
        let det = if cfg.auto_detect { detected } else { None };
        let resolve = |s: &str| -> Option<Vec<VKey>> { resolve_button(s, det) };

        log::info!("Tmux prefix resolved to: {:?}", prefix);

        Self {
            prefix,
            l1: resolve(&cfg.l1),
            r1: resolve(&cfg.r1),
            l2: resolve(&cfg.l2),
            r2: resolve(&cfg.r2),
            l3: resolve(&cfg.l3),
            r3: resolve(&cfg.r3),
            square: resolve(&cfg.square),
            share: resolve(&cfg.share),
            options: resolve(&cfg.options),
            touchpad: resolve(&cfg.touchpad),
        }
    }
}

// ── OpenCode hardcoded defaults ───────────────────────────────────────

/// Well-known OpenCode action → default binding fallback.
/// Used when auto-detection is unavailable or an action has no detected binding.
/// These mirror OpenCode's shipped defaults; users can override via config.
fn default_binding_for_opencode_action(action: &str) -> Option<ActionBinding> {
    match action {
        // Session navigation (ctrl+[ / ctrl+] are common TUI conventions)
        "session:prev" | "app:prev-session" | "app:session-prev" => {
            Some(ActionBinding::Combo(vec![VKey::Control, VKey::LeftBracket]))
        }
        "session:next" | "app:next-session" | "app:session-next" => {
            Some(ActionBinding::Combo(vec![VKey::Control, VKey::RightBracket]))
        }
        // New session (leader + n, leader defaults to ctrl+x)
        "app:new-session" | "app:session-new" => {
            Some(ActionBinding::LeaderKey(vec![VKey::N]))
        }
        // Toggle session list sidebar
        "app:toggle-session-list" => {
            Some(ActionBinding::Combo(vec![VKey::Control, VKey::Shift, VKey::S]))
        }
        _ => None,
    }
}

/// Resolve a button config value to an `ActionBinding` for the OpenCode profile.
///
/// Resolution order:
/// 1. Empty string  → None (unmapped)
/// 2. Auto-detected binding from opencode.json
/// 3. Hardcoded OpenCode defaults (action name lookup)
/// 4. Direct binding parse (e.g., "ctrl+]" or "<leader>n")
fn resolve_opencode_button(
    value: &str,
    detected: Option<&OpenCodeDetected>,
) -> Option<ActionBinding> {
    if value.is_empty() {
        return None;
    }

    // Try auto-detected bindings first
    if let Some(det) = detected {
        if let Some(binding) = det.binding_for_action(value) {
            log::debug!("Resolved OpenCode action '{value}' from detected bindings");
            return Some(binding.clone());
        }
    }

    // Try hardcoded defaults for well-known OpenCode actions
    if let Some(binding) = default_binding_for_opencode_action(value) {
        log::debug!("Resolved OpenCode action '{value}' from hardcoded defaults");
        return Some(binding);
    }

    // Try parsing as a direct binding string (e.g., "ctrl+]" or "<leader>n")
    crate::opencode_detect::parse_opencode_binding(value)
}

/// Resolved OpenCode button mappings (parsed once from config strings).
/// None = unmapped; Combo = direct keypress; LeaderKey = leader then key.
#[derive(Clone)]
struct OpenCodeState {
    leader: Vec<VKey>,
    l1: Option<ActionBinding>,
    r1: Option<ActionBinding>,
    l2: Option<ActionBinding>,
    r2: Option<ActionBinding>,
    l3: Option<ActionBinding>,
    r3: Option<ActionBinding>,
    square: Option<ActionBinding>,
    share: Option<ActionBinding>,
    options: Option<ActionBinding>,
    touchpad: Option<ActionBinding>,
}

impl Default for OpenCodeState {
    fn default() -> Self {
        // ctrl+x leader, session nav on L1/R1, new-session on Square
        Self {
            leader: vec![VKey::Control, VKey::X],
            l1: Some(ActionBinding::Combo(vec![VKey::Control, VKey::LeftBracket])),
            r1: Some(ActionBinding::Combo(vec![VKey::Control, VKey::RightBracket])),
            l2: None,
            r2: None,
            l3: None,
            r3: None,
            square: Some(ActionBinding::LeaderKey(vec![VKey::N])),
            share: None,
            options: None,
            touchpad: None,
        }
    }
}

impl OpenCodeState {
    fn from_config(cfg: &OpenCodeConfig, detected: Option<&OpenCodeDetected>) -> Self {
        // Leader: prefer detected, fall back to config string, then ctrl+x
        let leader = if cfg.auto_detect {
            detected
                .and_then(|d| d.leader.clone())
                .unwrap_or_else(|| {
                    parse_key_combo(&cfg.leader)
                        .unwrap_or_else(|| vec![VKey::Control, VKey::X])
                })
        } else {
            parse_key_combo(&cfg.leader).unwrap_or_else(|| vec![VKey::Control, VKey::X])
        };

        let det = if cfg.auto_detect { detected } else { None };
        let resolve = |s: &str| -> Option<ActionBinding> { resolve_opencode_button(s, det) };

        log::info!("OpenCode leader resolved to: {:?}", leader);

        Self {
            leader,
            l1: resolve(&cfg.l1),
            r1: resolve(&cfg.r1),
            l2: resolve(&cfg.l2),
            r2: resolve(&cfg.r2),
            l3: resolve(&cfg.l3),
            r3: resolve(&cfg.r3),
            square: resolve(&cfg.square),
            share: resolve(&cfg.share),
            options: resolve(&cfg.options),
            touchpad: resolve(&cfg.touchpad),
        }
    }
}

// ── Windows Terminal shortcut state ──────────────────────────────────

/// Resolved key combos for the Windows Terminal shortcut dictionary.
#[derive(Clone)]
struct WtState {
    square:  Option<Vec<VKey>>,   // newTab  (profile 1)
    l1:      Option<Vec<VKey>>,   // prevTab
    r1:      Option<Vec<VKey>>,   // nextTab
    l2:      Option<Vec<VKey>>,
    r2:      Option<Vec<VKey>>,
    l3:      Option<Vec<VKey>>,
    r3:      Option<Vec<VKey>>,
    share:   Option<Vec<VKey>>,
    options: Option<Vec<VKey>>,
}

impl Default for WtState {
    fn default() -> Self {
        Self {
            square:  default_key_for_wt_action("newTab"),
            l1:      default_key_for_wt_action("prevTab"),
            r1:      default_key_for_wt_action("nextTab"),
            l2:      None,
            r2:      None,
            l3:      None,
            r3:      None,
            share:   None,
            options: None,
        }
    }
}

/// Hardcoded fallback keys for well-known Windows Terminal actions.
fn default_key_for_wt_action(action: &str) -> Option<Vec<VKey>> {
    match action {
        "newTab"       => parse_key_combo("ctrl+shift+1"),
        "prevTab"      => parse_key_combo("ctrl+shift+tab"),
        "nextTab"      => parse_key_combo("ctrl+tab"),
        "closeTab"     => parse_key_combo("ctrl+shift+w"),
        "duplicateTab" => parse_key_combo("ctrl+shift+d"),
        "newWindow"    => parse_key_combo("ctrl+shift+n"),
        "find"         => parse_key_combo("ctrl+shift+f"),
        "splitDown"    => parse_key_combo("alt+shift+minus"),
        "splitRight"   => parse_key_combo("alt+shift+plus"),
        _ => None,
    }
}

/// Resolve a single button's value for the Windows Terminal profile.
/// Priority: auto-detected → hardcoded default → direct combo parse.
fn resolve_wt_button(value: &str, detected: Option<&WtDetected>) -> Option<Vec<VKey>> {
    if value.is_empty() {
        return None;
    }
    if let Some(det) = detected {
        if let Some(keys) = det.key_for_action(value) {
            log::debug!("Resolved WT action '{value}' from detected bindings");
            return Some(keys.clone());
        }
    }
    if let Some(keys) = default_key_for_wt_action(value) {
        log::debug!("Resolved WT action '{value}' from hardcoded defaults");
        return Some(keys);
    }
    parse_key_combo(value)
}

impl WtState {
    fn from_config(cfg: &WtConfig, detected: Option<&WtDetected>) -> Self {
        let det = if cfg.auto_detect { detected } else { None };
        let resolve = |s: &str| -> Option<Vec<VKey>> { resolve_wt_button(s, det) };
        Self {
            square:  resolve(&cfg.square),
            l1:      resolve(&cfg.l1),
            r1:      resolve(&cfg.r1),
            l2:      resolve(&cfg.l2),
            r2:      resolve(&cfg.r2),
            l3:      resolve(&cfg.l3),
            r3:      resolve(&cfg.r3),
            share:   resolve(&cfg.share),
            options: resolve(&cfg.options),
        }
    }
}

/// Main mapper state.
pub struct MapperState {
    prev: ButtonState,
    // D-pad repeat timers
    repeat_up: RepeatTimer,
    repeat_down: RepeatTimer,
    repeat_left: RepeatTimer,
    repeat_right: RepeatTimer,
    // Scroll state
    last_scroll_at: Option<Instant>,
    scroll_dead_zone: i16,
    scroll_sensitivity: f32,
    scroll_horizontal: bool,
    // Left stick as mouse cursor state
    stick_mouse_enabled: bool,
    stick_mouse_sensitivity: f32,
    stick_mouse_dead_zone: i16,
    stick_acc_x: f32,
    stick_acc_y: f32,
    // Mouse mode toggle: shared with tray thread.
    // false = touchpad touch moves cursor; true = left stick moves cursor.
    // Touchpad click (press) fires regardless of mode.
    mouse_stick_active: Arc<AtomicBool>,
    // Touchpad-as-mouse state
    prev_touch: Option<(u16, u16)>,
    touchpad_enabled: bool,
    touchpad_sensitivity: f32,
    // Profile system
    active_profile: Profile,
    tmux_available: bool, // false = only Default profile, PS does nothing
    tmux: TmuxState,
    opencode: OpenCodeState,
    wt: WtState,
}

impl Default for MapperState {
    fn default() -> Self {
        Self {
            prev: ButtonState::default(),
            repeat_up: RepeatTimer::default(),
            repeat_down: RepeatTimer::default(),
            repeat_left: RepeatTimer::default(),
            repeat_right: RepeatTimer::default(),
            last_scroll_at: None,
            scroll_dead_zone: 20,
            scroll_sensitivity: 1.0,
            scroll_horizontal: true,
            stick_mouse_enabled: true,
            stick_mouse_sensitivity: 8.0,
            stick_mouse_dead_zone: 15,
            stick_acc_x: 0.0,
            stick_acc_y: 0.0,
            mouse_stick_active: Arc::new(AtomicBool::new(false)),
            prev_touch: None,
            touchpad_enabled: true,
            touchpad_sensitivity: 1.5,
            active_profile: Profile::Default,
            tmux_available: true,
            tmux: TmuxState::default(),
            opencode: OpenCodeState::default(),
            wt: WtState::default(),
        }
    }
}

impl MapperState {
    /// Create a mapper with config-driven settings.
    /// Detected configurations are used to resolve action-name → key bindings.
    pub fn new(
        scroll: &ScrollConfig,
        stick_mouse: &StickMouseConfig,
        touchpad: &TouchpadConfig,
        tmux: &TmuxConfig,
        tmux_detected: Option<&TmuxDetected>,
        opencode: &OpenCodeConfig,
        opencode_detected: Option<&OpenCodeDetected>,
        wt: &WtConfig,
        wt_detected: Option<&WtDetected>,
        mouse_stick_active: Arc<AtomicBool>,
    ) -> Self {
        Self {
            scroll_dead_zone: scroll.dead_zone as i16,
            scroll_sensitivity: scroll.sensitivity,
            scroll_horizontal: scroll.horizontal,
            stick_mouse_enabled: stick_mouse.enabled,
            stick_mouse_sensitivity: stick_mouse.sensitivity,
            stick_mouse_dead_zone: stick_mouse.dead_zone as i16,
            mouse_stick_active,
            touchpad_enabled: touchpad.enabled,
            touchpad_sensitivity: touchpad.sensitivity,
            active_profile: Profile::Default,
            tmux_available: tmux.enabled,
            tmux: TmuxState::from_config(tmux, tmux_detected),
            opencode: OpenCodeState::from_config(opencode, opencode_detected),
            wt: WtState::from_config(wt, wt_detected),
            ..Default::default()
        }
    }

    /// Returns the currently active profile.
    pub fn profile(&self) -> Profile {
        self.active_profile
    }

    /// Given current input, return actions for newly pressed buttons and analog input.
    pub fn update(&mut self, input: &UnifiedInput) -> Vec<Action> {
        let current = &input.buttons;
        let mut actions = Vec::new();
        let now = Instant::now();

        // --- Face buttons: rising edge only ---
        macro_rules! on_press {
            ($field:ident, $action:expr) => {
                if current.$field && !self.prev.$field {
                    actions.push($action);
                }
            };
        }

        // --- Touchpad: touch → cursor movement, click → left mouse button (always active) ---
        self.process_touchpad(input, &mut actions);

        // --- Left stick → mouse cursor (always active) ---
        self.process_stick_mouse(input, &mut actions);

        // --- Always active face buttons ---
        on_press!(cross, Action::KeyCombo(vec![VKey::Return]));
        on_press!(circle, Action::KeyCombo(vec![VKey::Escape]));
        on_press!(triangle, Action::KeyCombo(vec![VKey::Tab]));

        // --- PS button: cycle profiles ---
        if current.ps && !self.prev.ps && self.tmux_available {
            self.active_profile = match self.active_profile {
                Profile::Default => Profile::Tmux,
                Profile::Tmux    => Profile::Default,
            };
            actions.push(Action::Custom(format!("profile:{}", self.active_profile)));
            log::info!("Profile switched to: {}", self.active_profile);
        }

        // --- Profile-dependent buttons ---
        match self.active_profile {
            Profile::Default => {
                // Square → Windows Terminal new tab (profile 1, auto-detected or ctrl+shift+1)
                if current.square && !self.prev.square {
                    if let Some(ref keys) = self.wt.square {
                        actions.push(Action::KeyCombo(keys.clone()));
                    }
                }
                // L1 → previous tab (auto-detected or ctrl+shift+tab)
                if current.l1 && !self.prev.l1 {
                    if let Some(ref keys) = self.wt.l1 {
                        actions.push(Action::KeyCombo(keys.clone()));
                    }
                }
                // R1 → next tab (auto-detected or ctrl+tab)
                if current.r1 && !self.prev.r1 {
                    if let Some(ref keys) = self.wt.r1 {
                        actions.push(Action::KeyCombo(keys.clone()));
                    }
                }
                // L2: hold Ctrl+Win while button is held
                if current.l2 && !self.prev.l2 {
                    actions.push(Action::KeyDown(vec![VKey::Control, VKey::Win]));
                } else if !current.l2 && self.prev.l2 {
                    actions.push(Action::KeyUp(vec![VKey::Control, VKey::Win]));
                }
                on_press!(r2, Action::KeyCombo(vec![VKey::Control, VKey::C]));
                on_press!(l3, Action::KeyCombo(vec![VKey::Control, VKey::T]));
                on_press!(r3, Action::KeyCombo(vec![VKey::Control, VKey::P]));
            }
            Profile::Tmux => {
                macro_rules! on_press_tmux {
                    ($field:ident, $keys_field:ident) => {
                        if current.$field && !self.prev.$field {
                            if let Some(ref keys) = self.tmux.$keys_field {
                                actions.push(Action::KeySequence(vec![
                                    self.tmux.prefix.clone(),
                                    keys.clone(),
                                ]));
                            }
                        }
                    };
                }

                on_press_tmux!(l1, l1);
                on_press_tmux!(r1, r1);
                on_press_tmux!(square, square);
                // L2: hold Ctrl+Win while button is held
                if current.l2 && !self.prev.l2 {
                    actions.push(Action::KeyDown(vec![VKey::Control, VKey::Win]));
                } else if !current.l2 && self.prev.l2 {
                    actions.push(Action::KeyUp(vec![VKey::Control, VKey::Win]));
                }
                on_press_tmux!(r2, r2);
                on_press!(l3, Action::KeyCombo(vec![VKey::Control, VKey::T]));
                on_press!(r3, Action::KeyCombo(vec![VKey::Control, VKey::P]));
                on_press_tmux!(share, share);
                on_press_tmux!(options, options);
                // Note: touchpad button is handled globally by process_touchpad() above.
            }

        }

        // --- D-pad with two-frame confirm + repeat ---
        let up_held = matches!(current.dpad, DPad::Up | DPad::UpLeft | DPad::UpRight);
        let down_held = matches!(current.dpad, DPad::Down | DPad::DownLeft | DPad::DownRight);
        let left_held = matches!(current.dpad, DPad::Left | DPad::UpLeft | DPad::DownLeft);
        let right_held = matches!(current.dpad, DPad::Right | DPad::UpRight | DPad::DownRight);

        let prev_up = matches!(self.prev.dpad, DPad::Up | DPad::UpLeft | DPad::UpRight);
        let prev_down = matches!(self.prev.dpad, DPad::Down | DPad::DownLeft | DPad::DownRight);
        let prev_left = matches!(self.prev.dpad, DPad::Left | DPad::UpLeft | DPad::DownLeft);
        let prev_right = matches!(self.prev.dpad, DPad::Right | DPad::UpRight | DPad::DownRight);

        macro_rules! dpad {
            ($held:expr, $prev:expr, $timer:expr, $key:expr) => {
                if $held && !$prev {
                    $timer.on_press(now);
                } else if $held {
                    if $timer.on_hold(now) {
                        actions.push(Action::KeyCombo(vec![$key]));
                    }
                } else {
                    $timer.on_release();
                }
            };
        }

        dpad!(up_held, prev_up, self.repeat_up, VKey::Up);
        dpad!(down_held, prev_down, self.repeat_down, VKey::Down);
        dpad!(left_held, prev_left, self.repeat_left, VKey::Left);
        dpad!(right_held, prev_right, self.repeat_right, VKey::Right);

        // --- Right stick → scroll ---
        self.process_scroll(input.right_stick, now, &mut actions);

        self.prev = *current;
        actions
    }

    /// Process right stick into scroll actions with dead zone and rate limiting.
    fn process_scroll(&mut self, stick: (u8, u8), now: Instant, actions: &mut Vec<Action>) {
        let (rx, ry) = stick;
        let dx = rx as i16 - 128;
        let dy = ry as i16 - 128;

        // Apply dead zone
        let dx = if dx.abs() < self.scroll_dead_zone { 0 } else { dx };
        let dy = if dy.abs() < self.scroll_dead_zone { 0 } else { dy };

        // Ignore horizontal if disabled
        let dx = if self.scroll_horizontal { dx } else { 0 };

        if dx == 0 && dy == 0 {
            self.last_scroll_at = None;
            return;
        }

        // Deflection magnitude (0.0 to 1.0)
        let max_deflection = (dx.abs().max(dy.abs()) as f32 / 127.0).min(1.0);

        // Rate limiting: more deflection → shorter interval → faster scrolling
        let interval_ms = SCROLL_MAX_INTERVAL_MS
            - ((SCROLL_MAX_INTERVAL_MS - SCROLL_MIN_INTERVAL_MS) as f32 * max_deflection) as u64;

        if let Some(last) = self.last_scroll_at {
            if now.duration_since(last).as_millis() < interval_ms as u128 {
                return;
            }
        }

        // Y: stick up (dy < 0) → scroll up (positive vertical wheel delta)
        let vertical = if dy != 0 {
            let norm = (dy as f32 / -127.0).clamp(-1.0, 1.0);
            (norm * self.scroll_sensitivity * WHEEL_DELTA as f32) as i32
        } else {
            0
        };

        // X: stick right (dx > 0) → scroll right (positive horizontal)
        let horizontal = if dx != 0 {
            let norm = (dx as f32 / 127.0).clamp(-1.0, 1.0);
            (norm * self.scroll_sensitivity * WHEEL_DELTA as f32) as i32
        } else {
            0
        };

        if vertical != 0 || horizontal != 0 {
            actions.push(Action::Scroll { horizontal, vertical });
            self.last_scroll_at = Some(now);
        }
    }

    /// Translate touchpad touch coordinates into relative mouse movement and
    /// touchpad click into a left mouse button click.
    ///
    /// Called on every frame BEFORE profile-dependent dispatch so that the
    /// touchpad works identically in both Default and Tmux profiles.
    fn process_touchpad(&mut self, input: &UnifiedInput, actions: &mut Vec<Action>) {
        if !self.touchpad_enabled {
            return; // config-level disable: suppresses both movement and click
        }

        // ── Touch movement: only in touchpad mode (not when left stick drives cursor) ──
        let stick_active = self.mouse_stick_active.load(Ordering::Relaxed);
        let tp = &input.touchpad[0];
        if tp.active && !stick_active {
            if let Some((px, py)) = self.prev_touch {
                let raw_dx = tp.x as i32 - px as i32;
                let raw_dy = tp.y as i32 - py as i32;
                let dx = (raw_dx as f32 * self.touchpad_sensitivity) as i32;
                let dy = (raw_dy as f32 * self.touchpad_sensitivity) as i32;
                if dx != 0 || dy != 0 {
                    log::debug!("TouchpadMove raw=({raw_dx},{raw_dy}) scaled=({dx},{dy})");
                    actions.push(Action::MouseMove { dx, dy });
                }
            }
            self.prev_touch = Some((tp.x, tp.y));
        } else {
            // Clear prev_touch so switching back to touchpad mode doesn't
            // produce a spurious large jump.
            self.prev_touch = None;
        }

        // ── Touchpad press → left click (always active regardless of mouse mode) ──
        if input.buttons.touchpad && !self.prev.touchpad {
            log::debug!("TouchpadClick → MouseClick");
            actions.push(Action::MouseClick);
        }
    }

    /// Translate left analog stick deflection into relative mouse movement.
    ///
    /// Velocity-based: stick position → cursor speed per frame.
    /// A sub-pixel accumulator (`stick_acc_x/y`) carries fractional pixels
    /// across frames so slow, precise movements don't stutter.
    fn process_stick_mouse(&mut self, input: &UnifiedInput, actions: &mut Vec<Action>) {
        if !self.stick_mouse_enabled || !self.mouse_stick_active.load(Ordering::Relaxed) {
            return;
        }

        let (lx, ly) = input.left_stick;
        let dx_raw = lx as i16 - 128;
        let dy_raw = ly as i16 - 128;

        // Apply dead zone per axis
        let dx_raw = if dx_raw.abs() < self.stick_mouse_dead_zone { 0 } else { dx_raw };
        let dy_raw = if dy_raw.abs() < self.stick_mouse_dead_zone { 0 } else { dy_raw };

        if dx_raw == 0 && dy_raw == 0 {
            // Reset accumulators when stick returns to center so no phantom move
            // fires when the stick is next pushed.
            self.stick_acc_x = 0.0;
            self.stick_acc_y = 0.0;
            return;
        }

        // Normalize to -1.0..1.0 and scale by sensitivity (pixels/frame at full deflection)
        let vx = (dx_raw as f32 / 127.0).clamp(-1.0, 1.0) * self.stick_mouse_sensitivity;
        let vy = (dy_raw as f32 / 127.0).clamp(-1.0, 1.0) * self.stick_mouse_sensitivity;

        // Accumulate; extract whole pixels; keep remainder for next frame
        self.stick_acc_x += vx;
        self.stick_acc_y += vy;

        let dx = self.stick_acc_x as i32;
        let dy = self.stick_acc_y as i32;

        if dx != 0 || dy != 0 {
            self.stick_acc_x -= dx as f32;
            self.stick_acc_y -= dy as f32;
            log::debug!("StickMouse move=({dx},{dy}) acc=({:.2},{:.2})", self.stick_acc_x, self.stick_acc_y);
            actions.push(Action::MouseMove { dx, dy });
        }
    }
}

// ── Windows SendInput functions ──────────────────────────────────────

/// Send a key combo via Windows SendInput. Modifiers held, main key pressed+released, modifiers released.
#[cfg(windows)]
pub fn send_key_combo(keys: &[VKey]) {
    if keys.is_empty() {
        return;
    }

    let (modifiers, main_key) = keys.split_at(keys.len() - 1);
    let mut inputs: Vec<INPUT> = Vec::with_capacity(keys.len() * 2);

    for &m in modifiers {
        inputs.push(make_key_input(m.code(), 0));
    }
    inputs.push(make_key_input(main_key[0].code(), 0));
    inputs.push(make_key_input(main_key[0].code(), KEYEVENTF_KEYUP));
    for &m in modifiers.iter().rev() {
        inputs.push(make_key_input(m.code(), KEYEVENTF_KEYUP));
    }

    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

/// Press keys down (hold). Call send_key_up to release.
#[cfg(windows)]
pub fn send_key_down(keys: &[VKey]) {
    if keys.is_empty() {
        return;
    }
    let inputs: Vec<INPUT> = keys.iter().map(|k| make_key_input(k.code(), 0)).collect();
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

/// Release held keys (reverse order for proper modifier release).
#[cfg(windows)]
pub fn send_key_up(keys: &[VKey]) {
    if keys.is_empty() {
        return;
    }
    let inputs: Vec<INPUT> = keys.iter().rev().map(|k| make_key_input(k.code(), KEYEVENTF_KEYUP)).collect();
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

/// Send a sequence of key combos with a delay between each (e.g., tmux prefix + action).
#[cfg(windows)]
pub fn send_key_sequence(combos: &[Vec<VKey>], delay_ms: u64) {
    for (i, combo) in combos.iter().enumerate() {
        send_key_combo(combo);
        if i < combos.len() - 1 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
    }
}

/// Move the mouse cursor by a relative offset via Windows SendInput.
#[cfg(windows)]
pub fn send_mouse_move(dx: i32, dy: i32) {
    let input = make_mouse_move_input(dx, dy);
    unsafe {
        SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Send a left mouse button click (down + up) via Windows SendInput.
#[cfg(windows)]
pub fn send_mouse_click() {
    let inputs = [
        make_mouse_flag_input(MOUSEEVENTF_LEFTDOWN),
        make_mouse_flag_input(MOUSEEVENTF_LEFTUP),
    ];
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

/// Send a mouse scroll event via Windows SendInput.
#[cfg(windows)]
pub fn send_scroll(horizontal: i32, vertical: i32) {
    let mut inputs: Vec<INPUT> = Vec::new();

    if vertical != 0 {
        inputs.push(make_mouse_input(MOUSEEVENTF_WHEEL, vertical));
    }
    if horizontal != 0 {
        inputs.push(make_mouse_input(MOUSEEVENTF_HWHEEL, horizontal));
    }

    if !inputs.is_empty() {
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }
}

#[cfg(windows)]
fn make_key_input(vk: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(windows)]
fn make_mouse_input(flags: u32, wheel_delta: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: wheel_delta as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Build a relative mouse-move INPUT struct.
#[cfg(windows)]
fn make_mouse_move_input(dx: i32, dy: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Build a mouse button INPUT struct (no dx/dy, no wheel data).
#[cfg(windows)]
fn make_mouse_flag_input(flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Execute an action (send keystrokes, scroll, mouse movement/click, or handle custom actions).
#[cfg(windows)]
pub fn execute_action(action: &Action) {
    match action {
        Action::KeyCombo(keys) => send_key_combo(keys),
        Action::KeyDown(keys) => send_key_down(keys),
        Action::KeyUp(keys) => send_key_up(keys),
        Action::KeySequence(combos) => send_key_sequence(combos, 10),
        Action::Scroll { horizontal, vertical } => send_scroll(*horizontal, *vertical),
        Action::MouseMove { dx, dy } => send_mouse_move(*dx, *dy),
        Action::MouseClick => send_mouse_click(),
        Action::Custom(name) => {
            log::info!("Custom action triggered: {name}");
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn input_with(f: impl FnOnce(&mut UnifiedInput)) -> UnifiedInput {
        let mut input = UnifiedInput::default();
        f(&mut input);
        input
    }

    #[test]
    fn detects_rising_edge() {
        let mut mapper = MapperState::default();

        let actions = mapper.update(&UnifiedInput::default());
        assert!(actions.is_empty());

        let input = input_with(|i| i.buttons.cross = true);
        let actions = mapper.update(&input);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::KeyCombo(keys) => assert_eq!(keys, &[VKey::Return]),
            _ => panic!("Expected KeyCombo"),
        }

        // Hold: no new action
        let actions = mapper.update(&input);
        assert!(actions.is_empty());

        // Release
        let actions = mapper.update(&UnifiedInput::default());
        assert!(actions.is_empty());
    }

    #[test]
    fn dpad_two_frame_confirm() {
        let mut mapper = MapperState::default();

        // Frame 1: press Up — pending, no fire
        let input = input_with(|i| i.buttons.dpad = DPad::Up);
        let actions = mapper.update(&input);
        assert!(actions.is_empty(), "Should not fire on first frame");

        // Frame 2: still held — confirmed, fires
        let actions = mapper.update(&input);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::KeyCombo(keys) => assert_eq!(keys, &[VKey::Up]),
            _ => panic!("Expected KeyCombo"),
        }

        // Frame 3: still held — no repeat yet
        let actions = mapper.update(&input);
        assert!(actions.is_empty());

        // Release
        let actions = mapper.update(&UnifiedInput::default());
        assert!(actions.is_empty());
    }

    #[test]
    fn dpad_single_frame_glitch_filtered() {
        let mut mapper = MapperState::default();

        let input = input_with(|i| i.buttons.dpad = DPad::Up);
        let actions = mapper.update(&input);
        assert!(actions.is_empty(), "Pending, not fired");

        let actions = mapper.update(&UnifiedInput::default());
        assert!(actions.is_empty(), "Single-frame glitch should not fire");
    }

    #[test]
    fn l1_produces_prev_tab() {
        // Default profile: L1 → Windows Terminal prevTab (ctrl+shift+tab)
        let mut mapper = MapperState::default();
        let input = input_with(|i| i.buttons.l1 = true);
        let actions = mapper.update(&input);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::KeyCombo(keys) => assert_eq!(keys, &[VKey::Control, VKey::Shift, VKey::Tab]),
            _ => panic!("Expected KeyCombo"),
        }
    }

    #[test]
    fn square_produces_wt_new_tab() {
        // Default profile: Square → Windows Terminal newTab (ctrl+shift+1)
        let mut mapper = MapperState::default();
        let input = input_with(|i| i.buttons.square = true);
        let actions = mapper.update(&input);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::KeyCombo(keys) => assert_eq!(
                keys,
                &[VKey::Control, VKey::Shift, VKey::D1],
                "Expected ctrl+shift+1 for newTab"
            ),
            _ => panic!("Expected KeyCombo(ctrl+shift+1)"),
        }
    }

    #[test]
    fn scroll_dead_zone_no_action() {
        let mut mapper = MapperState::default();

        // Center stick
        let input = input_with(|i| i.right_stick = (128, 128));
        let actions = mapper.update(&input);
        assert!(!actions.iter().any(|a| matches!(a, Action::Scroll { .. })));

        // Small deflection within dead zone (±20)
        let input = input_with(|i| i.right_stick = (138, 138));
        let actions = mapper.update(&input);
        assert!(!actions.iter().any(|a| matches!(a, Action::Scroll { .. })));
    }

    #[test]
    fn scroll_beyond_dead_zone_fires() {
        let mut mapper = MapperState::default();

        // Stick up (ry=80, deflection=48 > dead_zone=20)
        let input = input_with(|i| i.right_stick = (128, 80));
        let actions = mapper.update(&input);
        assert!(
            actions.iter().any(|a| matches!(a, Action::Scroll { vertical, .. } if *vertical > 0)),
            "Expected positive vertical scroll for stick-up"
        );
    }

    #[test]
    fn scroll_rate_limited() {
        let mut mapper = MapperState::default();

        let input = input_with(|i| i.right_stick = (128, 0));
        let a1 = mapper.update(&input);
        assert!(a1.iter().any(|a| matches!(a, Action::Scroll { .. })));

        // Immediate second call: rate-limited
        let a2 = mapper.update(&input);
        assert!(!a2.iter().any(|a| matches!(a, Action::Scroll { .. })));
    }

    #[test]
    fn scroll_down_negative_vertical() {
        let mut mapper = MapperState::default();

        // Stick down (ry=200, deflection=72 > dead_zone=20)
        let input = input_with(|i| i.right_stick = (128, 200));
        let actions = mapper.update(&input);
        assert!(
            actions.iter().any(|a| matches!(a, Action::Scroll { vertical, .. } if *vertical < 0)),
            "Expected negative vertical scroll for stick-down"
        );
    }

    /// Helper: activate tmux profile by pressing PS.
    fn switch_to_tmux(mapper: &mut MapperState) {
        let ps_press = input_with(|i| i.buttons.ps = true);
        let actions = mapper.update(&ps_press);
        assert!(actions.iter().any(|a| matches!(a, Action::Custom(s) if s == "profile:tmux")));
        assert_eq!(mapper.profile(), Profile::Tmux);
        // Release PS
        mapper.update(&UnifiedInput::default());
    }

    #[test]
    fn ps_cycles_profiles() {
        let mut mapper = MapperState::default();
        assert_eq!(mapper.profile(), Profile::Default);

        // Press PS → switch to Tmux
        switch_to_tmux(&mut mapper);

        // Press PS again → back to Default
        let ps_press = input_with(|i| i.buttons.ps = true);
        let actions = mapper.update(&ps_press);
        assert!(actions.iter().any(|a| matches!(a, Action::Custom(s) if s == "profile:default")));
        assert_eq!(mapper.profile(), Profile::Default);
    }

    #[test]
    fn default_profile_l2_does_nothing() {
        let mut mapper = MapperState::default();
        assert_eq!(mapper.profile(), Profile::Default);

        let input = input_with(|i| i.buttons.l2 = true);
        let actions = mapper.update(&input);
        assert!(!actions.iter().any(|a| matches!(a, Action::KeySequence(_))));
    }

    #[test]
    fn tmux_l1_fires_key_sequence() {
        let mut mapper = MapperState::default();
        switch_to_tmux(&mut mapper);

        let input = input_with(|i| i.buttons.l1 = true);
        let actions = mapper.update(&input);

        let tmux_actions: Vec<_> = actions.iter()
            .filter(|a| matches!(a, Action::KeySequence(_)))
            .collect();
        assert_eq!(tmux_actions.len(), 1);

        match &tmux_actions[0] {
            Action::KeySequence(seq) => {
                assert_eq!(seq.len(), 2);
                assert_eq!(seq[0], vec![VKey::Control, VKey::B]);
                assert_eq!(seq[1], vec![VKey::P]);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn tmux_disabled_ps_does_nothing() {
        let scroll_cfg = ScrollConfig::default();
        let mut tmux_cfg = TmuxConfig::default();
        tmux_cfg.enabled = false;
        let mut mapper = MapperState::new(&scroll_cfg, &crate::config::StickMouseConfig::default(), &crate::config::TouchpadConfig::default(), &tmux_cfg, None, &crate::config::OpenCodeConfig::default(), None, &crate::config::WtConfig::default(), None, Arc::new(AtomicBool::new(false)));

        // PS press should not switch profiles
        let ps_press = input_with(|i| i.buttons.ps = true);
        let actions = mapper.update(&ps_press);
        assert!(!actions.iter().any(|a| matches!(a, Action::Custom(s) if s.starts_with("profile:"))));
        assert_eq!(mapper.profile(), Profile::Default);
    }

    #[test]
    fn tmux_mapped_buttons() {
        let mut mapper = MapperState::default();
        switch_to_tmux(&mut mapper);

        let tests: Vec<(fn(&mut UnifiedInput), Vec<VKey>)> = vec![
            (|i| i.buttons.l1 = true, vec![VKey::P]),                   // prev window
            (|i| i.buttons.r1 = true, vec![VKey::N]),                   // next window
            (|i| i.buttons.r2 = true, vec![VKey::Shift, VKey::D7]),     // kill window (&)
            (|i| i.buttons.square = true, vec![VKey::C]),               // new window
        ];

        for (setup, expected_action) in tests {
            mapper = MapperState::default();
            switch_to_tmux(&mut mapper);
            let input = input_with(setup);
            let actions = mapper.update(&input);
            let seq: Vec<_> = actions.iter()
                .filter_map(|a| match a { Action::KeySequence(s) => Some(s), _ => None })
                .collect();
            assert_eq!(seq.len(), 1, "Expected 1 KeySequence for button");
            assert_eq!(seq[0][0], vec![VKey::Control, VKey::B], "Wrong prefix");
            assert_eq!(seq[0][1], expected_action, "Wrong action key");
        }
    }

    #[test]
    fn tmux_unmapped_buttons_do_nothing() {
        let mut mapper = MapperState::default();
        switch_to_tmux(&mut mapper);

        // These buttons are unmapped in the default tmux config
        let unmapped: Vec<fn(&mut UnifiedInput)> = vec![
            |i| i.buttons.share = true,
            |i| i.buttons.options = true,
            |i| i.buttons.touchpad = true,
        ];

        for setup in unmapped {
            mapper = MapperState::default();
            switch_to_tmux(&mut mapper);
            let input = input_with(setup);
            let actions = mapper.update(&input);
            assert!(
                !actions.iter().any(|a| matches!(a, Action::KeySequence(_))),
                "Unmapped button should not fire KeySequence"
            );
        }
    }

    #[test]
    fn r3_ctrl_p_both_profiles() {
        // Default profile
        let mut mapper = MapperState::default();
        let input = input_with(|i| i.buttons.r3 = true);
        let actions = mapper.update(&input);
        assert!(
            actions.iter().any(|a| matches!(a, Action::KeyCombo(k) if *k == vec![VKey::Control, VKey::P])),
            "R3 should send Ctrl+P in Default profile"
        );

        // Tmux profile
        let mut mapper = MapperState::default();
        switch_to_tmux(&mut mapper);
        let input = input_with(|i| i.buttons.r3 = true);
        let actions = mapper.update(&input);
        assert!(
            actions.iter().any(|a| matches!(a, Action::KeyCombo(k) if *k == vec![VKey::Control, VKey::P])),
            "R3 should send Ctrl+P in Tmux profile"
        );
    }

    #[test]
    fn l3_ctrl_t_both_profiles() {
        // Default profile
        let mut mapper = MapperState::default();
        let input = input_with(|i| i.buttons.l3 = true);
        let actions = mapper.update(&input);
        assert!(
            actions.iter().any(|a| matches!(a, Action::KeyCombo(k) if *k == vec![VKey::Control, VKey::T])),
            "L3 should send Ctrl+T in Default profile"
        );

        // Tmux profile
        let mut mapper = MapperState::default();
        switch_to_tmux(&mut mapper);
        let input = input_with(|i| i.buttons.l3 = true);
        let actions = mapper.update(&input);
        assert!(
            actions.iter().any(|a| matches!(a, Action::KeyCombo(k) if *k == vec![VKey::Control, VKey::T])),
            "L3 should send Ctrl+T in Tmux profile"
        );
    }

    #[test]
    fn tmux_overrides_default_l1() {
        let mut mapper = MapperState::default();

        // Default profile: L1 → Windows Terminal prevTab (ctrl+shift+tab)
        let input = input_with(|i| i.buttons.l1 = true);
        let actions = mapper.update(&input);
        assert!(actions.iter().any(|a| matches!(a, Action::KeyCombo(k) if *k == vec![VKey::Control, VKey::Shift, VKey::Tab])));

        // Release and switch to tmux
        mapper.update(&UnifiedInput::default());
        switch_to_tmux(&mut mapper);

        // Tmux profile: L1 → prefix + P
        let actions = mapper.update(&input);
        let seq: Vec<_> = actions.iter()
            .filter_map(|a| match a { Action::KeySequence(s) => Some(s), _ => None })
            .collect();
        assert_eq!(seq.len(), 1);
        assert_eq!(seq[0][1], vec![VKey::P]);
    }

    #[test]
    fn tmux_overrides_default_square() {
        let mut mapper = MapperState::default();

        // Default profile: Square → Windows Terminal newTab (ctrl+shift+1)
        let input = input_with(|i| i.buttons.square = true);
        let actions = mapper.update(&input);
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::KeyCombo(k) if *k == vec![VKey::Control, VKey::Shift, VKey::D1]
        )));

        // Release and switch to tmux
        mapper.update(&UnifiedInput::default());
        switch_to_tmux(&mut mapper);

        // Tmux profile: Square → prefix + C
        let actions = mapper.update(&input);
        let seq: Vec<_> = actions.iter()
            .filter_map(|a| match a { Action::KeySequence(s) => Some(s), _ => None })
            .collect();
        assert_eq!(seq.len(), 1);
        assert_eq!(seq[0][1], vec![VKey::C]);
    }

    #[test]
    fn parse_combo_ctrl_b() {
        let combo = parse_key_combo("Ctrl+B").unwrap();
        assert_eq!(combo, vec![VKey::Control, VKey::B]);
    }

    #[test]
    fn parse_single_key() {
        let combo = parse_key_combo("p").unwrap();
        assert_eq!(combo, vec![VKey::P]);
    }

    // ── Touchpad tests ────────────────────────────────────────────────

    fn input_with_touch(x: u16, y: u16, click: bool) -> UnifiedInput {
        let mut i = UnifiedInput::default();
        i.touchpad[0] = crate::input::TouchPoint { active: true, x, y };
        i.buttons.touchpad = click;
        i
    }

    #[test]
    fn touchpad_first_frame_no_move() {
        let mut mapper = MapperState::default();
        // First frame of active touch: no prev → no MouseMove emitted
        let input = input_with_touch(500, 300, false);
        let actions = mapper.update(&input);
        assert!(
            !actions.iter().any(|a| matches!(a, Action::MouseMove { .. })),
            "No MouseMove on first touch frame"
        );
    }

    #[test]
    fn touchpad_second_frame_emits_move() {
        let mut mapper = MapperState::default();
        mapper.update(&input_with_touch(500, 300, false));
        // Second frame: moved right 10, down 5
        let actions = mapper.update(&input_with_touch(510, 305, false));
        let moves: Vec<_> = actions.iter()
            .filter_map(|a| match a { Action::MouseMove { dx, dy } => Some((*dx, *dy)), _ => None })
            .collect();
        assert_eq!(moves.len(), 1, "Expected one MouseMove");
        // With default sensitivity 1.5: dx=(10*1.5)=15, dy=(5*1.5)=7
        assert_eq!(moves[0], (15, 7));
    }

    #[test]
    fn touchpad_lift_clears_prev() {
        let mut mapper = MapperState::default();
        mapper.update(&input_with_touch(500, 300, false));
        mapper.update(&input_with_touch(510, 305, false));
        // Lift
        mapper.update(&UnifiedInput::default());
        assert!(mapper.prev_touch.is_none(), "prev_touch cleared after lift");
        // Re-touch at a new position: should NOT emit move (no prev)
        let actions = mapper.update(&input_with_touch(900, 600, false));
        assert!(
            !actions.iter().any(|a| matches!(a, Action::MouseMove { .. })),
            "No move on re-touch after lift"
        );
    }

    #[test]
    fn touchpad_no_move_when_stationary() {
        let mut mapper = MapperState::default();
        mapper.update(&input_with_touch(500, 300, false));
        // Same position: raw delta = 0,0 → scaled = 0,0 → no action
        let actions = mapper.update(&input_with_touch(500, 300, false));
        assert!(!actions.iter().any(|a| matches!(a, Action::MouseMove { .. })));
    }

    #[test]
    fn touchpad_click_rising_edge() {
        let mut mapper = MapperState::default();
        let input = input_with_touch(500, 300, true);
        let actions = mapper.update(&input);
        assert!(
            actions.iter().any(|a| matches!(a, Action::MouseClick)),
            "MouseClick on first press frame"
        );
        // Hold: no second click
        let actions = mapper.update(&input);
        assert!(
            !actions.iter().any(|a| matches!(a, Action::MouseClick)),
            "No click on hold"
        );
    }

    #[test]
    fn touchpad_disabled_no_actions() {
        let mut mapper = MapperState::default();
        mapper.touchpad_enabled = false;
        let input = input_with_touch(500, 300, true);
        mapper.update(&input_with_touch(400, 200, false)); // set prev_touch (should be skipped)
        let actions = mapper.update(&input);
        assert!(!actions.iter().any(|a| matches!(a, Action::MouseMove { .. })));
        assert!(!actions.iter().any(|a| matches!(a, Action::MouseClick)));
    }

    // ── Left stick mouse tests ────────────────────────────────────────

    fn input_with_left_stick(lx: u8, ly: u8) -> UnifiedInput {
        let mut i = UnifiedInput::default();
        i.left_stick = (lx, ly);
        i
    }

    #[test]
    fn stick_mouse_center_no_action() {
        let mut mapper = MapperState::default();
        // Centered stick → no MouseMove
        let actions = mapper.update(&input_with_left_stick(128, 128));
        assert!(!actions.iter().any(|a| matches!(a, Action::MouseMove { .. })));
    }

    #[test]
    fn stick_mouse_dead_zone_no_action() {
        let mut mapper = MapperState::default();
        // Deflection of 10 < dead_zone (15) → no move
        let actions = mapper.update(&input_with_left_stick(138, 118));
        assert!(!actions.iter().any(|a| matches!(a, Action::MouseMove { .. })));
    }

    /// Helper: activate stick mouse mode for tests.
    fn enable_stick_mode(mapper: &MapperState) {
        mapper.mouse_stick_active.store(true, Ordering::Relaxed);
    }

    #[test]
    fn stick_mouse_beyond_dead_zone_emits_move() {
        let mut mapper = MapperState::default();
        enable_stick_mode(&mapper);
        // Full right deflection (lx=255, dy_raw=127 > dead_zone=15)
        let actions = mapper.update(&input_with_left_stick(255, 128));
        assert!(
            actions.iter().any(|a| matches!(a, Action::MouseMove { dx, .. } if *dx > 0)),
            "Full right deflection should produce positive dx"
        );
    }

    #[test]
    fn stick_mouse_direction_up_negative_dy() {
        let mut mapper = MapperState::default();
        enable_stick_mode(&mapper);
        // Full up deflection (ly=0, dy_raw=-128 < 0)
        let actions = mapper.update(&input_with_left_stick(128, 0));
        assert!(
            actions.iter().any(|a| matches!(a, Action::MouseMove { dy, .. } if *dy < 0)),
            "Full up deflection should produce negative dy"
        );
    }

    #[test]
    fn stick_mouse_accumulates_subpixel() {
        // sensitivity=0.3, dx_raw=64 → vx≈0.151 px/frame → needs ~7 frames to cross 1px
        let mut mapper = MapperState::default();
        enable_stick_mode(&mapper);
        mapper.stick_mouse_sensitivity = 0.3;
        mapper.stick_mouse_dead_zone = 0;

        let input = input_with_left_stick(192, 128); // dx_raw=64
        let fired = (0..10).any(|_| {
            mapper.update(&input).iter().any(|a| matches!(a, Action::MouseMove { dx, .. } if *dx > 0))
        });
        assert!(fired, "Sub-pixel accumulator should emit move after enough frames");
    }

    #[test]
    fn stick_mouse_acc_resets_at_center() {
        let mut mapper = MapperState::default();
        enable_stick_mode(&mapper);
        // Push right, then center
        mapper.update(&input_with_left_stick(255, 128));
        mapper.update(&UnifiedInput::default()); // center
        assert_eq!(mapper.stick_acc_x, 0.0, "Accumulator should reset at center");
        assert_eq!(mapper.stick_acc_y, 0.0);
    }

    #[test]
    fn stick_mouse_disabled_no_actions() {
        let mut mapper = MapperState::default();
        // stick_mouse_enabled=false overrides even if stick mode is selected
        mapper.stick_mouse_enabled = false;
        enable_stick_mode(&mapper);
        let actions = mapper.update(&input_with_left_stick(255, 0));
        assert!(!actions.iter().any(|a| matches!(a, Action::MouseMove { .. })));
    }

    // ── Mouse mode switching tests ────────────────────────────────────

    #[test]
    fn stick_mode_off_suppresses_stick_move() {
        let mut mapper = MapperState::default();
        // Default: stick mode off → full stick deflection produces no MouseMove
        let actions = mapper.update(&input_with_left_stick(255, 128));
        assert!(
            !actions.iter().any(|a| matches!(a, Action::MouseMove { .. })),
            "Stick should not move cursor when stick mode is off"
        );
    }

    #[test]
    fn stick_mode_on_suppresses_touchpad_move() {
        let mut mapper = MapperState::default();
        enable_stick_mode(&mapper);
        // Prime prev_touch as if we were in touchpad mode, then switch
        mapper.prev_touch = Some((500, 300));
        // Touchpad touch should NOT emit MouseMove when stick mode is active
        let actions = mapper.update(&input_with_touch(510, 305, false));
        assert!(
            !actions.iter().any(|a| matches!(a, Action::MouseMove { .. })),
            "Touchpad touch should not move cursor when stick mode is on"
        );
    }

    #[test]
    fn touchpad_click_always_fires_in_stick_mode() {
        let mut mapper = MapperState::default();
        enable_stick_mode(&mapper);
        // Touchpad press → click must fire even in stick mode
        let actions = mapper.update(&input_with_touch(500, 300, true));
        assert!(
            actions.iter().any(|a| matches!(a, Action::MouseClick)),
            "Touchpad click must fire regardless of mouse mode"
        );
        // But no touch movement
        assert!(
            !actions.iter().any(|a| matches!(a, Action::MouseMove { .. })),
            "No touch movement in stick mode"
        );
    }

    #[test]
    fn switching_modes_clears_prev_touch() {
        let mut mapper = MapperState::default();
        // Establish prev_touch in touchpad mode
        mapper.update(&input_with_touch(500, 300, false));
        assert!(mapper.prev_touch.is_some());
        // Switch to stick mode — next frame clears prev_touch
        enable_stick_mode(&mapper);
        mapper.update(&input_with_touch(510, 305, false));
        assert!(mapper.prev_touch.is_none(), "prev_touch must clear when mode switches to stick");
    }

    #[test]
    fn vkey_from_name_coverage() {
        assert_eq!(VKey::from_name("enter"), Some(VKey::Return));
        assert_eq!(VKey::from_name("Ctrl"), Some(VKey::Control));
        assert_eq!(VKey::from_name(";"), Some(VKey::Semicolon));
        assert_eq!(VKey::from_name("["), Some(VKey::LeftBracket));
        assert_eq!(VKey::from_name("z"), Some(VKey::Z));
        assert_eq!(VKey::from_name("unknown"), None);
    }
}
