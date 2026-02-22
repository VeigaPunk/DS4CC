/// Lightbar engine: maps agent state + elapsed time to RGB color.
///
/// States:
///   Idle    → orange, solid
///   Working → blue, pulsing (sinusoidal brightness)
///   Done    → green, solid
///   Error   → red, solid

use crate::config::LightbarConfig;
use crate::state::AgentState;

/// Compute the current lightbar RGB given state and time.
pub fn compute_color(
    config: &LightbarConfig,
    state: AgentState,
    elapsed_ms: u64,
) -> (u8, u8, u8) {
    match state {
        AgentState::Idle => (config.idle.r, config.idle.g, config.idle.b),
        AgentState::Done => (config.done.r, config.done.g, config.done.b),
        AgentState::Error => (config.error.r, config.error.g, config.error.b),
        AgentState::Working => {
            // Sinusoidal pulse: brightness oscillates between 0.3 and 1.0
            let period = config.pulse_period_ms as f64;
            let phase = (elapsed_ms as f64 / period) * std::f64::consts::TAU;
            let brightness = 0.65 + 0.35 * phase.sin(); // range [0.3, 1.0]
            let r = (config.working.r as f64 * brightness) as u8;
            let g = (config.working.g as f64 * brightness) as u8;
            let b = (config.working.b as f64 * brightness) as u8;
            (r, g, b)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LightbarConfig;

    fn default_config() -> LightbarConfig {
        LightbarConfig::default()
    }

    #[test]
    fn idle_is_solid_orange() {
        let cfg = default_config();
        let (r, g, b) = compute_color(&cfg, AgentState::Idle, 0);
        assert_eq!((r, g, b), (255, 140, 0));
        // Same color regardless of time
        let (r2, g2, b2) = compute_color(&cfg, AgentState::Idle, 5000);
        assert_eq!((r, g, b), (r2, g2, b2));
    }

    #[test]
    fn working_pulses() {
        let cfg = default_config();
        let (_, _, b0) = compute_color(&cfg, AgentState::Working, 0);
        // At quarter period (sin = 1.0), brightness should be max
        let quarter = cfg.pulse_period_ms / 4;
        let (_, _, b_max) = compute_color(&cfg, AgentState::Working, quarter);
        // At three-quarter period (sin = -1.0), brightness should be min
        let three_quarter = (cfg.pulse_period_ms * 3) / 4;
        let (_, _, b_min) = compute_color(&cfg, AgentState::Working, three_quarter);
        assert!(b_max > b_min);
        // b0 should be between min and max (sin(0)=0 → brightness=0.65)
        assert!(b0 > b_min);
        assert!(b0 < b_max);
    }

    #[test]
    fn done_is_solid_green() {
        let cfg = default_config();
        let (r, g, b) = compute_color(&cfg, AgentState::Done, 0);
        assert_eq!((r, g, b), (0, 255, 0));
    }

    #[test]
    fn error_is_solid_red() {
        let cfg = default_config();
        let (r, g, b) = compute_color(&cfg, AgentState::Error, 0);
        assert_eq!((r, g, b), (255, 0, 0));
    }
}
