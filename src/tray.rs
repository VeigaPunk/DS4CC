/// System tray icon: DualSense PNG silhouette, luminance-tinted per profile.
/// White on OLED black = Default profile.
/// Neon green on OLED black = Tmux profile.
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
use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc};

use tray_icon::{Icon, TrayIconBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};

use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

const ICON_SIZE: u32 = 32;
const APP_NAME: &str = "DS4CC";
const REG_RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// Commands from the async runtime to the tray thread.
pub enum TrayCmd {
    SetProfile(Profile),
}

/// Spawn the tray icon on a background thread. Returns a channel sender.
pub fn spawn(initial: Profile, mouse_stick_active: Arc<AtomicBool>) -> mpsc::Sender<TrayCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || run(rx, initial, mouse_stick_active))
        .expect("spawn tray thread");
    tx
}

fn run(rx: mpsc::Receiver<TrayCmd>, initial: Profile, mouse_stick_active: Arc<AtomicBool>) {
    let auto_start_enabled = is_auto_start_enabled();
    let stick_initially = mouse_stick_active.load(Ordering::Relaxed);
    let (r, g, b) = profile_color(initial);
    let icon = make_icon(r, g, b);

    // Build context menu
    let wispr_item    = MenuItem::new("Open Wispr Flow", true, None);
    let restart_item  = MenuItem::new("Restart", true, None);
    let startup_item  = CheckMenuItem::new("Enable auto start-up", true, auto_start_enabled, None);
    let stick_item    = CheckMenuItem::new("Mouse: Left Stick", true, stick_initially, None);
    let exit_item     = MenuItem::new("Exit", true, None);

    // Capture IDs for event matching
    let wispr_id   = wispr_item.id().clone();
    let restart_id = restart_item.id().clone();
    let startup_id = startup_item.id().clone();
    let stick_id   = stick_item.id().clone();
    let exit_id    = exit_item.id().clone();

    let menu = Menu::new();
    menu.append(&wispr_item).expect("menu append");
    menu.append(&restart_item).expect("menu append");
    menu.append(&startup_item).expect("menu append");
    menu.append(&stick_item).expect("menu append");
    menu.append(&PredefinedMenuItem::separator()).expect("menu append");
    menu.append(&exit_item).expect("menu append");

    let tray = match TrayIconBuilder::new()
        .with_tooltip(format!("DS4CC — {initial}"))
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
            } else if event.id == stick_id {
                let stick = stick_item.is_checked();
                mouse_stick_active.store(stick, Ordering::Relaxed);
                let mode = if stick { "left stick" } else { "touchpad" };
                log::info!("Mouse cursor mode: {mode}");
            }
        }

        match rx.try_recv() {
            Ok(TrayCmd::SetProfile(profile)) => {
                let (r, g, b) = profile_color(profile);
                let _ = tray.set_icon(Some(make_icon(r, g, b)));
                let _ = tray.set_tooltip(Some(format!("DS4CC — {profile}")));
            }
            Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

// ── Menu actions ──────────────────────────────────────────────────────

fn open_wispr_flow() {
    match find_wispr_flow() {
        Some(path) => {
            log::info!("Launching Wispr Flow: {}", path.display());
            if let Err(e) = std::process::Command::new(&path).spawn() {
                log::error!("Failed to launch Wispr Flow: {e}");
            }
        }
        None => {
            log::warn!("Wispr Flow not found — prompting user");
            prompt_download_wispr_flow();
        }
    }
}

/// Search for the Wispr Flow executable.
///
/// Resolution order:
///   1. HKLM App Paths registry key (reliable if installer registered it)
///   2. Common install locations under %LOCALAPPDATA%, %PROGRAMFILES%, %PROGRAMFILES(X86)%
fn find_wispr_flow() -> Option<PathBuf> {
    // 1. Registry App Paths
    if let Some(path) = wispr_flow_from_app_paths() {
        return Some(path);
    }

    // 2. Known filesystem locations
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(PathBuf::from(&local).join("WisprFlow").join("Wispr Flow.exe"));
        candidates.push(PathBuf::from(&local).join("Programs").join("Wispr Flow").join("Wispr Flow.exe"));
        candidates.push(PathBuf::from(&local).join("Programs").join("wispr-flow").join("Wispr Flow.exe"));
        candidates.push(PathBuf::from(&local).join("Programs").join("WisprFlow").join("WisprFlow.exe"));
    }
    if let Ok(pf) = std::env::var("PROGRAMFILES") {
        candidates.push(PathBuf::from(&pf).join("Wispr Flow").join("Wispr Flow.exe"));
        candidates.push(PathBuf::from(&pf).join("WisprFlow").join("Wispr Flow.exe"));
    }
    if let Ok(pf86) = std::env::var("PROGRAMFILES(X86)") {
        candidates.push(PathBuf::from(&pf86).join("Wispr Flow").join("Wispr Flow.exe"));
    }

    candidates.into_iter().find(|p| p.exists())
}

