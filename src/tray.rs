/// System tray icon: DualShock controller silhouette.
/// Black = Default profile, Green = Tmux profile.
///
/// Right-click context menu:
///   Open Wispr Flow
///   Restart
///   Enable auto start-up  [toggle]
///   ──────────────────────
///   Exit
///
/// Runs on a dedicated OS thread with a Win32 message pump.
/// The async runtime sends [`TrayCmd`] messages to update the icon.

use crate::mapper::Profile;
use std::sync::mpsc;

use tray_icon::{Icon, TrayIconBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};

use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

const ICON_SIZE: u32 = 32;
const APP_NAME: &str = "GamePadCC";
const REG_RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// Commands from the async runtime to the tray thread.
pub enum TrayCmd {
    SetProfile(Profile),
}

/// Spawn the tray icon on a background thread. Returns a channel sender.
pub fn spawn(initial: Profile) -> mpsc::Sender<TrayCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || run(rx, initial))
        .expect("spawn tray thread");
    tx
}

fn run(rx: mpsc::Receiver<TrayCmd>, initial: Profile) {
    let auto_start_enabled = is_auto_start_enabled();
    let (r, g, b) = profile_color(initial);
    let icon = make_icon(r, g, b);

    // Build context menu
    let wispr_item    = MenuItem::new("Open Wispr Flow", true, None);
    let restart_item  = MenuItem::new("Restart", true, None);
    let startup_item  = CheckMenuItem::new("Enable auto start-up", true, auto_start_enabled, None);
    let exit_item     = MenuItem::new("Exit", true, None);

    // Capture IDs for event matching
    let wispr_id   = wispr_item.id().clone();
    let restart_id = restart_item.id().clone();
    let startup_id = startup_item.id().clone();
    let exit_id    = exit_item.id().clone();

    let menu = Menu::new();
    menu.append(&wispr_item).expect("menu append");
    menu.append(&restart_item).expect("menu append");
    menu.append(&startup_item).expect("menu append");
    menu.append(&PredefinedMenuItem::separator()).expect("menu append");
    menu.append(&exit_item).expect("menu append");

    let tray = match TrayIconBuilder::new()
        .with_tooltip(format!("GamePadCC — {initial}"))
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .build()
    {
        Ok(t) => t,
        Err(e) => {
            log::error!("Failed to create tray icon: {e}");
            return;
        }
    };

    log::info!("Tray icon created (profile: {initial}, auto-start: {auto_start_enabled})");

    loop {
        // Pump Win32 messages so the tray icon stays responsive.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // Handle menu events
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == exit_id {
                std::process::exit(0);
            } else if event.id == restart_id {
                restart_app();
            } else if event.id == wispr_id {
                open_wispr_flow();
            } else if event.id == startup_id {
                // CheckMenuItem auto-toggles on click; is_checked() reflects new state
                set_auto_start(startup_item.is_checked());
            }
        }

        match rx.try_recv() {
            Ok(TrayCmd::SetProfile(profile)) => {
                let (r, g, b) = profile_color(profile);
                let _ = tray.set_icon(Some(make_icon(r, g, b)));
                let _ = tray.set_tooltip(Some(format!("GamePadCC — {profile}")));
            }
            Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

// ── Menu actions ──────────────────────────────────────────────────────

fn open_wispr_flow() {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    // Try known Electron-style install paths
    let candidates = [
        format!("{local}\\Programs\\Wispr Flow\\Wispr Flow.exe"),
        format!("{local}\\Programs\\wispr-flow\\Wispr Flow.exe"),
        format!("{local}\\Programs\\WisprFlow\\WisprFlow.exe"),
    ];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            if let Err(e) = std::process::Command::new(path).spawn() {
                log::warn!("Failed to launch Wispr Flow at {path}: {e}");
            }
            return;
        }
    }
    // Fall back: let Windows resolve via App Paths registry
    log::warn!("Wispr Flow not found at known paths, trying shell resolution");
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "Wispr Flow.exe"])
        .spawn();
}

fn restart_app() {
    if let Ok(exe) = std::env::current_exe() {
        if let Err(e) = std::process::Command::new(&exe).spawn() {
            log::error!("Failed to restart: {e}");
            return;
        }
    }
    std::process::exit(0);
}

