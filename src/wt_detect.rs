/// Auto-detect Windows Terminal keybindings from settings.json.
///
/// Reads both the `"actions"` and `"keybindings"` arrays and resolves action names
/// (e.g., "prevTab", "nextTab", "newTab") to key combos.  Supports both the legacy
/// `"command"` field format and the modern `"id"` field format (WT ≥1.18).
///
/// Falls back gracefully if the settings file is missing or unparseable.
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
/// Merges entries from both `"actions"` and `"keybindings"` arrays.  Modern WT
/// (≥1.18) puts user customizations in `"keybindings"` using an `"id"` field
/// while leaving `"actions"` empty — the old either/or logic missed those.
///
/// When the same action appears multiple times the first entry wins.
fn parse_settings(json: &str) -> HashMap<String, Vec<VKey>> {
    let mut actions: HashMap<String, Vec<VKey>> = HashMap::new();

    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("Failed to parse Windows Terminal settings.json: {e}");
            return actions;
        }
    };

    // Merge both "actions" and "keybindings" — not either/or.
    // Modern WT may have "actions": [] (empty) with all customizations in "keybindings".
    let mut all_entries: Vec<&serde_json::Value> = Vec::new();
    if let Some(arr) = value.get("actions").and_then(|v| v.as_array()) {
        all_entries.extend(arr.iter());
    }
    if let Some(arr) = value.get("keybindings").and_then(|v| v.as_array()) {
        all_entries.extend(arr.iter());
    }

    if all_entries.is_empty() {
        log::debug!("No 'actions' or 'keybindings' entries in Windows Terminal settings.json");
        return actions;
    }

    for entry in &all_entries {
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

/// Extract the canonical action name from an entry.
///
/// Supports three formats used across WT versions:
///
///   1. `"command": "prevTab"`             (string — legacy & modern)
///   2. `"command": { "action": "newTab" }` (object — parameterized commands)
///   3. `"id": "Terminal.PrevTab"`          (modern ≥1.18, normalized via [`normalize_wt_id`])
///
/// Object-style commands with extra fields (index, profile, etc.) are stored
/// under the bare action name.
fn get_action_name(entry: &serde_json::Value) -> Option<String> {
    // Try "command" field first (works in both old and new WT formats)
    if let Some(cmd) = entry.get("command") {
        if let Some(s) = cmd.as_str() {
            return Some(s.to_string());
        }
        if let Some(obj) = cmd.as_object() {
            if let Some(action) = obj.get("action").and_then(|v| v.as_str()) {
                return Some(action.to_string());
            }
        }
    }

    // Try "id" field (modern WT ≥1.18 uses this for GUI-customized bindings)
    if let Some(id) = entry.get("id").and_then(|v| v.as_str()) {
        if let Some(short) = normalize_wt_id(id) {
            return Some(short.to_string());
        }
        // Store unrecognized IDs as-is (may match a direct config value)
        return Some(id.to_string());
    }

    None
}

/// Map well-known Windows Terminal internal action IDs to the short camelCase
/// names used in DS4CC config and `default_key_for_wt_action()`.
fn normalize_wt_id(id: &str) -> Option<&'static str> {
    match id {
        // Tab management
        "Terminal.OpenNewTab"                        => Some("newTab"),
        "Terminal.PrevTab" | "Terminal.PreviousTab"  => Some("prevTab"),
        "Terminal.NextTab"                           => Some("nextTab"),
        "Terminal.CloseTab"                          => Some("closeTab"),
        "Terminal.DuplicateTab"                      => Some("duplicateTab"),
        // Window / pane
        "Terminal.OpenNewWindow"                     => Some("newWindow"),
        "Terminal.DuplicatePaneAuto"                 => Some("duplicatePane"),
        "Terminal.SplitPane"                         => Some("splitDown"),
        // Search & misc
        "Terminal.FindText"                          => Some("find"),
        "Terminal.ToggleFullscreen"                  => Some("toggleFullscreen"),
        "Terminal.OpenSettings"                      => Some("openSettings"),
        "Terminal.ToggleCommandPalette"
        | "Terminal.OpenCommandPalette"              => Some("commandPalette"),
        // Clipboard
        "Terminal.CopyToClipboard"                   => Some("copy"),
        "Terminal.PasteFromClipboard"                => Some("paste"),
        _ => None,
    }
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
        // ctrl+pgdn fails to parse (pgdn not in VKey), ctrl+tab succeeds
        assert_eq!(map.get("nextTab").map(|k| k.len()), Some(2));
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

    // ── Modern "id" format tests (WT ≥1.18) ──────────────────────────

    #[test]
    fn detects_id_field_with_normalization() {
        let json = r#"{"keybindings": [{"id": "Terminal.NextTab", "keys": "ctrl+tab"}]}"#;
        let map = parse_settings(json);
        assert_eq!(map.get("nextTab"), Some(&vec![VKey::Control, VKey::Tab]));
    }

    #[test]
    fn detects_id_field_copy_paste() {
        let json = r#"{"keybindings": [
            {"id": "Terminal.CopyToClipboard", "keys": "ctrl+c"},
            {"id": "Terminal.PasteFromClipboard", "keys": "ctrl+v"}
        ]}"#;
        let map = parse_settings(json);
        assert_eq!(map.get("copy"), Some(&vec![VKey::Control, VKey::C]));
        assert_eq!(map.get("paste"), Some(&vec![VKey::Control, VKey::V]));
    }

    #[test]
    fn merges_actions_and_keybindings() {
        // Modern WT: actions has command-style, keybindings has id-style
        let json = r#"{
            "actions": [{"command": "prevTab", "keys": "ctrl+shift+tab"}],
            "keybindings": [{"id": "Terminal.NextTab", "keys": "ctrl+tab"}]
        }"#;
        let map = parse_settings(json);
        assert!(map.contains_key("prevTab"), "should find prevTab from actions");
        assert!(map.contains_key("nextTab"), "should find nextTab from keybindings");
    }

    #[test]
    fn empty_actions_falls_through_to_keybindings() {
        // Typical modern WT: empty actions array, customizations in keybindings
        let json = r#"{
            "actions": [],
            "keybindings": [{"id": "Terminal.DuplicatePaneAuto", "keys": "alt+shift+d"}]
        }"#;
        let map = parse_settings(json);
        assert!(map.contains_key("duplicatePane"));
    }

    #[test]
    fn unknown_id_stored_as_is() {
        let json = r#"{"keybindings": [{"id": "Terminal.SomeFutureAction", "keys": "ctrl+f12"}]}"#;
        let map = parse_settings(json);
        // Unrecognized IDs are stored verbatim
        assert!(map.contains_key("Terminal.SomeFutureAction"));
    }

    #[test]
    fn actions_entry_wins_over_keybindings_duplicate() {
        // Same action in both arrays — "actions" entry comes first, wins
        let json = r#"{
            "actions": [{"command": "nextTab", "keys": "ctrl+tab"}],
            "keybindings": [{"id": "Terminal.NextTab", "keys": "ctrl+shift+tab"}]
        }"#;
        let map = parse_settings(json);
        // ctrl+tab from actions should win (2 keys), not ctrl+shift+tab (3 keys)
        assert_eq!(map.get("nextTab").map(|k| k.len()), Some(2));
    }
}
