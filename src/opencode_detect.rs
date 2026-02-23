/// Auto-detect OpenCode keybind configuration from ~/.config/opencode/opencode.json via WSL.
///
/// Reads the "keybinds" section of the OpenCode config and resolves action names
/// to key combos or leader-key sequences. Falls back gracefully if the config file
/// is missing or WSL is unavailable.
///
/// The returned `OpenCodeDetected` is passed to `OpenCodeState::from_config()` in
/// `mapper.rs`, which resolves per-button action names with the priority:
///   1. Auto-detected binding  →  2. Hardcoded default  →  3. Direct combo parse

use crate::mapper::{parse_key_combo, VKey};
use std::collections::HashMap;

/// A resolved OpenCode key binding.
#[derive(Debug, Clone)]
pub enum ActionBinding {
    /// Direct key combo (e.g., ctrl+s → [Control, S]).
    Combo(Vec<VKey>),
    /// Leader key followed by a single key (e.g., <leader>n → leader then [N]).
    LeaderKey(Vec<VKey>),
}

/// Auto-detected OpenCode configuration.
#[derive(Debug, Clone)]
pub struct OpenCodeDetected {
    /// Detected leader key combo (e.g., [Control, X] for ctrl+x).
    pub leader: Option<Vec<VKey>>,
    /// Map of OpenCode action name → resolved binding.
    actions: HashMap<String, ActionBinding>,
}

impl OpenCodeDetected {
    /// Look up the binding for a given OpenCode action name.
    pub fn binding_for_action(&self, action: &str) -> Option<&ActionBinding> {
        self.actions.get(action)
    }
}

/// Detect OpenCode configuration by reading `~/.config/opencode/opencode.json` via WSL.
/// Returns `None` if detection fails (WSL unavailable, config not found, or parse error).
pub fn detect() -> Option<OpenCodeDetected> {
    log::info!("Auto-detecting OpenCode keybinds via WSL...");
    let start = std::time::Instant::now();

    let json_str = crate::wsl::run_wsl("cat ~/.config/opencode/opencode.json 2>/dev/null")?;
    let json_str = json_str.trim();

    if json_str.is_empty() {
        log::warn!("OpenCode config not found at ~/.config/opencode/opencode.json");
        return None;
    }

    let (leader, actions) = parse_config(json_str);
    let elapsed = start.elapsed();

    if let Some(ref l) = leader {
        log::info!("Detected OpenCode leader: {l:?}");
    }
    log::info!(
        "Detected {} OpenCode keybinds (took {elapsed:?})",
        actions.len()
    );

    Some(OpenCodeDetected { leader, actions })
}

// ── Config JSON parser ────────────────────────────────────────────────

/// Parse the OpenCode config JSON and extract leader + action → binding pairs.
fn parse_config(json: &str) -> (Option<Vec<VKey>>, HashMap<String, ActionBinding>) {
    let mut actions: HashMap<String, ActionBinding> = HashMap::new();
    let mut leader: Option<Vec<VKey>> = None;

    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("Failed to parse OpenCode config JSON: {e}");
            return (None, actions);
        }
    };

    let keybinds = match value.get("keybinds").and_then(|v| v.as_object()) {
        Some(kb) => kb,
        None => {
            log::debug!("No 'keybinds' section in OpenCode config");
            return (None, actions);
        }
    };

    for (action, key_value) in keybinds {
        let key_str = match key_value.as_str() {
            Some(s) => s,
            None => continue,
        };

        // The leader definition is stored as its own keybind entry.
        // OpenCode may use "leader" or "app:leader" as the key name.
        if action == "leader" || action == "app:leader" {
            leader = parse_key_combo(key_str);
            continue;
        }

        if let Some(binding) = parse_opencode_binding(key_str) {
            actions.insert(action.clone(), binding);
        }
    }

    (leader, actions)
}

// ── Key binding parsers ───────────────────────────────────────────────

/// Parse an OpenCode key binding string to an `ActionBinding`.
///
/// Handles comma-separated alternatives — takes the first valid one.
///
/// Examples:
/// - `"ctrl+s"`           → `Combo([Control, S])`
/// - `"ctrl+shift+["`     → `Combo([Control, Shift, LeftBracket])`
/// - `"<leader>n"`        → `LeaderKey([N])`
/// - `"f1"`               → `Combo([F1])`
/// - `"ctrl+[,<leader>p"` → `Combo([Control, LeftBracket])` (first valid)
pub fn parse_opencode_binding(s: &str) -> Option<ActionBinding> {
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(binding) = parse_single_binding(part) {
            return Some(binding);
        }
    }
    None
}

/// Parse a single (non-comma) OpenCode binding string.
fn parse_single_binding(s: &str) -> Option<ActionBinding> {
    if let Some(rest) = s.strip_prefix("<leader>") {
        let keys = parse_key_combo(rest.trim())?;
        return Some(ActionBinding::LeaderKey(keys));
    }
    let keys = parse_key_combo(s)?;
    Some(ActionBinding::Combo(keys))
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_direct_combo() {
        let b = parse_opencode_binding("ctrl+s").unwrap();
        match b {
            ActionBinding::Combo(keys) => assert_eq!(keys, vec![VKey::Control, VKey::S]),
            _ => panic!("Expected Combo"),
        }
    }

    #[test]
    fn parse_leader_key() {
        let b = parse_opencode_binding("<leader>n").unwrap();
        match b {
            ActionBinding::LeaderKey(keys) => assert_eq!(keys, vec![VKey::N]),
            _ => panic!("Expected LeaderKey"),
        }
    }

    #[test]
    fn parse_multi_modifier() {
        let b = parse_opencode_binding("ctrl+shift+[").unwrap();
        match b {
            ActionBinding::Combo(keys) => {
                assert_eq!(keys, vec![VKey::Control, VKey::Shift, VKey::LeftBracket])
            }
            _ => panic!("Expected Combo"),
        }
    }

    #[test]
    fn parse_f_key() {
        let b = parse_opencode_binding("f1").unwrap();
        match b {
            ActionBinding::Combo(keys) => assert_eq!(keys, vec![VKey::F1]),
            _ => panic!("Expected Combo"),
        }
    }

    #[test]
    fn parse_comma_separated_takes_first() {
        // "ctrl+[,<leader>p" → Combo from ctrl+[
        let b = parse_opencode_binding("ctrl+[,<leader>p").unwrap();
        match b {
            ActionBinding::Combo(keys) => {
                assert_eq!(keys, vec![VKey::Control, VKey::LeftBracket])
            }
            _ => panic!("Expected Combo (first valid alternative)"),
        }
    }

    #[test]
    fn parse_config_extracts_bindings() {
        let json = r#"{
            "keybinds": {
                "leader": "ctrl+x",
                "session:next": "ctrl+]",
                "session:prev": "ctrl+[",
                "app:new-session": "<leader>n"
            }
        }"#;
        let (leader, actions) = parse_config(json);
        assert_eq!(leader, Some(vec![VKey::Control, VKey::X]));
        assert!(actions.contains_key("session:next"));
        assert!(actions.contains_key("session:prev"));
        assert!(actions.contains_key("app:new-session"));
    }

    #[test]
    fn parse_config_no_keybinds_section() {
        let json = r#"{"theme": "dark"}"#;
        let (leader, actions) = parse_config(json);
        assert!(leader.is_none());
        assert!(actions.is_empty());
    }
}
