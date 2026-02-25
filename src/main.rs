mod codex_poll;
mod config;
mod controller;
mod crc32;
mod hid;
mod input;
mod lightbar;
mod mapper;
mod mic;
mod opencode_detect;
mod output;
mod rumble;
mod setup;
mod state;
mod tmux_detect;
mod tray;
mod wsl;

use crate::controller::ConnectionType;
use crate::output::OutputState;
use crate::state::AgentState;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    log::info!("DS4CC v2 starting...");

    let cfg = config::Config::load();
    log::info!("State dir: {}", cfg.state_dir);

    // Auto-install Claude Code hooks + OpenCode plugin (first run / after update).
    // Runs in background — startup is not blocked.  Subsequent runs are instant
    // (version stamp check) so there is no recurring overhead.
    tokio::spawn(async {
        if let Some(result) = tokio::task::spawn_blocking(setup::run).await.unwrap_or(None) {
            let mut installed = Vec::new();
            if result.claude_code { installed.push("Claude Code hook"); }
            if result.opencode    { installed.push("OpenCode plugin"); }
            if !installed.is_empty() {
                log::info!("Hooks installed: {}. Restart your AI tools to activate.", installed.join(", "));
            }
        }
    });

    // Auto-detect tmux configuration (prefix + key bindings) via WSL
    let tmux_detected = if cfg.tmux.auto_detect && cfg.tmux.enabled {
        tmux_detect::detect()
    } else {
        None
    };

    // Auto-detect OpenCode keybinds from ~/.config/opencode/opencode.json via WSL
    let opencode_detected = if cfg.opencode.auto_detect && cfg.opencode.enabled {
        opencode_detect::detect()
    } else {
        None
    };

    // Spawn native Codex JSONL poller (reads session files via WSL UNC path)
    if cfg.codex.enabled {
        let state_dir = PathBuf::from(&cfg.state_dir);
        let done_threshold_s = cfg.codex.done_threshold_s;
        let poll_ms = cfg.poll_interval_ms;
        tokio::spawn(async move {
            // Resolve the WSL sessions path (blocking I/O)
            let sessions_dir = tokio::task::spawn_blocking(codex_poll::resolve_sessions_dir)
                .await
                .ok()
                .flatten();
            if let Some(dir) = sessions_dir {
                codex_poll::run(dir, state_dir, done_threshold_s, poll_ms).await;
            }
        });
    }

    // Shared mouse mode toggle: false = touchpad, true = left stick.
    // Owned here; cloned into tray thread and each input loop iteration.
    let mouse_stick_active = Arc::new(AtomicBool::new(false));

    // Tray icon
    let tray_tx = tray::spawn(mapper::Profile::Default, Arc::clone(&mouse_stick_active));

    // Initialize HID
    let mut api = match hidapi::HidApi::new() {
        Ok(api) => api,
        Err(e) => {
            log::error!("Failed to initialize HID API: {e}");
            std::process::exit(1);
        }
    };

    // State channel (persists across reconnections)
    let (state_tx, state_rx) = watch::channel(AgentState::Idle);
    // Per-agent rumble channels (Arc<Mutex> so they survive reconnections)
    let (idle_reminder_tx, idle_reminder_rx) = mpsc::channel::<()>(4);
    let (done_rumble_tx, done_rumble_rx) = mpsc::channel::<()>(4);
    let idle_reminder_rx = Arc::new(tokio::sync::Mutex::new(idle_reminder_rx));
    let done_rumble_rx = Arc::new(tokio::sync::Mutex::new(done_rumble_rx));

    // Spawn state poller (scans ds4cc_agent_* files in state_dir)
    let state_dir = PathBuf::from(&cfg.state_dir);
    let poll_ms = cfg.poll_interval_ms;
    let idle_timeout_s = cfg.idle_timeout_s;
    let stale_timeout_s = cfg.stale_timeout_s;
    let idle_reminder_s = cfg.idle_reminder_s;
    tokio::spawn(async move {
        state::poll_state_file(state_dir, poll_ms, idle_timeout_s, stale_timeout_s, idle_reminder_s, WORKING_DONE_MIN_MS, state_tx, idle_reminder_tx, done_rumble_tx).await;
    });

    // Main connection loop — reconnects on disconnect
    loop {
        // Find controller
        let (info, device) = loop {
            if let Err(e) = api.refresh_devices() {
                log::debug!("HID refresh failed: {e}");
            }
            match hid::find_controller(&api) {
                Some(info) => match hid::open_device(&api, &info) {
                    Ok(dev) => break (info, dev),
                    Err(e) => {
                        log::warn!("Found controller but failed to open: {e}");
                    }
                },
                None => {
                    log::info!("No controller found. Retrying in 2s...");
                }
            }
            sleep(Duration::from_secs(2)).await;
        };

        log::info!(
            "Connected: {} ({})",
            info.controller_type,
            info.connection_type
        );

        // Activate BT extended mode if needed
        if info.connection_type == ConnectionType::Bluetooth {
            if let Err(e) = hid::activate_bt_extended_mode(&device, info.controller_type) {
                log::error!("Failed to activate BT extended mode: {e}");
                log::error!("Controller may not work correctly over Bluetooth.");
            }
        }

        let handle = hid::HidHandle::new(device);
        let ct = info.controller_type;
        let conn = info.connection_type;

        // Shared player indicator LED state (AtomicU8 so both loops can read/write it).
        // Start at Player 1 (Default profile) on every connection.
        let player_leds = Arc::new(AtomicU8::new(PLAYER1_LEDS));

        // Spawn output loop for this connection
        let output_handle = handle.clone_handle();
        let lightbar_cfg_clone = cfg.lightbar.clone();
        let mut state_rx_output = state_rx.clone();
        let player_leds_out = Arc::clone(&player_leds);
        let idle_rx = Arc::clone(&idle_reminder_rx);
        let done_rx = Arc::clone(&done_rumble_rx);
        let output_task = tokio::spawn(async move {
            run_output_loop(output_handle, ct, conn, lightbar_cfg_clone, &mut state_rx_output, player_leds_out, idle_rx, done_rx).await;
        });

        // Run input loop — returns when device disconnects
        run_input_loop(handle, ct, conn, &cfg.scroll, &cfg.stick_mouse, &cfg.touchpad, &cfg.tmux, tmux_detected.as_ref(), &cfg.opencode, opencode_detected.as_ref(), &tray_tx, Arc::clone(&player_leds), Arc::clone(&mouse_stick_active)).await;

        // Device disconnected — cancel output task and reconnect
        output_task.abort();
        log::info!("Controller disconnected. Scanning for new connection...");
        sleep(Duration::from_secs(1)).await;
    }
}

