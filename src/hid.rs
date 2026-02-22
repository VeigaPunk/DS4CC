/// HID device management: open controller, read input reports, write output reports.
///
/// Key DS4Windows patterns replicated here:
/// - Filter by VID/PID + usage page 0x01 / usage 0x05 (gamepad collection)
/// - Activate Bluetooth extended mode via feature report
/// - Non-blocking read with timeout
/// - Write errors are non-fatal (log and continue)

use crate::controller::{self, ConnectionType, ControllerType, GAMEPAD_USAGE, GAMEPAD_USAGE_PAGE};
use hidapi::{HidApi, HidDevice};
use std::sync::{Arc, Mutex};

/// Information about a discovered controller.
pub struct ControllerInfo {
    pub controller_type: ControllerType,
    pub connection_type: ConnectionType,
    pub path: String,
}

/// Find the first supported controller.
pub fn find_controller(api: &HidApi) -> Option<ControllerInfo> {
    for dev in api.device_list() {
        // Filter by usage page and usage for gamepad collection
        if dev.usage_page() != GAMEPAD_USAGE_PAGE || dev.usage() != GAMEPAD_USAGE {
            continue;
        }

        if let Some(ct) = controller::identify(dev.vendor_id(), dev.product_id()) {
            let path = dev.path().to_string_lossy().to_string();
            let conn = controller::detect_connection(&path);
            log::info!(
                "Found {} ({}) at {}",
                ct,
                conn,
                &path[..path.len().min(60)]
            );
            return Some(ControllerInfo {
                controller_type: ct,
                connection_type: conn,
                path,
            });
        }
    }
    None
}

/// Open the controller device.
pub fn open_device(api: &HidApi, info: &ControllerInfo) -> Result<HidDevice, hidapi::HidError> {
    let cpath = std::ffi::CString::new(info.path.as_bytes()).map_err(|_| {
        hidapi::HidError::HidApiError {
            message: "Invalid device path".into(),
        }
    })?;
    let device = api.open_path(&cpath)?;
    device.set_blocking_mode(false)?;
    Ok(device)
}

/// Activate Bluetooth extended mode by reading the appropriate feature report.
/// DualSense: feature report 0x05
/// DS4: feature report 0x02
pub fn activate_bt_extended_mode(
    device: &HidDevice,
    ct: ControllerType,
) -> Result<(), hidapi::HidError> {
    let report_id = if ct.is_dualsense() { 0x05 } else { 0x02 };
    let mut buf = [0u8; 64];
    buf[0] = report_id;
    match device.get_feature_report(&mut buf) {
        Ok(n) => {
            log::info!("BT extended mode activated (feature report 0x{report_id:02X}, {n} bytes)");
            Ok(())
        }
        Err(e) => {
            log::warn!("Failed to read feature report 0x{report_id:02X}: {e}");
            Err(e)
        }
    }
}

/// Wrapper around HidDevice for thread-safe write access.
/// Reads happen on the dedicated HID thread; writes can come from the lightbar/rumble tasks.
pub struct HidHandle {
    device: Arc<Mutex<HidDevice>>,
}

impl HidHandle {
    pub fn new(device: HidDevice) -> Self {
        Self {
            device: Arc::new(Mutex::new(device)),
        }
    }

    /// Clone the handle for sharing across tasks.
    pub fn clone_handle(&self) -> Self {
        Self {
            device: Arc::clone(&self.device),
        }
    }

    /// Read an input report.
    /// Returns Ok(n) with bytes read (0 = no data available).
    /// Returns Err(()) if the device is disconnected.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, ()> {
        let dev = self.device.lock().unwrap();
        match dev.read_timeout(buf, 5) {
            Ok(n) => Ok(n),
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("1167") || msg.contains("not connected") {
                    Err(()) // device disconnected
                } else {
                    log::error!("HID read error: {e}");
                    Ok(0)
                }
            }
        }
    }

    /// Write an output report. Errors are logged but not propagated (non-fatal).
    pub fn write(&self, report: &[u8]) -> bool {
        let dev = self.device.lock().unwrap();
        match dev.write(report) {
            Ok(_) => true,
            Err(e) => {
                log::debug!("HID write error (non-fatal): {e}");
                false
            }
        }
    }

}
