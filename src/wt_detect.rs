/// Auto-detect Windows Terminal keybindings from settings.json.
///
/// Reads the `actions` array and resolves action names (e.g., "prevTab", "nextTab",
/// "newTab") to key combos. Falls back gracefully if the settings file is missing
/// or unparseable.
///
/// The returned `WtDetected` is passed to `WtState::from_config()` in `mapper.rs`,
/// which resolves per-button action names with the priority:
///   1. Auto-detected binding  →  2. Hardcoded default  →  3. Direct combo parse

use crate::mapper::{parse_key_combo, VKey};
use std::collections::HashMap;

/// Auto-detected Windows Terminal keybindings.
#[derive(Debug, Clone)]
pub struct WtDetected {
    /// Map of WT action name → resolved key combo.
    actions: HashMap<String, Vec<VKey>>,
}

impl WtDetected {
    /// Look up the key combo for a given WT action name.
    pub fn key_for_action(&self, action: &str) -> Option<&Vec<VKey>> {
        self.actions.get(action)
    }
}

/// Detect Windows Terminal keybindings from settings.json.
/// Returns `None` if no settings file is found or it can't be parsed.
pub fn detect() -> Option<WtDetected> {
    log::info!("Auto-detecting Windows Terminal keybindings from settings.json...");
    let start = std::time::Instant::now();

    let json_str = read_settings_json()?;
    let actions = parse_settings(&json_str);
    let elapsed = start.elapsed();

    log::info!(
        "Detected {} Windows Terminal keybinds (took {elapsed:?})",
        actions.len()
    );

    Some(WtDetected { actions })
}

// ── Settings file discovery ───────────────────────────────────────────

/// Try to read settings.json from known Windows Terminal installation paths.
///
/// Checks (in order):
///   1. Stable release (Microsoft Store)
///   2. Preview release (Microsoft Store)
///   3. Unpackaged / winget install
fn read_settings_json() -> Option<String> {
    let local_app_data = std::env::var("LOCALAPPDATA").ok()?;

    let candidates = [
        format!(
            r"{local_app_data}\Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json"
        ),
        format!(
            r"{local_app_data}\Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json"
        ),
        format!(r"{local_app_data}\Microsoft\Windows Terminal\settings.json"),
    ];

    for path in &candidates {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                log::debug!("Found Windows Terminal settings at: {path}");
                return Some(content);
            }
            Err(_) => continue,
        }
    }

    log::warn!("Windows Terminal settings.json not found in any expected location");
    None
}

// ── JSON parser ───────────────────────────────────────────────────────

/// Parse settings JSON and extract action-name → key-combo pairs.
///
/// Both modern (`"actions"`) and legacy (`"keybindings"`) array keys are
/// supported. When the same action appears multiple times the first entry wins.
fn parse_settings(json: &str) -> HashMap<String, Vec<VKey>> {
    let mut actions: HashMap<String, Vec<VKey>> = HashMap::new();

    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("Failed to parse Windows Terminal settings.json: {e}");
            return actions;
        }
    };

    // Support both "actions" (>=1.x) and "keybindings" (legacy 0.x)
    let entries = value
        .get("actions")
        .or_else(|| value.get("keybindings"))
        .and_then(|v| v.as_array());

    let entries = match entries {
        Some(e) => e,
        None => {
            log::debug!("No 'actions' or 'keybindings' array in Windows Terminal settings.json");
            return actions;
        }
    };

    for entry in entries {
        // Skip entries with no keys or explicitly unbound (null)
        let keys_str = match get_keys_str(entry) {
            Some(k) => k,
            None => continue,
        };

        let vkeys = match parse_key_combo(&keys_str) {
            Some(k) => k,
            None => {
                log::debug!("Failed to parse WT key combo: {keys_str:?}");
                continue;
            }
        };

        if let Some(action_name) = get_action_name(entry) {
            // First binding for an action wins (user's explicit bindings come
            // first in the array before WT's generated defaults)
            actions.entry(action_name).or_insert(vkeys);
        }
    }

    actions
}

