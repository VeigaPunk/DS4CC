/// Check for updates via GitHub Releases API.
/// Downloads and runs the installer if a newer version is available.

use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONINFORMATION, MB_ICONWARNING, MB_YESNO, IDYES,
};

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const API_URL: &str = "https://api.github.com/repos/VeigaPunk/DS4CC/releases/latest";
const INSTALLER_NAME: &str = "DS4CC-Setup.exe";

/// Entry point — called from tray thread via `std::thread::spawn`.
pub fn check_for_update() {
    match check_inner() {
        Ok(()) => {}
        Err(e) => {
            log::error!("Update check failed: {e}");
            show_msg(
                "Could not check for updates.\nCheck your internet connection.",
                "Update Check Failed",
                MB_ICONWARNING,
            );
        }
    }
}

fn check_inner() -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Checking for updates...");

    let body: String = ureq::get(API_URL)
        .header("User-Agent", "DS4CC")
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_to_string()?;

    let json: serde_json::Value = serde_json::from_str(&body)?;

    let tag = json["tag_name"].as_str().ok_or("missing tag_name")?;
    let remote_version = tag.strip_prefix('v').unwrap_or(tag);

    if !is_newer(remote_version, CURRENT_VERSION) {
        log::info!("Already on latest version (v{CURRENT_VERSION})");
        show_msg(
            &format!("You're on the latest version (v{CURRENT_VERSION})."),
            "DS4CC Update",
            MB_ICONINFORMATION,
        );
        return Ok(());
    }

    // Find installer download URL
    let download_url = json["assets"]
        .as_array()
        .and_then(|assets| {
            assets.iter().find_map(|a| {
                let name = a["name"].as_str()?;
                if name == INSTALLER_NAME {
                    a["browser_download_url"].as_str().map(String::from)
                } else {
                    None
                }
            })
        })
        .ok_or("installer asset not found in release")?;

    log::info!("New version available: v{remote_version} (current: v{CURRENT_VERSION})");

    let msg = format!(
        "Version v{remote_version} is available (you have v{CURRENT_VERSION}).\n\nDownload and install?"
    );

    if !ask_yes_no(&msg, "DS4CC Update Available") {
        return Ok(());
    }

    // Download installer to %TEMP%
    let temp = std::env::temp_dir().join(INSTALLER_NAME);
    log::info!("Downloading installer to {}", temp.display());

    let bytes = ureq::get(&download_url)
        .header("User-Agent", "DS4CC")
        .call()?
        .body_mut()
        .with_config()
        .limit(50 * 1024 * 1024)
        .read_to_vec()?;

    std::fs::write(&temp, &bytes)?;
    log::info!("Installer downloaded ({} bytes)", bytes.len());

    // Run installer and exit
    log::info!("Launching installer...");
    std::process::Command::new(&temp).spawn()?;
    std::process::exit(0);
}

/// Returns true if `remote` is newer than `current` (semver comparison).
fn is_newer(remote: &str, current: &str) -> bool {
    let parse = |v: &str| -> Option<(u32, u32, u32)> {
        let parts: Vec<&str> = v.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        Some((
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ))
    };

    match (parse(remote), parse(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

fn show_msg(text: &str, caption: &str, flags: u32) {
    let text_w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let cap_w: Vec<u16> = caption.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text_w.as_ptr(), cap_w.as_ptr(), flags);
    }
}

fn ask_yes_no(text: &str, caption: &str) -> bool {
    let text_w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let cap_w: Vec<u16> = caption.encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text_w.as_ptr(),
            cap_w.as_ptr(),
            MB_YESNO | MB_ICONINFORMATION,
        )
    };
    result == IDYES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_newer() {
        assert!(is_newer("2.7.0", "2.6.0"));
        assert!(is_newer("3.0.0", "2.6.0"));
        assert!(is_newer("2.6.1", "2.6.0"));
    }

    #[test]
    fn version_same_or_older() {
        assert!(!is_newer("2.6.0", "2.6.0"));
        assert!(!is_newer("2.5.0", "2.6.0"));
        assert!(!is_newer("1.0.0", "2.6.0"));
    }
}
