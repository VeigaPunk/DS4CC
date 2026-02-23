<p align="center">
  <img src="imgs/logo.png" alt="DS4CC" width="320">
</p>

<h1 align="center">DS4CC</h1>

<p align="center">
  Turn a PlayStation controller into a programmable dev companion with AI awareness.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-2024_edition-f74c00?logo=rust&logoColor=white" alt="Rust">
</p>

---

## Mission

Turn a controller into a programmable command interface for developers.

Make invisible processes tactile. Make AI state observable. Reduce keyboard friction. Keep the system simple.

---

## What This Is

DS4CC is a small Rust program that runs in the background and lets your PlayStation controller:

- **Control tmux** — switch panes, split windows, navigate sessions
- **React to Claude Code / Codex activity** — lightbar changes when the AI is thinking
- **Give you rumble + lightbar feedback** when things happen
- **Act like a programmable dev companion** — buttons map to real keystrokes
- **Pair with [Wispr](https://wisprflow.ai/) for a keyboard-free workflow** — voice handles text, controller handles everything else

---

## Quick Start

```bash
# 1. Download DS4CC-Setup.exe from Releases and install it
#    https://github.com/VeigaPunk/DS4CC/releases

# 2. Install Claude Code hooks (WSL or Git Bash):
git clone https://github.com/VeigaPunk/DS4CC.git && cd DS4CC
bash install-hooks.sh
```

Plug in your DualSense. Launch DS4CC. Open Claude Code or Codex.

The lightbar reflects the real-time status of your AI agents — across all sessions, on both Windows and WSL CLIs. Rumble kicks in when a long-running task completes or an agent has been idle for a while, so you never miss the moment.

Colors, thresholds, and behavior are all customizable via `%APPDATA%\ds4cc\config.toml`.

---

## How It Actually Works

Here's the real flow, no buzzwords:

1. You launch `ds4cc.exe`
2. It loads your config (`%APPDATA%\ds4cc\config.toml`, or defaults)
3. It starts a tray icon
4. It starts watching agent state files in `%TEMP%`
5. It starts polling Codex JSONL session logs via `\\wsl.localhost\` (if available)
6. It connects to your controller via HID
7. It enters two loops:
   - **Input** — read buttons → send keystrokes, toggle profiles (shortcut mapping)
   - **Output** — conditional agent states → lightbar color | rumble

---

## Core Features

### 🎮 Controller → Keystrokes

Press buttons → things happen. D-pad sends arrow keys. Right stick scrolls. Face buttons map to Enter, Escape, Tab.

Two profiles: **Default** and **Tmux**, toggled with the PS button. Both are fully customizable — just ask Claude to change the mappings in the source and rebuild. Want a different button for Ctrl+C? Different tmux bindings? Change it per profile.

#### Always Active

| Button | Action |
|---|---|
| Cross (×) | Enter |
| Circle (○) | Escape |
| Triangle (△) | Tab |
| D-pad | Arrow keys |
| Right stick | Scroll (vertical — replaces mouse scroll wheel) |
| L2 | Wispr speech-to-text (hold to dictate) |
| PS | Toggle profile (Default ↔ Tmux) |
| Mute | Toggle system microphone |

#### Default Profile

| Button | Action |
|---|---|
| Square (□) | New session |
| L1 | Previous window (Shift+Alt+Tab) |
| R1 | Next window (Alt+Tab) |
| R2 | Ctrl+C |
| L3 | Ctrl+T |
| R3 | Ctrl+P |

#### Tmux Profile

| Button | Action |
|---|---|
| Square (□) | tmux: new-window |
| L1 | tmux: previous-window |
| R1 | tmux: next-window |
| R2 | tmux: kill-window |
| L3 | Ctrl+T |
| R3 | Ctrl+P |

Tmux bindings are auto-detected from the running tmux server via WSL. Falls back to standard defaults if detection fails. Override in config if needed.

### 🎙️ Controller + Wispr = No Keyboard

DS4CC was designed to pair with [Wispr Flow](https://wisprflow.ai/) — a voice-to-text tool that lets you dictate code, commands, and prompts.

The idea is simple:

- **Wispr** handles all text input — you talk, it types
- **DS4CC** handles everything else — navigation, window switching, scrolling, Enter/Escape/Tab, tmux control

Together they replace the keyboard entirely. You lean back, hold the controller, talk to your AI agent, and watch the lightbar pulse while it works. When it's done, you feel the rumble.

This is the workflow DS4CC was built for: voice + gamepad + AI agents. No keyboard required.

### 🤖 Controller → AI Awareness

DS4CC monitors Claude Code and Codex by watching state files. When the AI is:

- **Working** → lightbar pulses blue
- **Done** → lightbar flashes green, rumble kicks
- **Error** → silently recovers (no visual noise)
- **Idle** → default color

**Claude Code** — shell hooks in `~/.claude/hooks/` write per-session state files to `%TEMP%` on lifecycle events.

**Codex** — the daemon polls Codex JSONL session logs directly via `\\wsl.localhost\` UNC paths. No hooks, no bridge scripts, no external processes. It tail-follows the JSONL files, parses events (`user_message`, `task_complete`, etc.), and writes the same state files.

State files (`ds4cc_agent_<session_id>`) land in `%TEMP%`. The daemon polls them every 500ms and aggregates across all sessions — priority: **working > done > idle**.

Each agent is tracked individually:

- **Done rumble** — when any agent finishes a task that took >= 10 minutes, the controller rumbles. Short tasks go straight back to idle without notification.
- **Idle reminder** — when any agent sits idle for 8 minutes, an attention rumble fires — even if other agents are still working.
- **"Done" threshold** — short tasks (< 10 min by default) write "idle" instead of "done" at the hook level. Only real work triggers the green flash.

### 🔔 Feedback System

Your controller becomes a status light.

- **Lightbar** — color reflects agent state. Pulsing blue = thinking. Green = done. Configurable RGB.
- **Rumble** — haptic patterns on state transitions. You feel when the AI finishes.
- **Profile LEDs** — Profile 1 = 1 White LED on controller, Profile 2 = 2 White LEDs.
- **Mic mute** — mute button toggles system microphone via Core Audio. LED lit = muted. Works on any profile.

The output loop runs every ~33ms to keep LEDs smooth. Rumble is async but shares the HID device safely.

### 🧠 Agent State Model

The agent can be in one of four states:

```
Working > Error > Done > Idle
```

Priority logic: if any session is working, the controller shows working. "Done" automatically becomes idle after a configurable timeout. Stale "working" states (crashed sessions) get cleaned up after 15 minutes.

This prevents zombie states.

### 🖥️ Tray Icon

PS button switches profile (shortcut mappings). System tray icon shows current profile. Right-click to start Wispr, enable auto-start-up, restart or exit app. Tooltip shows `DS4CC — Default` or `DS4CC — Tmux`.

---

## Supported Controllers

| Controller | USB | Bluetooth |
|---|:---:|:---:|
| DualSense | ✓ | ✓ |
| DualShock 4 | WIP | WIP |

Bluetooth supports all features except Microphone Input (WIP).

## Requirements

- Windows 10 / 11
- DualSense controller (USB or Bluetooth)
- **Optional:** WSL2 — needed for Tmux profile and Codex integration
- **Optional:** [Claude Code](https://docs.anthropic.com/en/docs/claude-code) or [Codex](https://openai.com/index/codex/) for AI agent state feedback

---

## Install

### Installer (recommended)

Download **DS4CC-Setup.exe** from [Releases](https://github.com/VeigaPunk/DS4CC/releases) and run it.

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

Run from the repo root in WSL or Git Bash:

```bash
bash install-hooks.sh
```

This copies the hook script to `~/.claude/hooks/`, strips CRLF line endings, and merges the hook config into `~/.claude/settings.json`, registering three lifecycle events:

| Claude Code Event | What happens |
|---|---|
| `UserPromptSubmit` | Lightbar → blue pulse (working) |
| `Stop` | Lightbar → green (done) if task exceeded threshold, else idle |
| `PostToolUseFailure` | Logged as error — agent self-recovers silently |

Restart Claude Code after installing hooks.

### Codex

**Nothing to do.** DS4CC natively polls Codex JSONL session logs via `\\wsl.localhost\` UNC paths. No hooks, no bridge scripts, no external processes to manage. If WSL or Codex aren't installed, it skips silently.

To disable:

```toml
[codex]
enabled = false
```

---

## Configuration

All settings are optional. Sensible defaults work out of the box.

Config file: `%APPDATA%\ds4cc\config.toml`

```toml
poll_interval_ms = 500
idle_timeout_s = 30
stale_timeout_s = 600
idle_reminder_s = 480     # per-agent idle rumble (8 min, 0 = disabled)

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
done_threshold_s = 600    # seconds before "done" fires (vs. straight to idle)

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

Environment variables for the Claude Code hook script:

| Variable | Default | Description |
|---|---|---|
| `DS4CC_DONE_THRESHOLD_S` | `600` | Minimum task duration (seconds) before "done" fires |
| `DS4CC_STALE_WORKING_S` | `900` | Seconds before a stuck "working" state is pruned |
| `DS4CC_STATE_DIR` | `%TEMP%` | Directory for agent state files |

For Codex, the done threshold is configured in `config.toml` under `[codex] done_threshold_s`.

---

## Technical Notes

- Written in Rust (2024 edition)
- Uses HID directly via `hidapi`
- Async runtime: `tokio` with multi-threaded scheduler
- Input read timeout: 5ms
- Output write interval: ~33ms
- State polling interval: 500ms
- Mic mute: Windows Core Audio COM API (`IAudioEndpointVolume`)
- System tray: `tray-icon` crate
- Config: TOML with `serde` defaults

## Build from Source

```bash
git clone https://github.com/VeigaPunk/DS4CC.git
cd DS4CC
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
codex_poll.rs      Native Codex JSONL session poller via UNC paths
wsl.rs             Shared WSL command execution utility
```

---

## Why It Exists

When you run multiple AI agents, they can become hard to oversee. You might not know if they're working, idle or done.

DS4CC turns that invisible state into:

- **Light**
- **Color**
- **Vibration**

You feel when the AI finishes.

Pair it with [Wispr](https://wisprflow.ai/) and you don't even need a keyboard. Voice dictates. Controller navigates. The lightbar tells you what the AI is doing. You lean back and ship code from the couch.

*This is the way.*

## License

MIT
