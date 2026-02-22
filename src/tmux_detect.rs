/// Auto-detect tmux configuration from a running tmux server via WSL.
///
/// Queries `tmux show-options -g prefix` for the prefix key and
/// `tmux list-keys -T prefix` for the full binding table.
/// Parses tmux key notation (C-a, M-n, etc.) into VKey combos.
///
/// Falls back gracefully if tmux isn't running or WSL isn't available.

use crate::mapper::VKey;
use std::collections::HashMap;

/// Auto-detected tmux configuration.
#[derive(Debug, Clone)]
pub struct TmuxDetected {
    /// Detected prefix as VKey combo (e.g., [Control, A]).
    pub prefix: Option<Vec<VKey>>,
    /// Map of tmux command → VKey combo for the key bound to that command.
    /// e.g., "previous-window" → [P], "kill-window" → [Shift, D7]
    actions: HashMap<String, Vec<VKey>>,
}

impl TmuxDetected {
    /// Look up the key combo bound to a given tmux action/command.
    pub fn key_for_action(&self, action: &str) -> Option<&Vec<VKey>> {
        self.actions.get(action)
    }
}

/// Detect tmux configuration by querying a running tmux server via WSL.
/// Returns `None` if detection fails entirely (WSL not available, tmux not running).
pub fn detect() -> Option<TmuxDetected> {
    log::info!("Auto-detecting tmux configuration via WSL...");
    let start = std::time::Instant::now();

    let prefix = detect_prefix();
    let actions = detect_bindings();

    let elapsed = start.elapsed();

    if prefix.is_none() && actions.is_empty() {
        log::warn!("Tmux auto-detection failed (took {elapsed:?}). Using config defaults.");
        return None;
    }

    if let Some(ref p) = prefix {
        log::info!("Detected tmux prefix: {p:?}");
    } else {
        log::warn!("Could not detect tmux prefix, using config value");
    }
    log::info!("Detected {} tmux key bindings (took {elapsed:?})", actions.len());

    Some(TmuxDetected { prefix, actions })
}

// ── WSL command execution ────────────────────────────────────────────

fn run_wsl(cmd: &str) -> Option<String> {
    let output = std::process::Command::new("wsl")
        .args(["-e", "bash", "-lc", cmd])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

// ── Prefix detection ─────────────────────────────────────────────────

fn detect_prefix() -> Option<Vec<VKey>> {
    // Try running tmux server first
    if let Some(output) = run_wsl("tmux show-options -g prefix 2>/dev/null") {
        // Format: "prefix C-a\n"
        for line in output.lines() {
            if let Some(key_str) = line.strip_prefix("prefix ") {
                let key_str = key_str.trim();
                if !key_str.is_empty() {
                    log::debug!("Prefix from tmux server: {key_str}");
                    return parse_tmux_key(key_str);
                }
            }
        }
    }

    // Fallback: parse ~/.tmux.conf directly
    log::debug!("Tmux server not running, parsing ~/.tmux.conf...");
    if let Some(conf) = run_wsl("cat ~/.tmux.conf 2>/dev/null") {
        for line in conf.lines() {
            let line = line.trim();
            // Match: set -g prefix C-a  or  set-option -g prefix C-a
            if (line.starts_with("set ") || line.starts_with("set-option "))
                && line.contains("prefix")
                && !line.starts_with('#')
            {
                // Extract the key after "prefix"
                if let Some(idx) = line.find("prefix") {
                    let after = line[idx + 6..].trim();
                    // Skip "prefix2" lines
                    if after.starts_with('2') {
                        continue;
                    }
                    let key_str = after.split_whitespace().next().unwrap_or("");
                    if !key_str.is_empty() {
                        log::debug!("Prefix from tmux.conf: {key_str}");
                        return parse_tmux_key(key_str);
                    }
                }
            }
        }
    }

    None
}

// ── Binding table detection ──────────────────────────────────────────

fn detect_bindings() -> HashMap<String, Vec<VKey>> {
    let mut actions = HashMap::new();

    // Try running tmux server first
    if let Some(output) = run_wsl("tmux list-keys -T prefix 2>/dev/null") {
        for line in output.lines() {
            if let Some((vkeys, command)) = parse_binding_line(line) {
                insert_binding(&mut actions, command, vkeys);
            }
        }
        if !actions.is_empty() {
            return actions;
        }
    }

    // Fallback: parse ~/.tmux.conf bind commands
    log::debug!("Tmux server not running, parsing bind commands from ~/.tmux.conf...");
    if let Some(conf) = run_wsl("cat ~/.tmux.conf 2>/dev/null") {
        for line in conf.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            if let Some((vkeys, command)) = parse_conf_bind(line) {
                insert_binding(&mut actions, command, vkeys);
            }
        }
    }

    actions
}