// ── Auto-startup (HKCU Run registry key) ─────────────────────────────

fn is_auto_start_enabled() -> bool {
    std::process::Command::new("reg")
        .args(["query", REG_RUN_KEY, "/v", APP_NAME])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn set_auto_start(enabled: bool) {
    if enabled {
        let Ok(exe) = std::env::current_exe() else {
            log::error!("Cannot determine exe path for auto-start");
            return;
        };
        // Quote path to handle spaces
        let value = format!("\"{}\"", exe.to_string_lossy());
        let status = std::process::Command::new("reg")
            .args(["add", REG_RUN_KEY, "/v", APP_NAME, "/t", "REG_SZ", "/d", &value, "/f"])
            .status();
        match status {
            Ok(s) if s.success() => log::info!("Auto-start enabled: {value}"),
            Ok(s) => log::warn!("Auto-start reg add failed (exit {s})"),
            Err(e) => log::warn!("Auto-start reg add error: {e}"),
        }
    } else {
        let status = std::process::Command::new("reg")
            .args(["delete", REG_RUN_KEY, "/v", APP_NAME, "/f"])
            .status();
        match status {
            Ok(s) if s.success() => log::info!("Auto-start disabled"),
            Ok(s) => log::warn!("Auto-start reg delete failed (exit {s})"),
            Err(e) => log::warn!("Auto-start reg delete error: {e}"),
        }
    }
}

// ── Profile colors / icon ─────────────────────────────────────────────

fn profile_color(profile: Profile) -> (u8, u8, u8) {
    match profile {
        Profile::Default => (40, 40, 40),
        Profile::Tmux => (0, 190, 0),
    }
}

fn make_icon(r: u8, g: u8, b: u8) -> Icon {
    let rgba = generate_controller_rgba(r, g, b);
    Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).expect("valid icon data")
}

/// Generate 32×32 RGBA pixels of a DualShock-style controller silhouette.
fn generate_controller_rgba(r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut rgba = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];

    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            if controller_shape(x as f32, y as f32) {
                let i = ((y * ICON_SIZE + x) * 4) as usize;
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = 255;
            }
        }
    }

    rgba
}

/// Returns true if pixel (x, y) is inside the controller silhouette.
///
/// Shape: wide elliptical body (upper portion) + two tapered grip prongs (lower).
fn controller_shape(x: f32, y: f32) -> bool {
    let cx = 15.5;
    let cy = 12.0;

    // Main body — wide ellipse
    let ex = (x - cx) / 13.5;
    let ey = (y - cy) / 9.0;
    if ex * ex + ey * ey <= 1.0 {
        return true;
    }

    // Grips — extend below the body
    if y >= 17.0 && y <= 28.0 {
        let t = (y - 17.0) / 11.0; // 0 at top → 1 at bottom

        let hw = 4.5 - t * 1.5; // half-width narrows downward

        // Left grip
        let lc = 7.5 + t * 1.5; // center drifts slightly inward
        if (x - lc).abs() <= hw {
            return true;
        }

        // Right grip (mirror)
        let rc = 23.5 - t * 1.5;
        if (x - rc).abs() <= hw {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_center_is_filled() {
        assert!(controller_shape(15.0, 12.0));
        assert!(controller_shape(16.0, 12.0));
    }

    #[test]
    fn icon_corners_are_empty() {
        assert!(!controller_shape(0.0, 0.0));
        assert!(!controller_shape(31.0, 0.0));
        assert!(!controller_shape(0.0, 31.0));
        assert!(!controller_shape(31.0, 31.0));
    }

    #[test]
    fn icon_grips_exist() {
        // Left grip at row 22
        assert!(controller_shape(8.0, 22.0));
        // Right grip at row 22
        assert!(controller_shape(23.0, 22.0));
        // Gap between grips
        assert!(!controller_shape(15.5, 25.0));
    }

    #[test]
    fn rgba_has_correct_size() {
        let rgba = generate_controller_rgba(255, 0, 0);
        assert_eq!(rgba.len(), (32 * 32 * 4) as usize);
    }
}