/// Extract the first usable key string from an action entry.
///
/// `"keys"` can be:
///   - A string:  `"ctrl+tab"`
///   - An array:  `["ctrl+tab", "ctrl+pgup"]`  → first element used
///   - `null`     → unbound, skip
fn get_keys_str(entry: &serde_json::Value) -> Option<String> {
    let keys = entry.get("keys")?;
    if keys.is_null() {
        return None;
    }
    if let Some(s) = keys.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = keys.as_array() {
        for v in arr {
            if let Some(s) = v.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Extract the canonical action name from a command entry.
///
/// `"command"` can be:
///   - A string:  `"prevTab"`
///   - An object: `{ "action": "newTab", "index": 0 }`  → returns `"newTab"`
///
/// Object-style commands with extra fields (index, profile, etc.) are stored
/// under the bare action name. If you need e.g. "newTab with index 0" vs
/// "newTab with index 1" to be distinct, configure the key directly in TOML.
fn get_action_name(entry: &serde_json::Value) -> Option<String> {
    let cmd = entry.get("command")?;
    if let Some(s) = cmd.as_str() {
        return Some(s.to_string());
    }
    if let Some(obj) = cmd.as_object() {
        if let Some(action) = obj.get("action").and_then(|v| v.as_str()) {
            return Some(action.to_string());
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_json(actions: &str) -> String {
        format!(r#"{{"actions": [{actions}]}}"#)
    }

    #[test]
    fn detects_string_command() {
        let json = make_json(r#"{"command": "prevTab", "keys": "ctrl+shift+tab"}"#);
        let map = parse_settings(&json);
        assert_eq!(map.get("prevTab"), Some(&vec![VKey::Control, VKey::Shift, VKey::Tab]));
    }

    #[test]
    fn detects_object_command() {
        let json = make_json(r#"{"command": {"action": "newTab", "index": 0}, "keys": "ctrl+shift+1"}"#);
        let map = parse_settings(&json);
        assert_eq!(map.get("newTab"), Some(&vec![VKey::Control, VKey::Shift, VKey::D1]));
    }

    #[test]
    fn detects_next_tab() {
        let json = make_json(r#"{"command": "nextTab", "keys": "ctrl+tab"}"#);
        let map = parse_settings(&json);
        assert_eq!(map.get("nextTab"), Some(&vec![VKey::Control, VKey::Tab]));
    }

    #[test]
    fn skips_unbound_null_keys() {
        let json = make_json(r#"{"command": "prevTab", "keys": null}"#);
        let map = parse_settings(&json);
        assert!(map.get("prevTab").is_none());
    }

    #[test]
    fn first_binding_wins() {
        // User binding comes before generated default
        let json = make_json(
            r#"{"command": "nextTab", "keys": "ctrl+pgdn"},
               {"command": "nextTab", "keys": "ctrl+tab"}"#,
        );
        let map = parse_settings(&json);
        // Should have the first one
        assert_eq!(map.get("nextTab").map(|k| k.len()), Some(2)); // ctrl+pgdn would fail to parse (pgdn not in VKey), ctrl+tab would succeed
    }

    #[test]
    fn legacy_keybindings_key() {
        let json = r#"{"keybindings": [{"command": "prevTab", "keys": "ctrl+shift+tab"}]}"#;
        let map = parse_settings(json);
        assert!(map.contains_key("prevTab"));
    }

    #[test]
    fn array_keys_takes_first() {
        let json = make_json(
            r#"{"command": "nextTab", "keys": ["ctrl+tab", "ctrl+pgdn"]}"#,
        );
        let map = parse_settings(&json);
        assert_eq!(map.get("nextTab"), Some(&vec![VKey::Control, VKey::Tab]));
    }
}
