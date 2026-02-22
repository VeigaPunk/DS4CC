# GamePadCC v2

Windows daemon that bridges DualSense/DS4 controllers with CLI tools.

## How It Works

### Build & Run

```
cargo run --release
```

Compiles to `target/release/gamepadcc.exe`. You can also run the `.exe` directly.

### Startup Sequence

1. **Logger init** — `env_logger` starts, defaults to `info` level
2. **Config load** — Reads `%APPDATA%\gamepadcc\config.toml`. If the file doesn't exist, defaults kick in (no config file needed)
3. **HID API init** — Enumerates USB/BT devices
4. **State file poller spawns** — A tokio task polls `%TEMP%\gamepadcc_state` every 500ms, parsing its contents as `idle`/`working`/`done`/`error` and broadcasting changes via a `watch` channel
5. **Connection loop** (runs forever):
   - Scans for a DualSense/DS4 by VID/PID + usage page filtering
   - If not found, retries every 2 seconds
   - Once found, opens the device in non-blocking mode
   - If Bluetooth, activates extended mode via feature report
   - Spawns an **output task** (lightbar + rumble at ~30fps)
   - Runs the **input loop** on the main task (reads HID reports, parses buttons, fires keystrokes)
   - When the device disconnects, the input loop returns, the output task is aborted, and it loops back to scanning

### State-Driven Lightbar

An external tool (like Claude Code) writes a string (`idle`, `working`, `done`, `error`) to `%TEMP%\gamepadcc_state`. The poller picks it up, the output loop maps it to an RGB color, and sends it to the controller every 33ms.

| State     | Color         |
|-----------|---------------|
| `idle`    | Orange        |
| `working` | Blue (pulse)  |
| `done`    | Green         |
| `error`   | Red           |

### Button-to-Keystroke Mapping

Each HID report is parsed into a `ButtonState`. The mapper does rising-edge detection (pressed this frame but not last frame) and calls `SendInput` with the configured key combo.

| Button   | Default Action   |
|----------|------------------|
| Cross    | Enter            |
| Circle   | Escape           |
| Square   | new_session      |
| Triangle | Tab              |
| L1       | Shift+Alt+Tab    |
| R1       | Alt+Tab          |
| D-pad    | Arrow keys       |

### Supported Controllers

- DualSense (USB + Bluetooth)
- DualSense Edge (USB + Bluetooth)
- DualShock 4 v1 (USB + Bluetooth)
- DualShock 4 v2 (USB + Bluetooth)

### Configuration

All settings are optional. Create `%APPDATA%\gamepadcc\config.toml` to override defaults:

```toml
poll_interval_ms = 500

[lightbar.idle]
r = 255
g = 140
b = 0

[buttons]
cross = "Enter"
circle = "Escape"
```

No setup required beyond plugging in the controller.
