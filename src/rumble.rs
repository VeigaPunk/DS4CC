/// Rumble engine: fires haptic patterns on state transitions and time thresholds.
///
/// Working → Done (>= 5 min):  two short pulses (notification feel)
/// Idle > 3 min:                single strong pulse (attention reminder)
/// Error:                       no rumble — agent keeps resolving, not worth alarming

use crate::state::AgentState;
use tokio::time::{sleep, Duration};

/// A rumble command: intensity (0-255) for left and right motors, plus duration.
#[derive(Debug, Clone, Copy)]
pub struct RumbleStep {
    pub left: u8,
    pub right: u8,
    pub duration_ms: u64,
}

/// Determine the rumble pattern for a state transition.
/// Returns None if no rumble should fire.
pub fn pattern_for_transition(from: AgentState, to: AgentState) -> Option<Vec<RumbleStep>> {
    match (from, to) {
        (AgentState::Working, AgentState::Done) => Some(vec![
            RumbleStep { left: 180, right: 180, duration_ms: 120 },
            RumbleStep { left: 0, right: 0, duration_ms: 100 }, // pause
            RumbleStep { left: 180, right: 180, duration_ms: 120 },
        ]),
        _ => None,
    }
}

/// Rumble pattern for the idle attention reminder (agent idle > threshold).
pub fn idle_reminder_pattern() -> Vec<RumbleStep> {
    vec![RumbleStep { left: 255, right: 255, duration_ms: 300 }]
}

/// Execute a rumble pattern by calling `set_rumble` for each step.
/// `set_rumble` receives (left_intensity, right_intensity) and should write
/// the output report to the controller.
pub async fn play_pattern<F>(pattern: &[RumbleStep], mut set_rumble: F)
where
    F: FnMut(u8, u8),
{
    for step in pattern {
        set_rumble(step.left, step.right);
        sleep(Duration::from_millis(step.duration_ms)).await;
    }
    // Always end with motors off
    set_rumble(0, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_transition_has_pattern() {
        let pattern = pattern_for_transition(AgentState::Working, AgentState::Done);
        assert!(pattern.is_some());
        let steps = pattern.unwrap();
        assert_eq!(steps.len(), 3); // pulse, pause, pulse
    }

    #[test]
    fn error_transition_no_rumble() {
        // Error state is intentionally silent — agent self-recovers, no alarm needed.
        assert!(pattern_for_transition(AgentState::Idle, AgentState::Error).is_none());
        assert!(pattern_for_transition(AgentState::Working, AgentState::Error).is_none());
    }

    #[test]
    fn idle_to_working_no_rumble() {
        assert!(pattern_for_transition(AgentState::Idle, AgentState::Working).is_none());
    }
}
