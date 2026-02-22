# GamePadCC v2

Windows daemon that bridges DualSense/DS4 controllers with CLI coding tools (Claude Code, Codex). Turns your controller into a full navigation device with state-driven lightbar feedback, tmux integration, and right-stick scrolling.

## Features

- **Button-to-keystroke mapping** via Windows `SendInput` with rising-edge detection
- **D-pad with two-frame confirmation + auto-repeat** (filters single-frame glitches)
- **Right stick scroll** with configurable dead zone, sensitivity, and horizontal support
- **Two profiles** (Default + Tmux) toggled by PS button, with system tray indicator
- **Tmux auto-detection** queries the running tmux server via WSL for prefix and key bindings
- **State-driven lightbar** reflects agent state (idle/working/done/error) with smooth pulse animations
- **Haptic rumble patterns** on state transitions (done = double tap, error = buzz)
- **Multi-agent aggregation** supports concurrent Claude Code + Codex + OpenCode sessions with priority-based state
- **Codex hook bridge** auto-deployed to WSL on startup (Python daemon that tails JSONL session files)
- **OpenCode plugin** auto-deployed to WSL on startup (native JS plugin, no external daemon)
- **Claude Code hooks** installed via `install-hooks.sh`

## Supported Controllers

| Controller          | USB | Bluetooth |
|---------------------|-----|-----------|
| DualSense           | Yes | Yes       |
| DualSense Edge      | Yes | Yes       |
| DualShock 4 v1      | Yes | Yes       |
| DualShock 4 v2      | Yes | Yes       |

## Build & Run

```
cargo build --release
target\release\gamepadcc.exe
```

No config file required — sensible defaults work out of the box.

## Button Mapping

### Always Active (Both Profiles)

| Button       | Action                          |
|--------------|---------------------------------|
| Cross        | Enter                           |
| Circle       | Escape                          |
| Triangle     | Tab                             |
| D-pad        | Arrow keys (with auto-repeat)   |
| Right stick  | Mouse scroll (vertical + horizontal) |
| PS           | Cycle profile (Default / Tmux)  |

### Default Profile

| Button  | Action            |
|---------|-------------------|
| Square  | New session       |
| L1      | Shift+Alt+Tab (previous window) |
| R1      | Alt+Tab (next window)           |
| R2      | Ctrl+C            |
| L3      | Ctrl+T            |
| R3      | Ctrl+P            |

### Tmux Profile

| Button  | Action                              |
|---------|-------------------------------------|
| Square  | tmux prefix + new-window            |
| L1      | tmux prefix + previous-window       |
| R1      | tmux prefix + next-window           |
| R2      | tmux prefix + kill-window (&)       |
| L3      | Ctrl+T                              |
| R3      | Ctrl+P                              |

Tmux bindings are auto-detected from the running tmux server via WSL. If auto-detection fails, standard tmux defaults are used. You can also override with direct key combos in the config.

## State-Driven Lightbar

An external tool (Claude Code or Codex) writes state files (`gamepadcc_agent_{session_id}`) to `%TEMP%`. The daemon polls these every 500ms and drives the lightbar accordingly.

| State     | Color         | Behavior      |
|-----------|---------------|---------------|
| `idle`    | Orange        | Solid         |
| `working` | Blue          | Pulsing       |
| `done`    | Green         | Solid         |
| `error`   | Red           | Solid         |

Multiple concurrent agent sessions are aggregated with priority: working > error > done > idle. Stale "working" agents (>10 min) are auto-pruned.

## System Tray

A tray icon shows the current profile with a visual indicator:

- **Default** — orange accent
- **Tmux** — green accent

The icon updates in real-time as you toggle profiles with the PS button.

## Claude Code Integration

Install hooks to connect Claude Code's lifecycle events to the lightbar:

```bash
bash install-hooks.sh
```

This copies `gamepadcc-state.sh` to `~/.claude/hooks/` and merges hook config into `~/.claude/settings.json`. Run from Git Bash (Windows) or WSL. Restart Claude Code after installing.

**Events hooked:** `UserPromptSubmit` (working), `Stop` (done), `PostToolUseFailure` (error).

## Codex Integration

The Codex hook bridge is **auto-deployed on daemon startup** — no manual setup required. When the daemon starts:

1. Checks if `~/.codex/` exists in WSL
2. Deploys embedded hook scripts to `~/.codex/hooks/`
3. Starts the Python bridge daemon in a tmux session

The bridge tails `~/.codex/sessions/**/*.jsonl` and maps Codex events to the same state-file system:

| Codex Event            | Mapped Hook            |
|------------------------|------------------------|
| `user_message`         | UserPromptSubmit       |
| `task_complete`        | Stop                   |
| `turn_aborted`         | Stop                   |
| `function_call_output` (non-zero exit) | PostToolUseFailure |

Lifecycle management:
```bash
~/.codex/hooks/start.sh    # Start bridge
~/.codex/hooks/stop.sh     # Stop bridge
~/.codex/hooks/status.sh   # Check if running
```

To disable auto-setup, add to your config:
```toml
[codex]
enabled = false
```

## OpenCode Integration

The OpenCode plugin is **auto-deployed on daemon startup** — no manual setup required. When the daemon starts:

1. Checks if OpenCode is installed in WSL (`~/.config/opencode/` or `opencode` command)
2. Deploys the native JS plugin to `~/.config/opencode/plugins/`

Unlike Codex (which needs a separate bridge daemon), OpenCode has a first-class plugin system. The plugin runs inside OpenCode itself and listens for session events in real-time:

| OpenCode Event              | Mapped Hook            |
|-----------------------------|------------------------|
| `session.status` → active   | UserPromptSubmit       |
| `session.status` → idle     | Stop                   |
| `session.status` → error    | PostToolUseFailure     |
| `tool.execute.before`        | UserPromptSubmit       |

To disable auto-setup, add to your config:
```toml
[opencode]
enabled = false
```

## Configuration

All settings are optional. Create `%APPDATA%\gamepadcc\config.toml` to override:

```toml
poll_interval_ms = 500
idle_timeout_s = 30
stale_timeout_s = 600

[scroll]
dead_zone = 20
sensitivity = 1.0
horizontal = true

[tmux]
enabled = true
auto_detect = true
prefix = "Ctrl+B"
l1 = "previous-window"
r1 = "next-window"
r2 = "kill-window"
square = "new-window"

[codex]
enabled = true

[opencode]
enabled = true

[lightbar.idle]
r = 255
g = 140
b = 0

[lightbar.working]
r = 0
g = 100
b = 255

[lightbar.done]
r = 0
g = 255
b = 0

[lightbar.error]
r = 255
g = 0
b = 0
```

## Architecture

```
main.rs              Startup, connection loop, input/output task orchestration
config.rs            TOML config with serde defaults
controller.rs        VID/PID detection, controller type + connection type enums
hid.rs               HID device discovery, open, read/write, BT extended mode
input.rs             Raw HID report parsing into UnifiedInput
mapper.rs            Button mapping, profiles, d-pad repeat, right-stick scroll
output.rs            Build HID output reports (lightbar + rumble)
lightbar.rs          State-to-color mapping with pulse animation
rumble.rs            Haptic patterns for state transitions
state.rs             Multi-agent state file polling and aggregation
tray.rs              System tray icon with profile indicator
tmux_detect.rs       Auto-detect tmux prefix + key bindings via WSL
wsl.rs               Shared WSL command execution utility
codex_setup.rs       Auto-deploy Codex hook scripts to WSL
opencode_setup.rs    Auto-deploy OpenCode plugin to WSL
```
