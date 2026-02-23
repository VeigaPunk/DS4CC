/// TOML configuration with sensible defaults.
/// No config file is required to run — defaults work out of the box.

use serde::Deserialize;

/// Top-level configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub lightbar: LightbarConfig,
    pub buttons: ButtonConfig,
    pub scroll: ScrollConfig,
    pub tmux: TmuxConfig,
    pub codex: CodexConfig,
    /// Directory where agent state files are written (ds4cc_agent_*)
    pub state_dir: String,
    pub poll_interval_ms: u64,
    /// Seconds after "done" before auto-transitioning to "idle" (0 = disabled)
    pub idle_timeout_s: u64,
    /// Seconds before a "working" agent file is considered stale (crashed session)
    pub stale_timeout_s: u64,
    /// Seconds an individual agent must be idle before an attention rumble fires (0 = disabled)
    pub idle_reminder_s: u64,
}

/// Lightbar color configuration per agent state.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LightbarConfig {
    pub idle: ColorConfig,
    pub working: ColorConfig,
    pub done: ColorConfig,
    pub error: ColorConfig,
    /// Pulse speed for working state (full cycle in ms)
    pub pulse_period_ms: u64,
}

/// RGB color.
#[derive(Debug, Clone, Deserialize)]
pub struct ColorConfig {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Right stick scroll configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScrollConfig {
    /// Dead zone radius around center (0-127). Values within this range are ignored.
    pub dead_zone: u8,
    /// Scroll speed multiplier. 1.0 = normal, 2.0 = double speed.
    pub sensitivity: f32,
    /// Enable horizontal scrolling (X axis).
    pub horizontal: bool,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Self {
            dead_zone: 20,
            sensitivity: 1.0,
            horizontal: true,
        }
    }
}

/// Tmux integration configuration.
///
/// Button values are **tmux action names** (e.g., "previous-window") by default.
/// At startup, these are resolved to actual key combos by querying the running
/// tmux server. If auto-detection fails, well-known tmux defaults are used.
///
/// You can also specify direct key combos (e.g., "p", "Shift+7") to bypass
/// the action-name resolution — useful for custom overrides.
///
/// Empty strings mean "unmapped" — the button does nothing in Tmux profile.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TmuxConfig {
    /// Enable tmux profile (PS button cycles to it).
    pub enabled: bool,
    /// Auto-detect prefix and key bindings from tmux via WSL.
    /// When true, button action names are resolved to actual keys automatically.
    pub auto_detect: bool,
    /// Tmux prefix key combo (e.g., "Ctrl+B"). Used as fallback if auto-detect fails.
    pub prefix: String,
    // Button → tmux action names or direct key combos (empty = unmapped)
    pub l1: String,
    pub r1: String,
    pub l2: String,
    pub r2: String,
    pub l3: String,
    pub r3: String,
    pub square: String,
    pub share: String,
    pub options: String,
    pub touchpad: String,
}

impl Default for TmuxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_detect: true,
            prefix: "Ctrl+B".into(),         // tmux default, overridden by auto-detect
            l1: "previous-window".into(),
            r1: "next-window".into(),
            l2: "".into(),                    // unmapped
            r2: "kill-window".into(),
            l3: "".into(),                    // unmapped
            r3: "".into(),                    // unmapped (R3 = Ctrl+P direct)
            square: "new-window".into(),
            share: "".into(),                 // unmapped
            options: "".into(),               // unmapped
            touchpad: "".into(),              // unmapped
        }
    }
}

/// Codex JSONL poller configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CodexConfig {
    /// Enable native Codex JSONL polling via WSL UNC paths.
    pub enabled: bool,
    /// Seconds the task must run before "done" fires (shorter tasks go straight to idle).
    pub done_threshold_s: u64,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            done_threshold_s: 600, // 10 minutes
        }
    }
}

/// Button mapping configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ButtonConfig {
    pub cross: String,
    pub circle: String,
    pub square: String,
    pub triangle: String,
    pub l1: String,
    pub r1: String,
    pub dpad_up: String,
    pub dpad_down: String,
    pub dpad_left: String,
    pub dpad_right: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lightbar: LightbarConfig::default(),
            buttons: ButtonConfig::default(),
            scroll: ScrollConfig::default(),
            tmux: TmuxConfig::default(),
            codex: CodexConfig::default(),
            state_dir: default_state_dir(),
            poll_interval_ms: 500, // 2Hz
            idle_timeout_s: 30,
            stale_timeout_s: 600, // 10 minutes
            idle_reminder_s: 480, // 8 minutes per-agent
        }
    }
}

impl Default for LightbarConfig {
    fn default() -> Self {
        Self {
            idle: ColorConfig { r: 255, g: 140, b: 0 },   // orange
            working: ColorConfig { r: 0, g: 100, b: 255 }, // blue
            done: ColorConfig { r: 0, g: 255, b: 0 },     // green
            error: ColorConfig { r: 0, g: 0, b: 0 },       // off (configurable)
            pulse_period_ms: 2000,
        }
    }
}

impl Default for ButtonConfig {
    fn default() -> Self {
        Self {
            cross: "Enter".into(),
            circle: "Escape".into(),
            square: "new_session".into(),
            triangle: "Tab".into(),
            l1: "Shift+Alt+Tab".into(),
            r1: "Alt+Tab".into(),
            dpad_up: "Up".into(),
            dpad_down: "Down".into(),
            dpad_left: "Left".into(),
            dpad_right: "Right".into(),
        }
    }
}

fn default_state_dir() -> String {
    if let Ok(temp) = std::env::var("TEMP") {
        temp
    } else {
        r"C:\Temp".into()
    }
}

impl Config {
    /// Load config from the default config file path, or return defaults if not found.
    pub fn load() -> Self {
        let config_path = config_file_path();
        match std::fs::read_to_string(&config_path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => {
                    log::info!("Loaded config from {config_path}");
                    config
                }
                Err(e) => {
                    log::warn!("Failed to parse config file {config_path}: {e}. Using defaults.");
                    Self::default()
                }
            },
            Err(_) => {
                log::info!("No config file found at {config_path}. Using defaults.");
                Self::default()
            }
        }
    }
}

fn config_file_path() -> String {
    if let Ok(appdata) = std::env::var("APPDATA") {
        format!("{appdata}\\ds4cc\\config.toml")
    } else {
        "ds4cc.toml".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config = Config::default();
        assert_eq!(config.poll_interval_ms, 500);
        assert_eq!(config.lightbar.idle.r, 255);
        assert_eq!(config.lightbar.idle.g, 140);
        assert_eq!(config.buttons.cross, "Enter");
    }

    #[test]
    fn deserialize_partial_toml() {
        let toml_str = r#"
            poll_interval_ms = 250

            [lightbar.idle]
            r = 100
            g = 100
            b = 100
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.poll_interval_ms, 250);
        assert_eq!(config.lightbar.idle.r, 100);
        // Other fields should be defaults
        assert_eq!(config.lightbar.working.b, 255);
        assert_eq!(config.buttons.cross, "Enter");
    }
}