/// Input loop: read HID reports, parse, map to keystrokes.
/// Returns when the device disconnects.
async fn run_input_loop(
    handle: hid::HidHandle,
    ct: controller::ControllerType,
    conn: controller::ConnectionType,
    scroll_cfg: &config::ScrollConfig,
    stick_mouse_cfg: &config::StickMouseConfig,
    touchpad_cfg: &config::TouchpadConfig,
    tmux_cfg: &config::TmuxConfig,
    tmux_detected: Option<&tmux_detect::TmuxDetected>,
    opencode_cfg: &config::OpenCodeConfig,
    opencode_detected: Option<&opencode_detect::OpenCodeDetected>,
    tray_tx: &std::sync::mpsc::Sender<tray::TrayCmd>,
    player_leds: Arc<AtomicU8>,
    mouse_stick_active: Arc<AtomicBool>,
) {
    let mut mapper_state = mapper::MapperState::new(
        scroll_cfg,
        stick_mouse_cfg,
        touchpad_cfg,
        tmux_cfg,
        tmux_detected,
        opencode_cfg,
        opencode_detected,
        mouse_stick_active,
    );
    let mut buf = [0u8; 128];
    let mut consecutive_errors = 0u32;
    let mut first_report = true;
    let mut last_profile = mapper_state.profile();
    let mut last_mute = false;

    loop {
        match handle.read(&mut buf) {
            Err(()) => {
                // Device disconnected
                return;
            }
            Ok(0) => {
                // No data available — yield and retry
                sleep(Duration::from_millis(4)).await;
                consecutive_errors = 0;
                continue;
            }
            Ok(n) => {
                let data = &buf[..n];

                if first_report {
                    let hex: Vec<String> = data.iter().take(16).map(|b| format!("{b:02X}")).collect();
                    log::info!("First report ({n} bytes): {}", hex.join(" "));
                    first_report = false;
                }

                // Validate CRC on Bluetooth
                if conn == ConnectionType::Bluetooth && !input::validate_bt_crc(ct, data) {
                    consecutive_errors += 1;
                    if consecutive_errors % 100 == 1 {
                        log::warn!("BT CRC validation failed ({consecutive_errors} times)");
                    }
                    continue;
                }

                match input::parse(ct, conn, data) {
                    Ok(unified) => {
                        consecutive_errors = 0;
                        let actions = mapper_state.update(&unified);
                        for action in &actions {
                            #[cfg(windows)]
                            mapper::execute_action(action);
                            log::debug!("Action: {action:?}");
                        }

                        // Mute button — toggle system mic on press (any profile)
                        let mute_now = unified.buttons.mute;
                        if mute_now && !last_mute {
                            tokio::task::spawn_blocking(mic::toggle_mute);
                        }
                        last_mute = mute_now;

                        // Update tray icon and player LED on profile change
                        let current_profile = mapper_state.profile();
                        if current_profile != last_profile {
                            let _ = tray_tx.send(tray::TrayCmd::SetProfile(current_profile));
                            last_profile = current_profile;

                            // Instantly show the new profile's player indicator LED.
                            let target_leds = match current_profile {
                                mapper::Profile::Default => PLAYER1_LEDS,
                                mapper::Profile::Tmux    => PLAYER2_LEDS,
                            };
                            player_leds.store(target_leds, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        consecutive_errors += 1;
                        if consecutive_errors % 100 == 1 {
                            log::warn!("Input parse error ({consecutive_errors}): {e}");
                        }
                    }
                }
            }
        }
    }
}

/// Minimum working duration before the Working → Done rumble fires.
/// Short tasks don't warrant a notification; only surface it for real work.
const WORKING_DONE_MIN_MS: u64 = 10 * 60 * 1000; // 10 minutes

/// Player indicator LED presets — mimics PS5 native player assignment.
///   Player 1 (Default profile) → center dot only
///   Player 2 (Tmux profile)    → inner two dots (center-left + center-right)
const PLAYER1_LEDS: u8 = 0x04; // center only
const PLAYER2_LEDS: u8 = 0x0A; // inner two (0x02 | 0x08)

/// Output loop: update lightbar based on aggregated state, fire rumble from per-agent signals.
async fn run_output_loop(
    handle: hid::HidHandle,
    ct: controller::ControllerType,
    conn: controller::ConnectionType,
    lightbar_cfg: config::LightbarConfig,
    state_rx: &mut watch::Receiver<AgentState>,
    player_leds: Arc<AtomicU8>,
    idle_reminder_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<()>>>,
    done_rumble_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<()>>>,
) {
    let mut bt_seq = 0u8;
    let mut current_state = AgentState::Idle;
    let mut state_start = Instant::now();

    // Prime mic mute state from system before first frame
    tokio::task::spawn_blocking(mic::init).await.ok();

    // Set initial lightbar + Player 1 indicator (Default profile on startup)
    send_output(
        &handle,
        ct,
        conn,
        &lightbar_cfg,
        current_state,
        0,
        PLAYER1_LEDS,
        &mut bt_seq,
    );

    let mut ticker = tokio::time::interval(Duration::from_millis(33)); // ~30fps for smooth pulse
    let mut idle_rx = idle_reminder_rx.lock().await;
    let mut done_rx = done_rumble_rx.lock().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let elapsed = state_start.elapsed().as_millis() as u64;
                let leds = player_leds.load(Ordering::Relaxed);
                send_output(&handle, ct, conn, &lightbar_cfg, current_state, elapsed, leds, &mut bt_seq);
            }
            _ = idle_rx.recv() => {
                // Per-agent idle reminder — fire rumble
                log::info!("Per-agent idle reminder rumble triggered");
                fire_rumble(&handle, ct, conn, &rumble::idle_reminder_pattern());
            }
            _ = done_rx.recv() => {
                // Per-agent Working → Done — fire celebratory rumble
                log::info!("Per-agent done rumble triggered");
                if let Some(pattern) = rumble::pattern_for_transition(AgentState::Working, AgentState::Done) {
                    fire_rumble(&handle, ct, conn, &pattern);
                }
            }
            result = state_rx.changed() => {
                if result.is_err() {
                    log::error!("State channel closed");
                    break;
                }
                let new_state = *state_rx.borrow();
                if new_state != current_state {
                    log::debug!("Lightbar transition {:?} → {:?}", current_state, new_state);
                    current_state = new_state;
                    state_start = Instant::now();
                }
            }
        }
    }
}

