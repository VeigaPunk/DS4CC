/// Button mapper: translates UnifiedInput → keyboard/mouse events via SendInput.
///
/// Always active (both profiles):
///   D-pad Up/Down/Left/Right → Arrow keys (two-frame confirm + repeat)
///   Cross    → Enter
///   Circle   → Escape
///   Triangle → Tab
///   Right stick → Mouse scroll wheel (vertical + horizontal)
///   PS       → Cycle profiles (Default ↔ Tmux)
///
/// Default profile:
///   Square   → Spawn new session (custom action)
///   L1       → Shift+Alt+Tab (previous window)
///   R1       → Alt+Tab (next window)
///   R2       → Ctrl+C
///   L3       → Ctrl+T
///   R3       → Ctrl+P
///
/// Tmux profile (auto-detected from tmux config):
///   L1       → tmux prefix + previous-window key
///   R1       → tmux prefix + next-window key
///   R2       → tmux prefix + kill-window key
///   R3       → Ctrl+P
///   Square   → tmux prefix + new-window key
///   L3       → Ctrl+T
///
/// Combos are sent atomically in a single SendInput call.

use crate::config::{ScrollConfig, TmuxConfig};
use crate::input::{ButtonState, DPad, UnifiedInput};
use crate::tmux_detect::TmuxDetected;
use std::time::Instant;

#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, MOUSEINPUT,
    KEYEVENTF_KEYUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_HWHEEL,
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
            _ => None,
        }
    }
}

/// Parse a key combo string like "Ctrl+B" or "p" into a Vec<VKey>.
pub fn parse_key_combo(s: &str) -> Option<Vec<VKey>> {
    s.split('+').map(|part| VKey::from_name(part.trim())).collect()
}

/// Active input profile. PS button cycles between these.
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
            Profile::Tmux => f.write_str("tmux"),
        }
    }
}

/// An action the mapper can produce.
#[derive(Debug, Clone)]
pub enum Action {
    /// Press and release a key combo (modifiers held, main key pressed+released, modifiers released).
    KeyCombo(Vec<VKey>),
    /// Sequence of key combos with a delay between each (for tmux prefix+key).
    KeySequence(Vec<Vec<VKey>>),
    /// Mouse scroll event. Values in wheel-delta units (positive = up/right).
    Scroll { horizontal: i32, vertical: i32 },
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
    // Profile system
    active_profile: Profile,
    tmux_available: bool, // false = only Default profile, PS does nothing
    tmux: TmuxState,
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
            active_profile: Profile::Default,
            tmux_available: true,
            tmux: TmuxState::default(),
        }
    }
}

impl MapperState {
    /// Create a mapper with config-driven scroll and tmux settings.
    /// If `detected` is provided, tmux prefix and action keys are auto-resolved.
    pub fn new(scroll: &ScrollConfig, tmux: &TmuxConfig, detected: Option<&TmuxDetected>) -> Self {
        Self {
            scroll_dead_zone: scroll.dead_zone as i16,
            scroll_sensitivity: scroll.sensitivity,
            scroll_horizontal: scroll.horizontal,
            active_profile: Profile::Default,
            tmux_available: tmux.enabled,
            tmux: TmuxState::from_config(tmux, detected),
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

        // --- Always active face buttons ---
        on_press!(cross, Action::KeyCombo(vec![VKey::Return]));
        on_press!(circle, Action::KeyCombo(vec![VKey::Escape]));
        on_press!(triangle, Action::KeyCombo(vec![VKey::Tab]));

        // --- PS button: cycle profiles ---
        if current.ps && !self.prev.ps && self.tmux_available {
            self.active_profile = match self.active_profile {
                Profile::Default => Profile::Tmux,
                Profile::Tmux => Profile::Default,
            };
            actions.push(Action::Custom(format!("profile:{}", self.active_profile)));
            log::info!("Profile switched to: {}", self.active_profile);
        }

        // --- Profile-dependent buttons ---
        match self.active_profile {
            Profile::Default => {
                on_press!(square, Action::Custom("new_session".into()));
                on_press!(l1, Action::KeyCombo(vec![VKey::Shift, VKey::Alt, VKey::Tab]));
                on_press!(r1, Action::KeyCombo(vec![VKey::Alt, VKey::Tab]));
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
                on_press_tmux!(l2, l2);
                on_press_tmux!(r2, r2);
                on_press!(l3, Action::KeyCombo(vec![VKey::Control, VKey::T]));
                on_press!(r3, Action::KeyCombo(vec![VKey::Control, VKey::P]));
                on_press_tmux!(share, share);
                on_press_tmux!(options, options);
                on_press_tmux!(touchpad, touchpad);
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

/// Execute an action (send keystrokes, scroll, or handle custom actions).
#[cfg(windows)]
pub fn execute_action(action: &Action) {
    match action {
        Action::KeyCombo(keys) => send_key_combo(keys),
        Action::KeySequence(combos) => send_key_sequence(combos, 10),
        Action::Scroll { horizontal, vertical } => send_scroll(*horizontal, *vertical),
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
    fn l1_produces_shift_alt_tab() {
        let mut mapper = MapperState::default();
        let input = input_with(|i| i.buttons.l1 = true);
        let actions = mapper.update(&input);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::KeyCombo(keys) => assert_eq!(keys, &[VKey::Shift, VKey::Alt, VKey::Tab]),
            _ => panic!("Expected KeyCombo"),
        }
    }

    #[test]
    fn square_produces_custom() {
        let mut mapper = MapperState::default();
        let input = input_with(|i| i.buttons.square = true);
        let actions = mapper.update(&input);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::Custom(name) => assert_eq!(name, "new_session"),
            _ => panic!("Expected Custom"),
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
        let mut mapper = MapperState::new(&scroll_cfg, &tmux_cfg, None);

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
            |i| i.buttons.l2 = true,
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

        // Default profile: L1 → Shift+Alt+Tab
        let input = input_with(|i| i.buttons.l1 = true);
        let actions = mapper.update(&input);
        assert!(actions.iter().any(|a| matches!(a, Action::KeyCombo(k) if *k == vec![VKey::Shift, VKey::Alt, VKey::Tab])));

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

        // Default profile: Square → Custom("new_session")
        let input = input_with(|i| i.buttons.square = true);
        let actions = mapper.update(&input);
        assert!(actions.iter().any(|a| matches!(a, Action::Custom(s) if s == "new_session")));

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
