mod codex_setup;
mod config;
mod controller;
mod crc32;
mod hid;
mod input;
mod lightbar;
mod mapper;
mod mic;
mod output;
mod rumble;
mod state;
mod tmux_detect;
mod tray;
mod wsl;

use crate::controller::ConnectionType;
use crate::output::OutputState;
use crate::state::AgentState;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;
use tokio::sync::watch;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    log::info!("GamePadCC v2 starting...");

    let cfg = config::Config::load();
    log::info!("State dir: {}", cfg.state_dir);

    // Auto-detect tmux configuration (prefix + key bindings) via WSL
    let tmux_detected = if cfg.tmux.auto_detect && cfg.tmux.enabled {
        tmux_detect::detect()
    } else {
        None
    };

    // Auto-setup Codex hook bridge via WSL
    if cfg.codex.enabled {
        codex_setup::setup();
    }

    // Tray icon
    let tray_tx = tray::spawn(mapper::Profile::Default);

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

    // Spawn state poller (scans gamepadcc_agent_* files in state_dir)
    let state_dir = PathBuf::from(&cfg.state_dir);
    let poll_ms = cfg.poll_interval_ms;
    let idle_timeout_s = cfg.idle_timeout_s;
    let stale_timeout_s = cfg.stale_timeout_s;
    tokio::spawn(async move {
        state::poll_state_file(state_dir, poll_ms, idle_timeout_s, stale_timeout_s, state_tx).await;
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
        let output_task = tokio::spawn(async move {
            run_output_loop(output_handle, ct, conn, lightbar_cfg_clone, &mut state_rx_output, player_leds_out).await;
        });

        // Run input loop — returns when device disconnects
        run_input_loop(handle, ct, conn, &cfg.scroll, &cfg.tmux, tmux_detected.as_ref(), &tray_tx, Arc::clone(&player_leds)).await;

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
    tmux_cfg: &config::TmuxConfig,
    tmux_detected: Option<&tmux_detect::TmuxDetected>,
    tray_tx: &std::sync::mpsc::Sender<tray::TrayCmd>,
    player_leds: Arc<AtomicU8>,
) {
    let mut mapper_state = mapper::MapperState::new(scroll_cfg, tmux_cfg, tmux_detected);
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

/// How long the agent must stay idle before the attention rumble fires.
const IDLE_REMINDER_MS: u64 = 3 * 60 * 1000; // 3 minutes

/// Minimum working duration before the Working → Done rumble fires.
/// Short tasks don't warrant a notification; only surface it for real work.
const WORKING_DONE_MIN_MS: u64 = 5 * 60 * 1000; // 5 minutes

/// Player indicator LED presets — mimics PS5 native player assignment.
///   Player 1 (Default profile) → center dot only
///   Player 2 (Tmux profile)    → inner two dots (center-left + center-right)
const PLAYER1_LEDS: u8 = 0x04; // center only
const PLAYER2_LEDS: u8 = 0x0A; // inner two (0x02 | 0x08)

/// Output loop: update lightbar and rumble based on state changes.
async fn run_output_loop(
    handle: hid::HidHandle,
    ct: controller::ControllerType,
    conn: controller::ConnectionType,
    lightbar_cfg: config::LightbarConfig,
    state_rx: &mut watch::Receiver<AgentState>,
    player_leds: Arc<AtomicU8>,
) {
    let mut bt_seq = 0u8;
    let mut current_state = AgentState::Idle;
    let state_entered_at = Instant::now();
    let mut state_start = state_entered_at;
    let mut idle_rumble_fired = false; // true once the 3-min reminder has fired for this idle stretch

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

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let elapsed = state_start.elapsed().as_millis() as u64;
                let leds = player_leds.load(Ordering::Relaxed);
                send_output(&handle, ct, conn, &lightbar_cfg, current_state, elapsed, leds, &mut bt_seq);

                // Idle attention reminder: fire once after 3 minutes in idle
                if current_state == AgentState::Idle
                    && !idle_rumble_fired
                    && elapsed >= IDLE_REMINDER_MS
                {
                    idle_rumble_fired = true;
                    log::info!("Idle reminder rumble triggered ({}min)", elapsed / 60_000);
                    let rumble_handle = handle.clone_handle();
                    let rumble_ct = ct;
                    let rumble_conn = conn;
                    tokio::spawn(async move {
                        let mut seq = 0u8;
                        rumble::play_pattern(&rumble::idle_reminder_pattern(), |left, right| {
                            let out = OutputState {
                                rumble_left: left,
                                rumble_right: right,
                                ..Default::default()
                            };
                            let report = output::build_report(rumble_ct, rumble_conn, &out, &mut seq);
                            rumble_handle.write(&report);
                        }).await;
                    });
                }
            }
            result = state_rx.changed() => {
                if result.is_err() {
                    log::error!("State channel closed");
                    break;
                }
                let new_state = *state_rx.borrow();
                if new_state != current_state {
                    let old_state = current_state;
                    let elapsed_in_old = state_start.elapsed().as_millis() as u64;
                    current_state = new_state;
                    state_start = Instant::now();
                    idle_rumble_fired = false; // reset for new idle stretch

                    // Working → Done only rumbles if the task ran long enough to be meaningful.
                    // All other transitions fire unconditionally.
                    let long_enough = !(old_state == AgentState::Working
                        && new_state == AgentState::Done
                        && elapsed_in_old < WORKING_DONE_MIN_MS);

                    if long_enough {
                        log::debug!(
                            "Transition {:?} → {:?} after {}s",
                            old_state, new_state, elapsed_in_old / 1000
                        );
                    } else {
                        log::debug!(
                            "Working → Done after {}s (< {}s threshold) — skipping rumble",
                            elapsed_in_old / 1000,
                            WORKING_DONE_MIN_MS / 1000
                        );
                    }

                    // Fire rumble pattern if applicable
                    if long_enough {
                        if let Some(pattern) = rumble::pattern_for_transition(old_state, new_state) {
                            let rumble_handle = handle.clone_handle();
                            let rumble_ct = ct;
                            let rumble_conn = conn;
                            tokio::spawn(async move {
                                let mut seq = 0u8;
                                rumble::play_pattern(&pattern, |left, right| {
                                    let out = OutputState {
                                        rumble_left: left,
                                        rumble_right: right,
                                        ..Default::default()
                                    };
                                    let report = output::build_report(rumble_ct, rumble_conn, &out, &mut seq);
                                    rumble_handle.write(&report);
                                }).await;
                            });
                        }
                    }
                }
            }
        }
    }
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
