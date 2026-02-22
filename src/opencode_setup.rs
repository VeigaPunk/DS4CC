/// Auto-setup OpenCode plugin for GamePadCC state integration via WSL.
///
/// Embeds the OpenCode plugin and state hook script at compile time and
/// deploys them to `~/.config/opencode/plugins/` in WSL on daemon startup.
///
/// Unlike the Codex bridge (which runs a separate Python daemon), the
/// OpenCode integration uses a native JS plugin that runs inside OpenCode
/// itself — no external process required.
///
/// Skips silently if WSL is unavailable or OpenCode is not installed.

use crate::wsl::run_wsl;
use base64::Engine;

// ── Embedded scripts (compiled into the binary) ─────────────────────

const BRIDGE_JS: &str = include_str!("../hooks/opencode/gamepadcc-bridge.js");
// Reuse the same enhanced state hook from Codex (identical functionality)
const STATE_SH: &str = include_str!("../hooks/codex/gamepadcc-state.sh");

// ── Public API ──────────────────────────────────────────────────────

/// Deploy OpenCode plugin and state hook to WSL.
///
/// Returns `true` if setup completed successfully.
/// Returns `false` if WSL is unavailable, OpenCode is not installed, or setup fails.
pub fn setup() -> bool {
    log::info!("Checking OpenCode plugin setup...");
    let start = std::time::Instant::now();

    // Check WSL is available
    if run_wsl("echo ok").is_none() {
        log::debug!("WSL not available, skipping OpenCode plugin setup");
        return false;
    }

    // Check if OpenCode config dir exists or can be created
    // OpenCode uses ~/.config/opencode/ for global config
    match run_wsl("command -v opencode >/dev/null 2>&1 || test -d ~/.config/opencode && echo yes") {
        Some(output) if output.trim() == "yes" => {}
        _ => {
            log::debug!("OpenCode not installed, skipping plugin setup");
            return false;
        }
    }

    // Create plugins directory
    if run_wsl("mkdir -p ~/.config/opencode/plugins").is_none() {
        log::warn!("Failed to create ~/.config/opencode/plugins/ directory");
        return false;
    }

    // Deploy the plugin (always overwrite — managed by GamePadCC)
    if !deploy_file("gamepadcc-bridge.js", BRIDGE_JS, "~/.config/opencode/plugins/gamepadcc-bridge.js") {
        log::warn!("Failed to deploy gamepadcc-bridge.js");
        return false;
    }

    // Deploy the state hook script (always overwrite — managed by GamePadCC)
    if !deploy_file("gamepadcc-state.sh", STATE_SH, "~/.config/opencode/plugins/gamepadcc-state.sh") {
        log::warn!("Failed to deploy gamepadcc-state.sh");
        return false;
    }

    // Make state script executable
    if run_wsl("chmod +x ~/.config/opencode/plugins/gamepadcc-state.sh").is_none() {
        log::warn!("Failed to chmod gamepadcc-state.sh");
    }

    let elapsed = start.elapsed();
    log::info!("OpenCode plugin setup complete (took {elapsed:?})");
    true
}

// ── Internal helpers ────────────────────────────────────────────────

/// Deploy content to a target path in WSL using base64 encoding.
fn deploy_file(label: &str, content: &str, target: &str) -> bool {
    let encoded = base64::engine::general_purpose::STANDARD.encode(content);
    let cmd = format!("echo '{encoded}' | base64 -d > {target}");

    if run_wsl(&cmd).is_some() {
        log::debug!("Deployed {label} -> {target}");
        true
    } else {
        log::warn!("Failed to write {target}");
        false
    }
}
