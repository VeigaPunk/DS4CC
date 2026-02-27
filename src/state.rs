/// State poller: scans per-agent state files and aggregates into a single state.
///
/// Each Claude Code session writes its own file: `ds4cc_agent_<session_id>`
/// containing a single word: idle | working | done | error
///
/// The poller scans all matching files and applies priority:
///   working > error > done > idle
///
/// "working" files older than `stale_timeout_s` are ignored (crashed sessions).
/// After `idle_timeout_s` in done, auto-transitions to idle.
/// Error mirrors Working visually (same blue pulse, no rumble) — agent is still active,
/// self-recovering silently. Working still takes priority over Error in aggregation.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration as StdDuration, Instant, SystemTime};
use tokio::sync::mpsc;
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

/// Scan all `ds4cc_agent_*` files in the state directory.
/// Returns the aggregated state and a map of agent_id → state for per-agent tracking.
/// Ignores "working" files older than `stale_timeout`.
fn scan_agent_states(
    state_dir: &PathBuf,
    stale_timeout: StdDuration,
) -> (AgentState, HashMap<String, AgentState>) {
    let pattern = "ds4cc_agent_";
    let now = SystemTime::now();
    let mut best = AgentState::Idle;
    let mut agents = HashMap::new();

    let entries = match std::fs::read_dir(state_dir) {
        Ok(e) => e,
        Err(_) => return (AgentState::Idle, agents),
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only match agent files, skip timestamp files (*_start)
        if !name_str.starts_with(pattern) || name_str.ends_with("_start") {
            continue;
        }

        let agent_id = name_str[pattern.len()..].to_string();

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
                            let _ = std::fs::remove_file(&path);
                            continue;
                        }
                    }
                }
            }
        }

        // Delete idle files immediately — they don't contribute to aggregation
        // and their removal lets agent_tracker self-prune finished sessions.
        if state == AgentState::Idle {
            let _ = std::fs::remove_file(&path);
            continue;
        }

        agents.insert(agent_id, state);

        if state.priority() > best.priority() {
            best = state;
        }
    }

    (best, agents)
}

/// Remove all "done" agent files from disk so they don't re-trigger after auto-idle.
fn clean_done_files(state_dir: &PathBuf) {
    let entries = match std::fs::read_dir(state_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("ds4cc_agent_") || name_str.ends_with("_start") {
            continue;
        }
        let contents = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if AgentState::parse(&contents) == Some(AgentState::Done) {
            let _ = std::fs::remove_file(entry.path());
            // Also remove its timestamp file
            let start_path = format!("{}_start", entry.path().display());
            let _ = std::fs::remove_file(start_path);
        }
    }
}

/// Backward-compatible wrapper for tests.
#[cfg(test)]
fn aggregate_agent_states(state_dir: &PathBuf, stale_timeout: StdDuration) -> AgentState {
    scan_agent_states(state_dir, stale_timeout).0
}

