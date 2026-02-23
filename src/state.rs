/// State poller: scans per-agent state files and aggregates into a single state.
///
/// Each Claude Code session writes its own file: `gamepadcc_agent_<session_id>`
/// containing a single word: idle | working | done | error
///
/// The poller scans all matching files and applies priority:
///   working > error > done > idle
///
/// "working" files older than `stale_timeout_s` are ignored (crashed sessions).
/// After `idle_timeout_s` in done, auto-transitions to idle.
/// Error has no special visual/haptic treatment — the agent self-recovers silently.

use std::path::PathBuf;
use std::time::{Duration as StdDuration, Instant, SystemTime};
use tokio::time::{interval, Duration};

/// Agent states that map to lightbar colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Working,
    Done,
    Error,
}

impl AgentState {
    /// Parse from the state file content. Trims whitespace, case-insensitive.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "idle" => Some(AgentState::Idle),
            "working" => Some(AgentState::Working),
            "done" => Some(AgentState::Done),
            "error" => Some(AgentState::Error),
            _ => None,
        }
    }

    /// Priority for aggregation (higher = wins).
    fn priority(self) -> u8 {
        match self {
            AgentState::Idle => 0,
            AgentState::Done => 1,
            AgentState::Error => 2,
            AgentState::Working => 3,
        }
    }
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Idle => f.write_str("idle"),
            AgentState::Working => f.write_str("working"),
            AgentState::Done => f.write_str("done"),
            AgentState::Error => f.write_str("error"),
        }
    }
}

/// Scan all `gamepadcc_agent_*` files in the state directory and aggregate.
/// Ignores "working" files older than `stale_timeout`.
fn aggregate_agent_states(state_dir: &PathBuf, stale_timeout: StdDuration) -> AgentState {
    let pattern = "gamepadcc_agent_";
    let now = SystemTime::now();
    let mut best = AgentState::Idle;

    let entries = match std::fs::read_dir(state_dir) {
        Ok(e) => e,
        Err(_) => return AgentState::Idle,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only match agent files, skip timestamp files (*_start)
        if !name_str.starts_with(pattern) || name_str.ends_with("_start") {
            continue;
        }

        let path = entry.path();
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let state = match AgentState::parse(&contents) {
            Some(s) => s,
            None => continue,
        };

        // Check staleness for "working" state — ignore crashed sessions
        if state == AgentState::Working {
            if let Ok(metadata) = std::fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age > stale_timeout {
                            log::debug!("Ignoring stale agent file: {name_str} ({}s old)", age.as_secs());
                            // Clean up stale file
                            let _ = std::fs::remove_file(&path);
                            continue;
                        }
                    }
                }
            }
        }

        // Skip idle agents — they don't contribute
        if state == AgentState::Idle {
            continue;
        }

        if state.priority() > best.priority() {
            best = state;
        }
    }

    best
}

/// Polls agent state files and sends aggregated state changes to a channel.
pub async fn poll_state_file(
    state_dir: PathBuf,
    poll_ms: u64,
    idle_timeout_s: u64,
    stale_timeout_s: u64,
    tx: tokio::sync::watch::Sender<AgentState>,
) {
    let mut ticker = interval(Duration::from_millis(poll_ms));
    let mut last_state = AgentState::Idle;
    let mut state_changed_at = Instant::now();
    let stale_timeout = StdDuration::from_secs(stale_timeout_s);

    loop {
        ticker.tick().await;

        // Auto-idle: if we've been in "done" long enough, transition to idle
        if idle_timeout_s > 0
            && last_state == AgentState::Done
            && state_changed_at.elapsed() >= Duration::from_secs(idle_timeout_s)
        {
            log::info!("Auto-idle: {last_state} → idle (after {idle_timeout_s}s)");
            last_state = AgentState::Idle;
            state_changed_at = Instant::now();
            let _ = tx.send(AgentState::Idle);
            continue;
        }

        let aggregated = aggregate_agent_states(&state_dir, stale_timeout);

        if aggregated != last_state {
            log::info!("State changed: {last_state} → {aggregated}");
            last_state = aggregated;
            state_changed_at = Instant::now();
            let _ = tx.send(aggregated);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_states() {
        assert_eq!(AgentState::parse("idle"), Some(AgentState::Idle));
        assert_eq!(AgentState::parse("WORKING"), Some(AgentState::Working));
        assert_eq!(AgentState::parse("  done\n"), Some(AgentState::Done));
        assert_eq!(AgentState::parse("Error"), Some(AgentState::Error));
        assert_eq!(AgentState::parse("unknown"), None);
        assert_eq!(AgentState::parse(""), None);
    }

    #[test]
    fn priority_order() {
        assert!(AgentState::Working.priority() > AgentState::Error.priority());
        assert!(AgentState::Error.priority() > AgentState::Done.priority());
        assert!(AgentState::Done.priority() > AgentState::Idle.priority());
    }

    #[test]
    fn aggregate_empty_dir() {
        let dir = std::env::temp_dir().join("gamepadcc_test_empty");
        let _ = std::fs::create_dir_all(&dir);
        let result = aggregate_agent_states(&dir, StdDuration::from_secs(600));
        assert_eq!(result, AgentState::Idle);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn aggregate_multiple_agents() {
        let dir = std::env::temp_dir().join("gamepadcc_test_multi");
        let _ = std::fs::create_dir_all(&dir);

        // Agent A is working, Agent B is idle
        std::fs::write(dir.join("gamepadcc_agent_aaa"), "working").unwrap();
        std::fs::write(dir.join("gamepadcc_agent_bbb"), "idle").unwrap();
        let result = aggregate_agent_states(&dir, StdDuration::from_secs(600));
        assert_eq!(result, AgentState::Working);

        // Agent A finishes (idle), Agent B still idle
        std::fs::write(dir.join("gamepadcc_agent_aaa"), "idle").unwrap();
        let result = aggregate_agent_states(&dir, StdDuration::from_secs(600));
        assert_eq!(result, AgentState::Idle);

        // Agent A done, Agent B working → working wins
        std::fs::write(dir.join("gamepadcc_agent_aaa"), "done").unwrap();
        std::fs::write(dir.join("gamepadcc_agent_bbb"), "working").unwrap();
        let result = aggregate_agent_states(&dir, StdDuration::from_secs(600));
        assert_eq!(result, AgentState::Working);

        // Clean up
        let _ = std::fs::remove_file(dir.join("gamepadcc_agent_aaa"));
        let _ = std::fs::remove_file(dir.join("gamepadcc_agent_bbb"));
        let _ = std::fs::remove_dir(&dir);
    }
}
