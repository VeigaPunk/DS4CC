/// First-run setup: auto-install Claude Code hooks and OpenCode plugin via WSL.
///
/// Hook scripts are compiled into the binary with `include_str!`.  On first
/// run (and after a version bump) they are written to the correct WSL paths
/// and the Claude Code settings.json is updated automatically.
///
/// A version stamp at `%APPDATA%\ds4cc\hook_version` prevents redundant
/// reinstalls on subsequent startups — subsequent calls return in microseconds.
///
/// Everything here is best-effort.  If WSL is unavailable the daemon continues
/// to work normally (Codex polling is native and does not need WSL hooks).

use crate::wsl;

// ── Embedded hook content ────────────────────────────────────────────────────

/// Claude Code hook script (bash).
const HOOK_SH: &str = include_str!("../hooks/ds4cc-state.sh");

/// OpenCode plugin (JavaScript).
const OPENCODE_JS: &str = include_str!("../hooks/opencode/ds4cc-opencode.js");

/// Bump this suffix to force a reinstall on the next launch after an update.
/// In practice this just needs to change whenever the hook content changes.
const HOOKS_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-r3");

// ── Python one-liner for merging settings.json ──────────────────────────────
//
// Passed directly to: wsl bash -lc "python3 -c '<SNIPPET>'"
// Written as a true one-liner (semicolon-separated, no block control flow)
// so it survives the -c argument without multi-line indentation issues.
// Uses double-quotes throughout — safe inside single quotes for bash.

const MERGE_SETTINGS_PY: &str = concat!(
    "import json,os;",
    "p=os.path.expanduser(\"~/.claude/settings.json\");",
    "t=(open(p).read() if os.path.isfile(p) else \"\");",
    "c=(json.loads(t) if t.strip() else {});",
    "h=[{\"matcher\":\"\",\"hooks\":[{\"type\":\"command\",\"command\":\"~/.claude/hooks/ds4cc-state.sh\"}]}];",
    "c.setdefault(\"hooks\",{});",
    "c[\"hooks\"].update({\"UserPromptSubmit\":h,\"Stop\":h,\"PostToolUseFailure\":h});",
    "d=os.path.dirname(p);os.makedirs(d,exist_ok=True);",
    "f=open(p,\"w\");json.dump(c,f,indent=2);f.write(\"\\n\");f.close()"
);

// ── Public API ───────────────────────────────────────────────────────────────

/// What setup installed on this run (only populated on first run / after update).
#[derive(Debug)]
pub struct SetupResult {
    pub claude_code: bool,
    pub opencode: bool,
}

/// Run hook setup.
///
/// Returns `Some(SetupResult)` if hooks were installed/updated, `None` if
/// everything was already current (common case after first run).
///
/// This function is **blocking** — call it inside `spawn_blocking` from async.
pub fn run() -> Option<SetupResult> {
    // Fast path: already up to date
    if is_current() {
        log::debug!("setup: hooks current ({}), skipping", HOOKS_VERSION);
        return None;
    }

    // WSL availability check
    match wsl::run_wsl("echo ok") {
        Some(s) if s.trim() == "ok" => {}
        _ => {
            log::info!("setup: WSL unavailable — hook auto-install skipped");
            // Stamp anyway so we don't retry on every startup when there's no WSL.
            stamp();
            return None;
        }
    }

    log::info!("setup: installing hooks ({})", HOOKS_VERSION);

    let claude_code = install_claude_code_hook();
    let opencode = install_opencode_plugin();

    stamp();

    Some(SetupResult { claude_code, opencode })
}

// ── Claude Code ──────────────────────────────────────────────────────────────

fn install_claude_code_hook() -> bool {
    // Strip Windows CRLF line endings — bash rejects scripts with \r\n.
    let hook_sh = HOOK_SH.replace("\r\n", "\n");

    // Write hook script to WSL home (used by Claude Code CLI in WSL)
    if !wsl::wsl_write("~/.claude/hooks/ds4cc-state.sh", &hook_sh) {
        log::warn!("setup: failed to write ds4cc-state.sh to WSL");
        return false;
    }

    // Make executable in WSL
    wsl::run_wsl("chmod +x ~/.claude/hooks/ds4cc-state.sh");

    // Also write to Windows user home — Claude Desktop reads hooks from here.
    install_windows_hook(&hook_sh);

    // Merge hook entries into settings.json (WSL and Windows)
    merge_claude_settings();

    log::info!("setup: Claude Code hook installed → ~/.claude/hooks/ds4cc-state.sh");
    true
}

