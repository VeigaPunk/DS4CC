/// Auto-setup Codex hook bridge via WSL.
///
/// Embeds all Codex hook scripts at compile time and deploys them to
/// `~/.codex/hooks/` in WSL on daemon startup. The bridge tails Codex
/// session JSONL files and maps events to Claude-style hooks, driving
/// the same state-file system the daemon polls for lightbar updates.
///
/// Skips silently if WSL is unavailable or Codex is not installed.

use crate::wsl::run_wsl;
use base64::Engine;

// ── Embedded scripts (compiled into the binary) ─────────────────────

const BRIDGE_PY: &str = include_str!("../hooks/codex/codex-hook-bridge.py");
const STATE_SH: &str = include_str!("../hooks/codex/gamepadcc-state.sh");
const HOOKS_JSON: &str = include_str!("../hooks/codex/hooks.json");
const START_SH: &str = include_str!("../hooks/codex/start.sh");
const STOP_SH: &str = include_str!("../hooks/codex/stop.sh");
const STATUS_SH: &str = include_str!("../hooks/codex/status.sh");
const INSTALL_SH: &str = include_str!("../hooks/codex/install-from-claude.sh");

// ── Public API ──────────────────────────────────────────────────────

/// Deploy Codex hook scripts to WSL and start the bridge.
///
/// Returns `true` if setup completed successfully.
/// Returns `false` if WSL is unavailable, Codex is not installed, or setup fails.
pub fn setup() -> bool {
    log::info!("Checking Codex hook bridge setup...");
    let start = std::time::Instant::now();

    // Check WSL is available
    if run_wsl("echo ok").is_none() {
        log::debug!("WSL not available, skipping Codex hook setup");
        return false;
    }

    // Check if Codex is installed (~/.codex/ exists)
    match run_wsl("test -d ~/.codex && echo yes") {
        Some(output) if output.trim() == "yes" => {}
        _ => {
            log::debug!("Codex not installed (~/.codex/ not found), skipping hook setup");
            return false;
        }
    }

    // Create hooks directory
    if run_wsl("mkdir -p ~/.codex/hooks").is_none() {
        log::warn!("Failed to create ~/.codex/hooks/ directory");
        return false;
    }

    // Deploy managed scripts (always overwrite — these are ours)
    let managed_scripts: &[(&str, &str)] = &[
        ("codex-hook-bridge.py", BRIDGE_PY),
        ("gamepadcc-state.sh", STATE_SH),
        ("start.sh", START_SH),
        ("stop.sh", STOP_SH),
        ("status.sh", STATUS_SH),
        ("install-from-claude.sh", INSTALL_SH),
    ];

    for (filename, content) in managed_scripts {
        if !deploy_script(filename, content) {
            log::warn!("Failed to deploy {filename}");
            return false;
        }
    }

    // Deploy hooks.json only if it doesn't exist (user may have customized)
    match run_wsl("test -f ~/.codex/hooks.json && echo exists") {
        Some(output) if output.trim() == "exists" => {
            log::debug!("hooks.json already exists, not overwriting");
        }
        _ => {
            if !deploy_file("hooks.json", HOOKS_JSON, "~/.codex/hooks.json") {
                log::warn!("Failed to deploy hooks.json");
                return false;
            }
            log::info!("Deployed ~/.codex/hooks.json");
        }
    }

    // Make scripts executable
    if run_wsl("chmod +x ~/.codex/hooks/*.sh ~/.codex/hooks/*.py").is_none() {
        log::warn!("Failed to chmod scripts");
    }

    // Start the bridge if not already running
    match run_wsl("~/.codex/hooks/status.sh 2>/dev/null") {
        Some(output) if output.contains("running") => {
            log::info!("Codex hook bridge already running");
        }
        _ => {
            match run_wsl("~/.codex/hooks/start.sh 2>/dev/null") {
                Some(output) => {
                    let msg = output.trim();
                    if !msg.is_empty() {
                        log::info!("Codex bridge: {msg}");
                    }
                }
                None => {
                    log::warn!("Failed to start Codex hook bridge (tmux may not be available)");
                }
            }
        }
    }

    let elapsed = start.elapsed();
    log::info!("Codex hook setup complete (took {elapsed:?})");
    true
}

// ── Internal helpers ────────────────────────────────────────────────

/// Deploy a script to `~/.codex/hooks/<filename>` using base64 transfer.
fn deploy_script(filename: &str, content: &str) -> bool {
    let target = format!("~/.codex/hooks/{filename}");
    deploy_file(filename, content, &target)
}

/// Deploy content to a target path in WSL using base64 encoding.
///
/// Base64 avoids shell escaping issues with special characters in scripts.
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
