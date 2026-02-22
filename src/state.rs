/// State file poller: reads agent state from disk at ~2Hz.
///
/// The CLI tool (or a hook) writes a single word to the state file:
///   idle | working | done | error

use std::path::PathBuf;
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

/// Polls the state file and sends state changes to a channel.
pub async fn poll_state_file(
    path: PathBuf,
    poll_ms: u64,
    tx: tokio::sync::watch::Sender<AgentState>,
) {
    let mut ticker = interval(Duration::from_millis(poll_ms));
    let mut last_state = AgentState::Idle;

    loop {
        ticker.tick().await;
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                if let Some(new_state) = AgentState::parse(&contents) {
                    if new_state != last_state {
                        log::info!("State changed: {last_state} → {new_state}");
                        last_state = new_state;
                        let _ = tx.send(new_state);
                    }
                }
            }
            Err(_) => {
                // File doesn't exist or can't be read — stay in current state
            }
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
}