/// Write the hook script to the Windows user home `.claude\hooks\` directory.
/// Claude Desktop (Windows app) resolves `~` to %USERPROFILE% when running hooks,
/// so it reads from here rather than from the WSL home.
fn install_windows_hook(hook_sh: &str) {
    let profile = match std::env::var("USERPROFILE") {
        Ok(p) => p,
        Err(_) => {
            log::debug!("setup: USERPROFILE not set, skipping Windows hook install");
            return;
        }
    };

    let hooks_dir = std::path::Path::new(&profile).join(".claude").join("hooks");
    if let Err(e) = std::fs::create_dir_all(&hooks_dir) {
        log::warn!("setup: failed to create Windows hooks dir: {e}");
        return;
    }

    let hook_path = hooks_dir.join("ds4cc-state.sh");
    if let Err(e) = std::fs::write(&hook_path, hook_sh) {
        log::warn!("setup: failed to write Windows hook: {e}");
    } else {
        log::info!("setup: Windows hook installed → {}", hook_path.display());
    }
}

fn merge_claude_settings() {
    // MERGE_SETTINGS_PY is already a semicolon-joined one-liner (via concat!).
    // Wrap it directly in single quotes for bash -c.  Double-quotes inside the
    // snippet are safe because the outer delimiter is single-quote.
    let cmd = format!("python3 -c '{MERGE_SETTINGS_PY}'");
    if wsl::run_wsl(&cmd).is_none() {
        log::warn!("setup: settings.json merge failed (python3 not available?)");
        log::warn!("setup: run 'bash install-hooks.sh' from the DS4CC repo as a fallback");
    } else {
        log::info!("setup: ~/.claude/settings.json updated");
    }

    // Also merge into the Windows user settings.json for Claude Desktop.
    merge_windows_claude_settings();
}

/// Merge hook entries into `%USERPROFILE%\.claude\settings.json`.
/// Claude Desktop (Windows app) reads from this file, not from WSL's ~/.claude/settings.json.
fn merge_windows_claude_settings() {
    let profile = match std::env::var("USERPROFILE") {
        Ok(p) => p,
        Err(_) => return,
    };

    let settings_dir = std::path::Path::new(&profile).join(".claude");
    let settings_path = settings_dir.join("settings.json");

    // Read existing settings or start fresh
    let mut config: serde_json::Value = if settings_path.exists() {
        match std::fs::read_to_string(&settings_path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or(serde_json::json!({})),
            Err(_) => serde_json::json!({}),
        }
    } else {
        serde_json::json!({})
    };

    // Build the hook entry
    let hook_entry = serde_json::json!([{
        "matcher": "",
        "hooks": [{"type": "command", "command": "~/.claude/hooks/ds4cc-state.sh"}]
    }]);

    let hooks = config
        .as_object_mut()
        .and_then(|o| {
            o.entry("hooks")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .map(|h| {
                    h.insert("UserPromptSubmit".into(), hook_entry.clone());
                    h.insert("Stop".into(), hook_entry.clone());
                    h.insert("PostToolUseFailure".into(), hook_entry.clone());
                })
        });

    if hooks.is_none() {
        log::warn!("setup: failed to merge Windows settings.json hooks");
        return;
    }

    if let Err(e) = std::fs::create_dir_all(&settings_dir) {
        log::warn!("setup: failed to create Windows .claude dir: {e}");
        return;
    }

    match serde_json::to_string_pretty(&config) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&settings_path, json + "\n") {
                log::warn!("setup: failed to write Windows settings.json: {e}");
            } else {
                log::info!("setup: Windows settings.json updated → {}", settings_path.display());
            }
        }
        Err(e) => log::warn!("setup: failed to serialize Windows settings.json: {e}"),
    }
}

// ── OpenCode ─────────────────────────────────────────────────────────────────

fn install_opencode_plugin() -> bool {
    // Only install if OpenCode is present (binary or config dir)
    let detected = wsl::run_wsl(
        "command -v opencode >/dev/null 2>&1 || [ -d ~/.config/opencode ] && echo yes || echo no",
    )
    .map(|s| s.trim() == "yes")
    .unwrap_or(false);

    if !detected {
        return false;
    }

    let opencode_js = OPENCODE_JS.replace("\r\n", "\n");
    if !wsl::wsl_write("~/.config/opencode/plugins/ds4cc-opencode.js", &opencode_js) {
        log::warn!("setup: failed to write ds4cc-opencode.js");
        return false;
    }

    log::info!("setup: OpenCode plugin installed → ~/.config/opencode/plugins/ds4cc-opencode.js");
    log::info!("setup: restart OpenCode to activate the plugin");
    true
}

// ── Version stamp ─────────────────────────────────────────────────────────────

fn stamp_path() -> Option<std::path::PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(std::path::Path::new(&appdata).join("ds4cc").join("hook_version"))
}

fn is_current() -> bool {
    stamp_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim() == HOOKS_VERSION)
        .unwrap_or(false)
}

fn stamp() {
    let Some(path) = stamp_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, HOOKS_VERSION);
}