fn insert_binding(actions: &mut HashMap<String, Vec<VKey>>, command: String, vkeys: Vec<VKey>) {
    if command.is_empty() {
        return;
    }
    // Store full command (e.g., "resize-pane -Z")
    actions.entry(command.clone()).or_insert_with(|| vkeys.clone());
    // Also store base command (e.g., "resize-pane") if different
    if let Some(base) = command.split_whitespace().next() {
        if base != command {
            actions.entry(base.to_string()).or_insert(vkeys);
        }
    }
}

/// Parse a tmux.conf bind/bind-key line.
/// Format: `bind [-r] <key> <command> [args...]` or `bind-key [-r] <key> <command> [args...]`
fn parse_conf_bind(line: &str) -> Option<(Vec<VKey>, String)> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let is_bind = matches!(tokens[0], "bind" | "bind-key");
    if !is_bind {
        return None;
    }

    let mut i = 1;
    // Skip flags
    while i < tokens.len() && tokens[i].starts_with('-') {
        // Skip -T <table> pair
        if tokens[i] == "-T" {
            i += 1; // skip the table name
            // If it's not prefix table, skip this binding
            if i < tokens.len() && tokens[i] != "prefix" {
                return None;
            }
        }
        i += 1;
    }

    if i >= tokens.len() {
        return None;
    }

    let key_str = tokens[i];
    let vkeys = parse_tmux_key(key_str)?;

    let cmd_start = i + 1;
    if cmd_start >= tokens.len() {
        return None;
    }

    let command = extract_command(&tokens[cmd_start..]);
    Some((vkeys, command))
}

/// Parse a single `tmux list-keys -T prefix` line.
/// Returns (key_vkeys, extracted_command).
fn parse_binding_line(line: &str) -> Option<(Vec<VKey>, String)> {
    // Format: "bind-key [-r] -T prefix <key> <command> [args...]"
    let parts: Vec<&str> = line.split_whitespace().collect();

    // Find "prefix" keyword
    let prefix_idx = parts.iter().position(|&p| p == "prefix")?;
    let key_idx = prefix_idx + 1;
    if key_idx >= parts.len() {
        return None;
    }

    let key_str = parts[key_idx];
    let vkeys = parse_tmux_key(key_str)?;

    let cmd_start = key_idx + 1;
    if cmd_start >= parts.len() {
        return None;
    }

    let command = extract_command(&parts[cmd_start..]);
    Some((vkeys, command))
}

/// Extract the effective tmux command from a binding's command + args.
///
/// Handles wrappers like `confirm-before` and `command-prompt`,
/// and includes semantically important flags (e.g., `-h`, `-v`, `-Z`).
fn extract_command(tokens: &[&str]) -> String {
    if tokens.is_empty() {
        return String::new();
    }

    match tokens[0] {
        "confirm-before" => {
            // Pattern: confirm-before -p "kill-window #W? (y/n)" kill-window
            // Skip flags and quoted strings to find the actual command
            let mut in_quote = false;
            for &token in &tokens[1..] {
                if in_quote {
                    if token.ends_with('"') {
                        in_quote = false;
                    }
                    continue;
                }
                if token.starts_with('-') {
                    continue;
                }
                if token.starts_with('"') {
                    if !token.ends_with('"') || token.len() == 1 {
                        in_quote = true;
                    }
                    continue;
                }
                // Found the actual command
                return token.to_string();
            }
            String::new()
        }
        "command-prompt" => {
            // Pattern: command-prompt [...] { <cmd> ... }
            if let Some(brace_idx) = tokens.iter().position(|&t| t == "{") {
                if brace_idx + 1 < tokens.len() && tokens[brace_idx + 1] != "}" {
                    return tokens[brace_idx + 1].to_string();
                }
            }
            String::new()
        }
        // Too complex to extract meaningfully
        "if-shell" | "run-shell" | "display-menu" => String::new(),
        // Internal tmux actions we don't map to gamepad buttons
        "send-keys" | "send-prefix" => String::new(),
        cmd => {
            // Direct command, include single-char flags that change semantics
            // e.g., split-window -h, resize-pane -Z
            let mut result = cmd.to_string();
            for &token in &tokens[1..] {
                if token.starts_with('-') && token.len() == 2 {
                    result.push(' ');
                    result.push_str(token);
                } else {
                    break;
                }
            }
            result
        }
    }
}