/// Query HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\Wispr Flow.exe
fn wispr_flow_from_app_paths() -> Option<PathBuf> {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\Wispr Flow.exe",
            "/ve",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // Output format:  "    (Default)    REG_SZ    C:\path\to\Wispr Flow.exe"
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("REG_SZ") {
            if let Some(value) = line.split("REG_SZ").nth(1) {
                let path = PathBuf::from(value.trim());
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }

    None
}

/// Show a Yes/No dialog when Wispr Flow can't be found.
/// "Yes" opens the download page; "No" closes the dialog.
fn prompt_download_wispr_flow() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONWARNING, MB_YESNO, IDYES,
    };

    let text: Vec<u16> = "Wispr Flow couldn't be located. Speech to Text won't work without it.\n\nWant to download?"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let caption: Vec<u16> = "Wispr Flow Not Found"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_YESNO | MB_ICONWARNING,
        )
    };

    if result == IDYES {
        // Open default browser to the Wispr Flow website
        let _ = std::process::Command::new("explorer.exe")
            .arg("https://ref.wisprflow.ai/vgpnk")
            .spawn();
    }
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

// ── Embedded controller PNG ────────────────────────────────────────────

/// White DualSense silhouette on near-black background.
/// Same source image used for icon.ico (exe / installer icon).
const ICON_PNG: &[u8] = include_bytes!("../imgs/ChatGPT Image Feb 23, 2026, 05_30_47 AM.png");

// ── Profile colors / icon ─────────────────────────────────────────────

fn profile_color(profile: Profile) -> (u8, u8, u8) {
    match profile {
        Profile::Default => (255, 255, 255), // white on OLED black
        Profile::Tmux    => (57, 255, 20),   // neon green (#39FF14)
    }
}

/// Load the embedded DualSense PNG, resize to 32×32, and tint the silhouette.
///
/// The source image is a white controller on a near-black background.
/// Each output pixel is fully opaque — luminance of the source pixel scales
/// the tint color, so the white silhouette becomes the tint, edges anti-alias
/// smoothly, and the OLED-black background stays black.
fn make_icon(r: u8, g: u8, b: u8) -> Icon {
    let img = image::load_from_memory(ICON_PNG)
        .expect("embedded controller PNG is valid")
        .resize_exact(ICON_SIZE, ICON_SIZE, image::imageops::FilterType::Lanczos3)
        .into_rgb8();

    let mut rgba = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for pixel in img.pixels() {
        // Rec. 601 luminance (0–255): white silhouette → high, black bg → low.
        let lum = (pixel[0] as u32 * 299
            + pixel[1] as u32 * 587
            + pixel[2] as u32 * 114) / 1000;
        let tr = (r as u32 * lum / 255) as u8;
        let tg = (g as u32 * lum / 255) as u8;
        let tb = (b as u32 * lum / 255) as u8;
        rgba.extend_from_slice(&[tr, tg, tb, 255]);
    }

    Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).expect("valid icon data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_icon_loads() {
        let (r, g, b) = profile_color(Profile::Default);
        make_icon(r, g, b); // must not panic
    }

    #[test]
    fn tmux_icon_loads() {
        let (r, g, b) = profile_color(Profile::Tmux);
        make_icon(r, g, b); // must not panic
    }

    #[test]
    fn rgba_has_correct_size() {
        let (r, g, b) = profile_color(Profile::Default);
        let icon = make_icon(r, g, b);
        drop(icon); // Icon::from_rgba already validates size internally
    }
}
