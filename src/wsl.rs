/// Shared WSL command execution utility.
///
/// Used by `tmux_detect`, `codex_poll`, and `setup` to run commands in WSL.

/// Run a command in WSL via `wsl -e bash -lc` (login shell, PATH-aware).
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

/// Write `content` to a WSL path (e.g. `~/.claude/hooks/ds4cc-state.sh`).
///
/// Uses `bash -c 'mkdir -p ... && cat > ...'` with content piped to stdin.
/// Tilde in `wsl_path` is kept as-is — bash expands `~` in the command.
/// Returns true if the file was written successfully.
pub fn wsl_write(wsl_path: &str, content: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Bash expands $HOME but not ~ inside double-quoted command substitutions,
    // so we convert ~ → $HOME for reliable expansion.
    let expanded = wsl_path.replace('~', "$HOME");
    let cmd = format!(
        r#"mkdir -p "$(dirname "{expanded}")" && cat > "{expanded}""#
    );

    let mut child = match Command::new("wsl")
        .args(["-e", "bash", "-c", &cmd])
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(content.as_bytes());
        drop(stdin); // close pipe → EOF → cat finishes
    }

    child.wait().map(|s| s.success()).unwrap_or(false)
}
