/// Shared WSL command execution utility.
///
/// Used by `tmux_detect` and `codex_setup` to run commands in WSL.

/// Run a command in WSL via `wsl -e bash -lc`.
/// Returns stdout on success, None on failure.
pub fn run_wsl(cmd: &str) -> Option<String> {
    let output = std::process::Command::new("wsl")
        .args(["-e", "bash", "-lc", cmd])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}