/// Spawn a rumble pattern on the controller (non-blocking).
fn fire_rumble(
    handle: &hid::HidHandle,
    ct: controller::ControllerType,
    conn: controller::ConnectionType,
    pattern: &[rumble::RumbleStep],
) {
    let rumble_handle = handle.clone_handle();
    let pattern = pattern.to_vec();
    tokio::spawn(async move {
        let mut seq = 0u8;
        rumble::play_pattern(&pattern, |left, right| {
            let out = OutputState {
                rumble_left: left,
                rumble_right: right,
                ..Default::default()
            };
            let report = output::build_report(ct, conn, &out, &mut seq);
            rumble_handle.write(&report);
        }).await;
    });
}

fn send_output(
    handle: &hid::HidHandle,
    ct: controller::ControllerType,
    conn: controller::ConnectionType,
    lightbar_cfg: &config::LightbarConfig,
    state: AgentState,
    elapsed_ms: u64,
    player_leds: u8,
    bt_seq: &mut u8,
) {
    let (r, g, b) = lightbar::compute_color(lightbar_cfg, state, elapsed_ms);
    let out = OutputState {
        lightbar_r: r,
        lightbar_g: g,
        lightbar_b: b,
        rumble_left: 0,
        rumble_right: 0,
        player_leds,
        mute_led: mic::MIC_MUTED.load(std::sync::atomic::Ordering::Relaxed) as u8,
    };
    let report = output::build_report(ct, conn, &out, bt_seq);
    handle.write(&report);
}