// ── Tmux key notation parser ─────────────────────────────────────────

/// Parse tmux key notation into VKey combo.
///
/// Handles:
/// - Modifier prefixes: `C-a` (Ctrl+A), `M-n` (Alt+N), `S-Left` (Shift+Left)
/// - Escaped symbols: `\;`, `\#`, `\{`, etc.
/// - Named keys: `Space`, `Enter`, `Up`, `Down`, etc.
/// - Single characters: `p`, `n`, `c`, `&`, `[`, etc.
pub fn parse_tmux_key(s: &str) -> Option<Vec<VKey>> {
    // Handle tmux escape prefix
    let s = s.strip_prefix('\\').unwrap_or(s);

    // Handle modifier prefixes
    if let Some(rest) = s.strip_prefix("C-") {
        let key = single_char_to_vkey(rest)?;
        return Some(vec![VKey::Control, key]);
    }
    if let Some(rest) = s.strip_prefix("M-C-") {
        let key = single_char_to_vkey(rest)?;
        return Some(vec![VKey::Alt, VKey::Control, key]);
    }
    if let Some(rest) = s.strip_prefix("M-") {
        // Could be M-Up, M-Down, etc.
        if let Some(named) = named_key_to_vkey(rest) {
            return Some(vec![VKey::Alt, named]);
        }
        let key = single_char_to_vkey(rest)?;
        return Some(vec![VKey::Alt, key]);
    }
    if let Some(rest) = s.strip_prefix("S-") {
        if let Some(named) = named_key_to_vkey(rest) {
            return Some(vec![VKey::Shift, named]);
        }
        let key = single_char_to_vkey(rest)?;
        return Some(vec![VKey::Shift, key]);
    }

    // Handle named keys
    if let Some(named) = named_key_to_vkey(s) {
        return Some(vec![named]);
    }

    // Single character → VKey combo (may include Shift for symbols)
    if s.len() == 1 {
        return symbol_to_vkeys(s.chars().next().unwrap());
    }

    None
}

/// Convert a single lowercase letter to VKey.
fn single_char_to_vkey(s: &str) -> Option<VKey> {
    if s.len() == 1 {
        let c = s.chars().next().unwrap();
        match c.to_ascii_lowercase() {
            'a'..='z' => VKey::from_name(&c.to_ascii_lowercase().to_string()),
            '0'..='9' => VKey::from_name(&c.to_string()),
            _ => None,
        }
    } else {
        None
    }
}

/// Convert a tmux named key to VKey.
fn named_key_to_vkey(s: &str) -> Option<VKey> {
    match s {
        "Space" => Some(VKey::Space),
        "Enter" => Some(VKey::Return),
        "Escape" => Some(VKey::Escape),
        "Tab" => Some(VKey::Tab),
        "Up" => Some(VKey::Up),
        "Down" => Some(VKey::Down),
        "Left" => Some(VKey::Left),
        "Right" => Some(VKey::Right),
        _ => None,
    }
}