/// Polls agent state files and sends aggregated state changes to a channel.
/// Tracks per-agent state transitions:
/// - Idle reminder: fires when any individual agent has been idle >= `idle_reminder_s`
/// - Done rumble: fires when any individual agent transitions Working → Done
///   after working >= `done_threshold_ms`
pub async fn poll_state_file(
    state_dir: PathBuf,
    poll_ms: u64,
    idle_timeout_s: u64,
    stale_timeout_s: u64,
    idle_reminder_s: u64,
    done_threshold_ms: u64,
    subagent_filter_s: u64,
    tx: tokio::sync::watch::Sender<AgentState>,
    idle_reminder_tx: mpsc::Sender<()>,
    done_rumble_tx: mpsc::Sender<()>,
) {
    let mut ticker = interval(Duration::from_millis(poll_ms));
    let mut last_state = AgentState::Idle;
    let mut state_changed_at = Instant::now();
    let stale_timeout = StdDuration::from_secs(stale_timeout_s);
    let idle_reminder_dur = Duration::from_secs(idle_reminder_s);
    let done_threshold = Duration::from_millis(done_threshold_ms);
    let subagent_filter = Duration::from_secs(subagent_filter_s);

    // Per-agent tracking: agent_id → (last known state, timestamp of that state)
    let mut agent_tracker: HashMap<String, (AgentState, Instant)> = HashMap::new();
    // Agents whose idle reminder has already fired for the current idle stretch
    let mut reminder_fired: HashSet<String> = HashSet::new();
    // Cooldown: after firing an idle reminder, skip per-agent checks for 5s
    let mut reminder_cooldown: Option<Instant> = None;

    loop {
        ticker.tick().await;

        // Auto-idle: if we've been in "done" long enough, transition to idle.
        // Also remove the "done" state files from disk so the next scan doesn't
        // re-read them and bounce back to Done (which caused an infinite loop).
        if idle_timeout_s > 0
            && last_state == AgentState::Done
            && state_changed_at.elapsed() >= Duration::from_secs(idle_timeout_s)
        {
            log::info!("Auto-idle: {last_state} → idle (after {idle_timeout_s}s)");
            clean_done_files(&state_dir);
            last_state = AgentState::Idle;
            state_changed_at = Instant::now();
            let _ = tx.send(AgentState::Idle);
            continue;
        }

        let (aggregated, current_agents) = scan_agent_states(&state_dir, stale_timeout);

        if aggregated != last_state {
            log::info!("State changed: {last_state} → {aggregated}");
            last_state = aggregated;
            state_changed_at = Instant::now();
            let _ = tx.send(aggregated);
        }

        let now = Instant::now();

        // Resolve cooldown
        let in_cooldown = match reminder_cooldown {
            Some(cd) if now.duration_since(cd) < Duration::from_secs(5) => true,
            Some(_) => { reminder_cooldown = None; false }
            None => false,
        };

        // 1. Update tracker for agents with active state files
        for (id, state) in &current_agents {
            match agent_tracker.get(id) {
                Some((prev, _)) if *prev == *state => { /* unchanged */ }
                Some((prev, since)) => {
                    // State changed — check Working → Done
                    let elapsed = now.duration_since(*since);
                    if *prev == AgentState::Working && *state == AgentState::Done {
                        if elapsed >= done_threshold {
                            log::info!(
                                "Per-agent done: agent {id} worked for {}s → rumble",
                                elapsed.as_secs()
                            );
                            let _ = done_rumble_tx.try_send(());
                        } else {
                            log::debug!(
                                "Per-agent done: agent {id} worked {}s (< {}s threshold) — skipping rumble",
                                elapsed.as_secs(),
                                done_threshold.as_secs()
                            );
                        }
                    }
                    agent_tracker.insert(id.clone(), (*state, now));
                    reminder_fired.remove(id);
                }
                None => {
                    agent_tracker.insert(id.clone(), (*state, now));
                }
            }
        }

        // 2. Transition disappeared agents to Idle in-memory.
        //    scan_agent_states deletes idle files immediately (keeps the dir lean);
        //    we continue tracking their idle duration here so the reminder can fire.
        //    Short-lived agents (worked < subagent_filter) are likely subagents —
        //    mark their reminder as already fired so they never trigger rumble.
        for id in agent_tracker.keys().cloned().collect::<Vec<_>>() {
            if !current_agents.contains_key(&id) {
                if let Some((state, since)) = agent_tracker.get_mut(&id) {
                    if *state != AgentState::Idle {
                        let worked = now.duration_since(*since);
                        let is_subagent =
                            *state == AgentState::Working && worked < subagent_filter;
                        if is_subagent {
                            log::debug!(
                                "Subagent filtered: {id} (worked {}s < {}s threshold)",
                                worked.as_secs(),
                                subagent_filter.as_secs()
                            );
                        }
                        *state = AgentState::Idle;
                        *since = now;
                        if is_subagent {
                            reminder_fired.insert(id.clone());
                        } else {
                            reminder_fired.remove(&id);
                        }
                    }
                }
            }
        }

        // 3. Check idle reminders across all tracked agents (skip during cooldown)
        if !in_cooldown {
            let mut fired_this_tick = false;
            for (id, (state, since)) in &agent_tracker {
                if idle_reminder_s > 0
                    && *state == AgentState::Idle
                    && !reminder_fired.contains(id)
                    && now.duration_since(*since) >= idle_reminder_dur
                {
                    log::info!(
                        "Per-agent idle reminder: agent {id} idle for {}s",
                        now.duration_since(*since).as_secs()
                    );
                    reminder_fired.insert(id.clone());
                    fired_this_tick = true;
                }
            }
            if fired_this_tick {
                let _ = idle_reminder_tx.try_send(());
                reminder_cooldown = Some(now);
            }
        }

        // 4. Prune idle agents whose reminder has already fired.
        //    Active agents are always kept. Idle-in-memory agents are kept only
        //    while their reminder is still pending.
        agent_tracker.retain(|id, (state, _)| {
            if current_agents.contains_key(id) { return true; }
            idle_reminder_s > 0 && *state == AgentState::Idle && !reminder_fired.contains(id)
        });
        reminder_fired.retain(|id| agent_tracker.contains_key(id));
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
        let dir = std::env::temp_dir().join("ds4cc_test_empty");
        let _ = std::fs::create_dir_all(&dir);
        let result = aggregate_agent_states(&dir, StdDuration::from_secs(600));
        assert_eq!(result, AgentState::Idle);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn aggregate_multiple_agents() {
        let dir = std::env::temp_dir().join("ds4cc_test_multi");
        let _ = std::fs::create_dir_all(&dir);

        // Agent A is working, Agent B is idle
        std::fs::write(dir.join("ds4cc_agent_aaa"), "working").unwrap();
        std::fs::write(dir.join("ds4cc_agent_bbb"), "idle").unwrap();
        let result = aggregate_agent_states(&dir, StdDuration::from_secs(600));
        assert_eq!(result, AgentState::Working);

        // Agent A finishes (idle), Agent B still idle
        std::fs::write(dir.join("ds4cc_agent_aaa"), "idle").unwrap();
        let result = aggregate_agent_states(&dir, StdDuration::from_secs(600));
        assert_eq!(result, AgentState::Idle);

        // Agent A done, Agent B working → working wins
        std::fs::write(dir.join("ds4cc_agent_aaa"), "done").unwrap();
        std::fs::write(dir.join("ds4cc_agent_bbb"), "working").unwrap();
        let result = aggregate_agent_states(&dir, StdDuration::from_secs(600));
        assert_eq!(result, AgentState::Working);

        // Clean up
        let _ = std::fs::remove_file(dir.join("ds4cc_agent_aaa"));
        let _ = std::fs::remove_file(dir.join("ds4cc_agent_bbb"));
        let _ = std::fs::remove_dir(&dir);
    }
}
