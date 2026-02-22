/// Button mapper: translates UnifiedInput → keyboard events via SendInput.
///
/// Default mappings from spec:
///   D-pad Up/Down/Left/Right → Arrow keys
///   Cross    → Enter
///   Circle   → Escape
///   Square   → Spawn new session (custom action)
///   Triangle → Tab
///   L1       → Shift+Alt+Tab (previous window)
///   R1       → Alt+Tab (next window)
///
/// Combos are sent atomically in a single SendInput call.

use crate::input::{ButtonState, DPad};
use std::time::Instant;

#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    VK_RETURN, VK_ESCAPE, VK_TAB, VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT,
    VK_MENU, VK_SHIFT,
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
        }
    }
}

/// An action the mapper can produce.
#[derive(Debug, Clone)]
pub enum Action {
    /// Press and release a sequence of keys (modifiers held, then main key pressed+released, then modifiers released).
    KeyCombo(Vec<VKey>),
    /// Custom action identifier (e.g., "new_session").
    Custom(String),
}

/// Key repeat timing configuration.
const REPEAT_DELAY_MS: u64 = 300;  // Hold this long before repeating starts
const REPEAT_RATE_MS: u64 = 100;   // Interval between repeats once started

/// Per-button repeat tracking with two-frame confirmation.
/// The first frame of a new press is treated as "pending" — only fires
/// if the direction is still held on the next frame. This filters out
/// single-frame glitches from hat switch bounce (~8ms latency, unnoticeable).
#[derive(Clone, Default)]
struct RepeatTimer {
    /// Set on the first frame of a new press; confirmed on the second frame.
    pending_since: Option<Instant>,
    /// When the confirmed press started (None = not held).
    pressed_at: Option<Instant>,
    /// When the last action was fired.
    last_fired: Option<Instant>,
}

impl RepeatTimer {
    /// Called when button is newly pressed (first frame). Marks as pending.
    fn on_press(&mut self, now: Instant) {
        self.pending_since = Some(now);
    }