/// Convert a single character (including symbols) to VKey combo.
/// Symbols that require Shift return [Shift, BaseKey].
fn symbol_to_vkeys(c: char) -> Option<Vec<VKey>> {
    match c {
        'a'..='z' => Some(vec![VKey::from_name(&c.to_string())?]),
        'A'..='Z' => Some(vec![VKey::Shift, VKey::from_name(&c.to_ascii_lowercase().to_string())?]),
        '0'..='9' => Some(vec![VKey::from_name(&c.to_string())?]),
        // Shifted digit symbols (US layout)
        '!' => Some(vec![VKey::Shift, VKey::D1]),
        '@' => Some(vec![VKey::Shift, VKey::D2]),
        '#' => Some(vec![VKey::Shift, VKey::D3]),
        '$' => Some(vec![VKey::Shift, VKey::D4]),
        '%' => Some(vec![VKey::Shift, VKey::D5]),
        '^' => Some(vec![VKey::Shift, VKey::D6]),
        '&' => Some(vec![VKey::Shift, VKey::D7]),
        '*' => Some(vec![VKey::Shift, VKey::D8]),
        '(' => Some(vec![VKey::Shift, VKey::D9]),
        ')' => Some(vec![VKey::Shift, VKey::D0]),
        // Punctuation (unshifted)
        '[' => Some(vec![VKey::LeftBracket]),
        ']' => Some(vec![VKey::RightBracket]),
        '\\' => Some(vec![VKey::Backslash]),
        ';' => Some(vec![VKey::Semicolon]),
        '\'' => Some(vec![VKey::Quote]),
        ',' => Some(vec![VKey::Comma]),
        '.' => Some(vec![VKey::Period]),
        '/' => Some(vec![VKey::Slash]),
        '-' => Some(vec![VKey::Minus]),
        '=' => Some(vec![VKey::Equals]),
        '`' => Some(vec![VKey::Backtick]),
        ' ' => Some(vec![VKey::Space]),
        // Punctuation (shifted)
        '{' => Some(vec![VKey::Shift, VKey::LeftBracket]),
        '}' => Some(vec![VKey::Shift, VKey::RightBracket]),
        '|' => Some(vec![VKey::Shift, VKey::Backslash]),
        ':' => Some(vec![VKey::Shift, VKey::Semicolon]),
        '"' => Some(vec![VKey::Shift, VKey::Quote]),
        '<' => Some(vec![VKey::Shift, VKey::Comma]),
        '>' => Some(vec![VKey::Shift, VKey::Period]),
        '?' => Some(vec![VKey::Shift, VKey::Slash]),
        '_' => Some(vec![VKey::Shift, VKey::Minus]),
        '+' => Some(vec![VKey::Shift, VKey::Equals]),
        '~' => Some(vec![VKey::Shift, VKey::Backtick]),
        _ => None,
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ctrl_a() {
        let keys = parse_tmux_key("C-a").unwrap();
        assert_eq!(keys, vec![VKey::Control, VKey::A]);
    }

    #[test]
    fn parse_ctrl_b() {
        let keys = parse_tmux_key("C-b").unwrap();
        assert_eq!(keys, vec![VKey::Control, VKey::B]);
    }

    #[test]
    fn parse_alt_n() {
        let keys = parse_tmux_key("M-n").unwrap();
        assert_eq!(keys, vec![VKey::Alt, VKey::N]);
    }

    #[test]
    fn parse_plain_letter() {
        assert_eq!(parse_tmux_key("p").unwrap(), vec![VKey::P]);
        assert_eq!(parse_tmux_key("n").unwrap(), vec![VKey::N]);
        assert_eq!(parse_tmux_key("c").unwrap(), vec![VKey::C]);
    }

    #[test]
    fn parse_uppercase_letter() {
        let keys = parse_tmux_key("D").unwrap();
        assert_eq!(keys, vec![VKey::Shift, VKey::D]);
    }

    #[test]
    fn parse_ampersand() {
        let keys = parse_tmux_key("&").unwrap();
        assert_eq!(keys, vec![VKey::Shift, VKey::D7]);
    }

    #[test]
    fn parse_left_bracket() {
        let keys = parse_tmux_key("[").unwrap();
        assert_eq!(keys, vec![VKey::LeftBracket]);
    }

    #[test]
    fn parse_escaped_semicolon() {
        let keys = parse_tmux_key("\\;").unwrap();
        assert_eq!(keys, vec![VKey::Semicolon]);
    }

    #[test]
    fn parse_named_key_space() {
        let keys = parse_tmux_key("Space").unwrap();
        assert_eq!(keys, vec![VKey::Space]);
    }

    #[test]
    fn parse_pipe() {
        let keys = parse_tmux_key("|").unwrap();
        assert_eq!(keys, vec![VKey::Shift, VKey::Backslash]);
    }

    #[test]
    fn parse_minus() {
        let keys = parse_tmux_key("-").unwrap();
        assert_eq!(keys, vec![VKey::Minus]);
    }

    #[test]
    fn extract_direct_command() {
        let tokens = vec!["previous-window"];
        assert_eq!(extract_command(&tokens), "previous-window");
    }

    #[test]
    fn extract_command_with_flags() {
        let tokens = vec!["split-window", "-h"];
        assert_eq!(extract_command(&tokens), "split-window -h");

        let tokens = vec!["resize-pane", "-Z"];
        assert_eq!(extract_command(&tokens), "resize-pane -Z");
    }

    #[test]
    fn extract_confirm_before() {
        // confirm-before -p "kill-window #W? (y/n)" kill-window
        let tokens = vec![
            "confirm-before", "-p", "\"kill-window", "#W?", "(y/n)\"", "kill-window",
        ];
        assert_eq!(extract_command(&tokens), "kill-window");
    }

    #[test]
    fn extract_command_prompt() {
        // command-prompt -I "#W" { rename-window "%%" }
        let tokens = vec![
            "command-prompt", "-I", "\"#W\"", "{", "rename-window", "\"%%\"", "}",
        ];
        assert_eq!(extract_command(&tokens), "rename-window");
    }

    #[test]
    fn binding_line_simple() {
        let line = "bind-key    -T prefix p       previous-window";
        let (keys, cmd) = parse_binding_line(line).unwrap();
        assert_eq!(keys, vec![VKey::P]);
        assert_eq!(cmd, "previous-window");
    }

    #[test]
    fn binding_line_with_repeat_flag() {
        let line = "bind-key -r -T prefix Up      select-pane -U";
        let (keys, cmd) = parse_binding_line(line).unwrap();
        assert_eq!(keys, vec![VKey::Up]);
        assert_eq!(cmd, "select-pane -U");
    }

    #[test]
    fn binding_line_confirm_before() {
        let line =
            "bind-key    -T prefix &       confirm-before -p \"kill-window #W? (y/n)\" kill-window";
        let (keys, cmd) = parse_binding_line(line).unwrap();
        assert_eq!(keys, vec![VKey::Shift, VKey::D7]);
        assert_eq!(cmd, "kill-window");
    }

    #[test]
    fn binding_line_custom_split() {
        let line = "bind-key    -T prefix |       split-window -h";
        let (keys, cmd) = parse_binding_line(line).unwrap();
        assert_eq!(keys, vec![VKey::Shift, VKey::Backslash]);
        assert_eq!(cmd, "split-window -h");
    }

    #[test]
    fn conf_bind_simple() {
        let line = "bind | split-window -h";
        let (keys, cmd) = parse_conf_bind(line).unwrap();
        assert_eq!(keys, vec![VKey::Shift, VKey::Backslash]);
        assert_eq!(cmd, "split-window -h");
    }

    #[test]
    fn conf_bind_with_flag() {
        let line = "bind -r n next-window";
        let (keys, cmd) = parse_conf_bind(line).unwrap();
        assert_eq!(keys, vec![VKey::N]);
        assert_eq!(cmd, "next-window");
    }

    #[test]
    fn conf_bind_key_form() {
        let line = "bind-key r source-file ~/.tmux.conf";
        let (keys, cmd) = parse_conf_bind(line).unwrap();
        assert_eq!(keys, vec![VKey::R]);
        assert_eq!(cmd, "source-file");
    }

    #[test]
    fn conf_bind_non_prefix_table_skipped() {
        // -T copy-mode-vi should be skipped (not prefix table)
        let line = "bind-key -T copy-mode-vi y send-keys -X copy-pipe-and-cancel";
        assert!(parse_conf_bind(line).is_none());
    }
}
