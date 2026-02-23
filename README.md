<p align="center">
  <img src="imgs/logo.png" alt="DS4CC" width="320">
</p>

<h1 align="center">DS4CC</h1>

<p align="center">
  DualSense / DS4 controller as a feedback device for Claude Code and Codex on Windows.
  <br>
  Lightbar, haptics, player LEDs, mic mute — all driven by your AI agent's state.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-2024_edition-f74c00?logo=rust&logoColor=white" alt="Rust">
</p>

---

## Quick Start

```bash
# 1. Download DS4CC-Setup.exe from Releases and install it
#    https://github.com/VeigaPunk/GamePadCCv2/releases

# 2. Install the Claude Code hook (WSL or Git Bash):
mkdir -p ~/.claude/hooks && cp hooks/ds4cc-state.sh ~/.claude/hooks/ && chmod +x ~/.claude/hooks/ds4cc-state.sh

# 3. Register the hook events — run from the repo root:
bash install-hooks.sh
```

Plug in your DualSense. Launch DS4CC. Open Claude Code.
The lightbar turns blue when the agent works, green when it's done.

---

## Features

| Feature | What it does |
|---|---|
| **Lightbar** | White (idle) → blue pulse (working) → green flash (done). Colors configurable. |
| **Profile switch** | PS button toggles Default ↔ Tmux. Button map and lightbar tint change instantly. |
| **Player LEDs** | 5-dot bar: Player 1 center dot (Default), Player 2 inner pair (Tmux). PS5-native presets. |
| **Mic mute** | Mute button toggles system microphone via Core Audio. LED lit = muted. |
| **Haptics** | Rumble on state transitions (working → done, etc.) |
| **Tray icon** | System tray with profile tooltip. Right-click to switch profile or quit. |
| **Multi-agent** | Aggregates concurrent Claude Code + Codex sessions. Priority: working > done > idle. |
| **Codex bridge** | Auto-deploys hook scripts to WSL on startup. Zero manual setup. |

## Supported Controllers

| Controller | USB | Bluetooth |
|---|:---:|:---:|
| DualSense | ✓ | ✓ |
| DualSense Edge | ✓ | ✓ |
| DualShock 4 v1 | ✓ | ✓ |
| DualShock 4 v2 | ✓ | ✓ |

## Requirements

- Windows 10 / 11
- DualSense or DualShock 4 controller (USB or Bluetooth)
- **Optional:** WSL2 — needed for Tmux profile and Codex integration
- **Optional:** [Claude Code](https://docs.anthropic.com/en/docs/claude-code) or [Codex](https://openai.com/index/codex/) for AI agent state feedback

## Install

### Installer (recommended)

Download **DS4CC-Setup.exe** from [Releases](https://github.com/VeigaPunk/GamePadCCv2/releases) and run it.

- Installs to `%LOCALAPPDATA%\DS4CC` — no admin rights needed
- Auto-start is **off by default** (opt-in checkbox)
- Optional desktop shortcut

### Manual

```
cargo build --release
target\release\ds4cc.exe
```

---

## Hook Setup

### Claude Code

Three commands, run from the repo root in WSL or Git Bash:

```bash
mkdir -p ~/.claude/hooks
cp hooks/ds4cc-state.sh ~/.claude/hooks/
chmod +x ~/.claude/hooks/ds4cc-state.sh
```

Then register the hook events in your Claude Code settings. The easiest way:

```bash
bash install-hooks.sh
```

This merges the hook config from `hooks/setup.json` into `~/.claude/settings.json`, registering three lifecycle events:

| Claude Code Event | DS4CC Action |
|---|---|
| `UserPromptSubmit` | Lightbar → **blue pulse** (working) |
| `Stop` | Lightbar → **green** (done) if task exceeded threshold, else idle |
| `PostToolUseFailure` | Logged as error — agent self-recovers silently |

> **Note:** "done" only fires if the task ran longer than a configurable threshold (default: 10 min). Short tasks go straight back to idle — no false green flashes.

Restart Claude Code after installing hooks.

### Codex

**Nothing to do.** DS4CC auto-deploys hook scripts to `~/.codex/hooks/` on startup via WSL. If WSL or Codex aren't installed, it skips silently.

Manual bridge management:

```bash
~/.codex/hooks/start.sh     # Start
~/.codex/hooks/stop.sh      # Stop
~/.codex/hooks/status.sh    # Check status
```

To disable Codex integration, add to your config:

```toml
[codex]
enabled = false
```

---

## Button Mapping

### Always Active

| Button | Action |
|---|---|
| Cross (×) | Enter |
| Circle (○) | Escape |
| Triangle (△) | Tab |
| D-pad | Arrow keys (auto-repeat) |
| Right stick | Scroll (vertical + horizontal) |
| PS | Toggle profile (Default ↔ Tmux) |
| Mute | Toggle system microphone |

### Default Profile

| Button | Action |
|---|---|
| Square (□) | New session |
| L1 | Previous window (Shift+Alt+Tab) |
| R1 | Next window (Alt+Tab) |
| R2 | Ctrl+C |
| L3 | Ctrl+T |
| R3 | Ctrl+P |

### Tmux Profile

| Button | Action |
|---|---|
| Square (□) | tmux: new-window |
| L1 | tmux: previous-window |
| R1 | tmux: next-window |
| R2 | tmux: kill-window |
| L3 | Ctrl+T |
| R3 | Ctrl+P |

Tmux bindings are auto-detected from the running tmux server via WSL. Falls back to standard defaults if detection fails. Override in config if needed.

---

## Configuration

All settings are optional. Sensible defaults work out of the box.

Config file: `%APPDATA%\ds4cc\config.toml`

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

[codex]
enabled = true

# Lightbar colors (RGB) — customize per state
[lightbar.idle]
r = 255
g = 255
b = 255

[lightbar.working]
r = 0
g = 100
b = 255

[lightbar.done]
r = 0
g = 255
b = 0
```

Environment variables for the hook script:

| Variable | Default | Description |
|---|---|---|
| `DS4CC_DONE_THRESHOLD_S` | `600` | Minimum task duration (seconds) before "done" fires |
| `DS4CC_STALE_WORKING_S` | `900` | Seconds before a stuck "working" state is pruned |
| `DS4CC_STATE_DIR` | `%TEMP%` | Directory for agent state files |

---

## Build from Source

```bash
git clone https://github.com/VeigaPunk/GamePadCCv2.git
cd GamePadCCv2
cargo build --release
```

Binary: `target\release\ds4cc.exe`

To build the installer, open `installer/ds4cc.iss` in [Inno Setup](https://jrsoftware.org/isinfo.php) and compile.

## Architecture

```
main.rs            Startup, connection loop, input/output orchestration
config.rs          TOML config with serde defaults
controller.rs      VID/PID detection, controller type enums
hid.rs             HID device discovery, open, read/write
input.rs           Raw HID report parsing → UnifiedInput
mapper.rs          Button mapping, profiles, d-pad repeat, right-stick scroll
output.rs          HID output reports (lightbar + rumble + player LEDs + mic LED)
lightbar.rs        State → RGB color with pulse animation
rumble.rs          Haptic patterns for state transitions
state.rs           Multi-agent state file polling and aggregation
mic.rs             System microphone toggle via Core Audio COM
tray.rs            System tray icon with profile indicator
tmux_detect.rs     Auto-detect tmux prefix + key bindings via WSL
codex_setup.rs     Auto-deploy Codex hook scripts to WSL
wsl.rs             Shared WSL command execution utility
```

## License

MIT