    /// Called each frame while button is held. Returns true if an action should fire.
    fn on_hold(&mut self, now: Instant) -> bool {
        // Confirm pending press (second consecutive frame)
        if let Some(pending) = self.pending_since.take() {
            self.pressed_at = Some(pending);
            self.last_fired = Some(now);
            return true;
        }

        // Normal hold-repeat logic
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

    /// Called when button is released.
    fn on_release(&mut self) {
        self.pressed_at = None;
        self.pending_since = None;
    }
}

/// Track which buttons were pressed last frame to detect edges (rising edge = newly pressed).
pub struct MapperState {
    prev: ButtonState,
    // Repeat timers for d-pad directions
    repeat_up: RepeatTimer,
    repeat_down: RepeatTimer,
    repeat_left: RepeatTimer,
    repeat_right: RepeatTimer,
}

impl Default for MapperState {
    fn default() -> Self {
        Self {
            prev: ButtonState::default(),
            repeat_up: RepeatTimer::default(),
            repeat_down: RepeatTimer::default(),
            repeat_left: RepeatTimer::default(),
            repeat_right: RepeatTimer::default(),
        }
    }
}

impl MapperState {
    /// Given current button state, return actions for newly pressed buttons.
    /// D-pad uses repeat timers: fire once on press, wait REPEAT_DELAY, then repeat at REPEAT_RATE.
    pub fn update(&mut self, current: &ButtonState) -> Vec<Action> {
        let mut actions = Vec::new();
        let now = Instant::now();

        // Helper: detect rising edge for non-dpad buttons (no repeat needed)
        macro_rules! on_press {
            ($field:ident, $action:expr) => {
                if current.$field && !self.prev.$field {
                    actions.push($action);
                }
            };
        }

        on_press!(cross, Action::KeyCombo(vec![VKey::Return]));
        on_press!(circle, Action::KeyCombo(vec![VKey::Escape]));
        on_press!(triangle, Action::KeyCombo(vec![VKey::Tab]));
        on_press!(square, Action::Custom("new_session".into()));
        on_press!(l1, Action::KeyCombo(vec![VKey::Shift, VKey::Alt, VKey::Tab]));
        on_press!(r1, Action::KeyCombo(vec![VKey::Alt, VKey::Tab]));

        // D-pad with repeat timers
        let up_held = matches!(current.dpad, DPad::Up | DPad::UpLeft | DPad::UpRight);
        let down_held = matches!(current.dpad, DPad::Down | DPad::DownLeft | DPad::DownRight);
        let left_held = matches!(current.dpad, DPad::Left | DPad::UpLeft | DPad::DownLeft);
        let right_held = matches!(current.dpad, DPad::Right | DPad::UpRight | DPad::DownRight);

        let prev_up = matches!(self.prev.dpad, DPad::Up | DPad::UpLeft | DPad::UpRight);
        let prev_down = matches!(self.prev.dpad, DPad::Down | DPad::DownLeft | DPad::DownRight);
        let prev_left = matches!(self.prev.dpad, DPad::Left | DPad::UpLeft | DPad::DownLeft);
        let prev_right = matches!(self.prev.dpad, DPad::Right | DPad::UpRight | DPad::DownRight);

        // Up
        if up_held && !prev_up {
            self.repeat_up.on_press(now);
        } else if up_held {
            if self.repeat_up.on_hold(now) {
                actions.push(Action::KeyCombo(vec![VKey::Up]));
            }
        } else if !up_held {
            self.repeat_up.on_release();
        }

        // Down
        if down_held && !prev_down {
            self.repeat_down.on_press(now);
        } else if down_held {
            if self.repeat_down.on_hold(now) {
                actions.push(Action::KeyCombo(vec![VKey::Down]));
            }
        } else if !down_held {
            self.repeat_down.on_release();
        }

        // Left
        if left_held && !prev_left {
            self.repeat_left.on_press(now);
        } else if left_held {
            if self.repeat_left.on_hold(now) {
                actions.push(Action::KeyCombo(vec![VKey::Left]));
            }
        } else if !left_held {
            self.repeat_left.on_release();
        }

        // Right
        if right_held && !prev_right {
            self.repeat_right.on_press(now);
        } else if right_held {
            if self.repeat_right.on_hold(now) {
                actions.push(Action::KeyCombo(vec![VKey::Right]));
            }
        } else if !right_held {
            self.repeat_right.on_release();
        }

        self.prev = *current;
        actions
    }
}

/// Send a key combo via Windows SendInput. Modifiers are everything except the last key.
/// All keys are sent atomically in a single SendInput call.
#[cfg(windows)]
pub fn send_key_combo(keys: &[VKey]) {
    if keys.is_empty() {
        return;
    }

    let (modifiers, main_key) = keys.split_at(keys.len() - 1);

    // Build INPUT array: modifier downs + main key down + main key up + modifier ups
    let mut inputs: Vec<INPUT> = Vec::with_capacity(keys.len() * 2);

    // Modifier key downs
    for &m in modifiers {
        inputs.push(make_key_input(m.code(), 0));
    }

    // Main key down + up
    inputs.push(make_key_input(main_key[0].code(), 0));
    inputs.push(make_key_input(main_key[0].code(), KEYEVENTF_KEYUP));

    // Modifier key ups (reverse order)
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

/// Execute an action (send keystrokes or handle custom actions).
#[cfg(windows)]
pub fn execute_action(action: &Action) {
    match action {
        Action::KeyCombo(keys) => send_key_combo(keys),
        Action::Custom(name) => {
            log::info!("Custom action triggered: {name}");
            // Custom actions are handled by the main loop
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rising_edge() {
        let mut mapper = MapperState::default();
        let mut buttons = ButtonState::default();

        // No buttons pressed → no actions
        let actions = mapper.update(&buttons);
        assert!(actions.is_empty());

        // Press cross → should produce Enter
        buttons.cross = true;
        let actions = mapper.update(&buttons);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::KeyCombo(keys) => assert_eq!(keys, &[VKey::Return]),
            _ => panic!("Expected KeyCombo"),
        }

        // Hold cross (no change) → no new actions
        let actions = mapper.update(&buttons);
        assert!(actions.is_empty());

        // Release cross → no action on release
        buttons.cross = false;
        let actions = mapper.update(&buttons);
        assert!(actions.is_empty());
    }

    #[test]
    fn dpad_two_frame_confirm() {
        let mut mapper = MapperState::default();
        let mut buttons = ButtonState::default();

        // Frame 1: press Up — should NOT fire yet (pending)
        buttons.dpad = DPad::Up;
        let actions = mapper.update(&buttons);
        assert!(actions.is_empty(), "Should not fire on first frame");

        // Frame 2: still held — confirmed, fires
        let actions = mapper.update(&buttons);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::KeyCombo(keys) => assert_eq!(keys, &[VKey::Up]),
            _ => panic!("Expected KeyCombo"),
        }

        // Frame 3: still held — no repeat yet (within delay)
        let actions = mapper.update(&buttons);
        assert!(actions.is_empty(), "Should not repeat immediately");

        // Release
        buttons.dpad = DPad::Neutral;
        let actions = mapper.update(&buttons);
        assert!(actions.is_empty());
    }

    #[test]
    fn dpad_single_frame_glitch_filtered() {
        let mut mapper = MapperState::default();
        let mut buttons = ButtonState::default();

        // Frame 1: glitch press Up
        buttons.dpad = DPad::Up;
        let actions = mapper.update(&buttons);
        assert!(actions.is_empty(), "Pending, not fired");

        // Frame 2: immediately released — glitch filtered
        buttons.dpad = DPad::Neutral;
        let actions = mapper.update(&buttons);
        assert!(actions.is_empty(), "Single-frame glitch should not fire");
    }

    #[test]
    fn l1_produces_shift_alt_tab() {
        let mut mapper = MapperState::default();
        let mut buttons = ButtonState::default();

        buttons.l1 = true;
        let actions = mapper.update(&buttons);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::KeyCombo(keys) => assert_eq!(keys, &[VKey::Shift, VKey::Alt, VKey::Tab]),
            _ => panic!("Expected KeyCombo"),
        }
    }

    #[test]
    fn square_produces_custom() {
        let mut mapper = MapperState::default();
        let mut buttons = ButtonState::default();

        buttons.square = true;
        let actions = mapper.update(&buttons);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::Custom(name) => assert_eq!(name, "new_session"),
            _ => panic!("Expected Custom"),
        }
    }
}
