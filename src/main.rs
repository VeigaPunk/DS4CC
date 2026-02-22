mod config;
mod controller;
mod crc32;
mod hid;
mod input;
mod lightbar;
mod mapper;
mod output;
mod rumble;
mod state;
mod tmux_detect;
mod tray;

use crate::controller::ConnectionType;
use crate::output::OutputState;
use crate::state::AgentState;

use std::path::PathBuf;
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

        // Spawn output loop for this connection
        let output_handle = handle.clone_handle();
        let lightbar_cfg_clone = cfg.lightbar.clone();
        let mut state_rx_output = state_rx.clone();
        let output_task = tokio::spawn(async move {
            run_output_loop(output_handle, ct, conn, lightbar_cfg_clone, &mut state_rx_output).await;
        });

        // Run input loop — returns when device disconnects
        run_input_loop(handle, ct, conn, &cfg.scroll, &cfg.tmux, tmux_detected.as_ref(), &tray_tx).await;

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
) {
    let mut mapper_state = mapper::MapperState::new(scroll_cfg, tmux_cfg, tmux_detected);
    let mut buf = [0u8; 128];
    let mut consecutive_errors = 0u32;
    let mut first_report = true;
    let mut last_profile = mapper_state.profile();

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

                        // Update tray icon on profile change
                        let current_profile = mapper_state.profile();
                        if current_profile != last_profile {
                            let _ = tray_tx.send(tray::TrayCmd::SetProfile(current_profile));
                            last_profile = current_profile;
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

/// Output loop: update lightbar and rumble based on state changes.
async fn run_output_loop(
    handle: hid::HidHandle,
    ct: controller::ControllerType,
    conn: controller::ConnectionType,
    lightbar_cfg: config::LightbarConfig,
    state_rx: &mut watch::Receiver<AgentState>,
) {
    let mut bt_seq = 0u8;
    let mut current_state = AgentState::Idle;
    let state_entered_at = Instant::now();
    let mut state_start = state_entered_at;

    // Set initial lightbar
    send_output(
        &handle,
        ct,
        conn,
        &lightbar_cfg,
        current_state,
        0,
        &mut bt_seq,
    );

    let mut ticker = tokio::time::interval(Duration::from_millis(33)); // ~30fps for smooth pulse

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let elapsed = state_start.elapsed().as_millis() as u64;
                send_output(&handle, ct, conn, &lightbar_cfg, current_state, elapsed, &mut bt_seq);
            }
            result = state_rx.changed() => {
                if result.is_err() {
                    log::error!("State channel closed");
                    break;
                }
                let new_state = *state_rx.borrow();
                if new_state != current_state {
                    let old_state = current_state;
                    current_state = new_state;
                    state_start = Instant::now();

                    // Fire rumble pattern if applicable
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

fn send_output(
    handle: &hid::HidHandle,
    ct: controller::ControllerType,
    conn: controller::ConnectionType,
    lightbar_cfg: &config::LightbarConfig,
    state: AgentState,
    elapsed_ms: u64,
    bt_seq: &mut u8,
) {
    let (r, g, b) = lightbar::compute_color(lightbar_cfg, state, elapsed_ms);
    let out = OutputState {
        lightbar_r: r,
        lightbar_g: g,
        lightbar_b: b,
        rumble_left: 0,
        rumble_right: 0,
    };
    let report = output::build_report(ct, conn, &out, bt_seq);
    handle.write(&report);
}
