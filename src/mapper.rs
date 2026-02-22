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

/// Track which buttons were pressed last frame to detect edges (rising edge = newly pressed).
#[derive(Default)]
pub struct MapperState {
    prev: ButtonState,
}

impl MapperState {
    /// Given current button state, return actions for newly pressed buttons.
    pub fn update(&mut self, current: &ButtonState) -> Vec<Action> {
        let mut actions = Vec::new();

        // Helper: detect rising edge
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

        // D-pad
        if current.dpad != self.prev.dpad {
            match current.dpad {
                DPad::Up => actions.push(Action::KeyCombo(vec![VKey::Up])),
                DPad::Down => actions.push(Action::KeyCombo(vec![VKey::Down])),
                DPad::Left => actions.push(Action::KeyCombo(vec![VKey::Left])),
                DPad::Right => actions.push(Action::KeyCombo(vec![VKey::Right])),
                DPad::UpLeft => {
                    actions.push(Action::KeyCombo(vec![VKey::Up]));
                    actions.push(Action::KeyCombo(vec![VKey::Left]));
                }
                DPad::UpRight => {
                    actions.push(Action::KeyCombo(vec![VKey::Up]));
                    actions.push(Action::KeyCombo(vec![VKey::Right]));
                }
                DPad::DownLeft => {
                    actions.push(Action::KeyCombo(vec![VKey::Down]));
                    actions.push(Action::KeyCombo(vec![VKey::Left]));
                }
                DPad::DownRight => {
                    actions.push(Action::KeyCombo(vec![VKey::Down]));
                    actions.push(Action::KeyCombo(vec![VKey::Right]));
                }
                DPad::Neutral => {} // released, no action
            }
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
    fn dpad_produces_arrows() {
        let mut mapper = MapperState::default();
        let mut buttons = ButtonState::default();

        buttons.dpad = DPad::Up;
        let actions = mapper.update(&buttons);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::KeyCombo(keys) => assert_eq!(keys, &[VKey::Up]),
            _ => panic!("Expected KeyCombo"),
        }
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
