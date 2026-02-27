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

/// Find all supported controllers, sorted with USB devices first.
/// When a controller is connected via both USB and Bluetooth simultaneously,
/// USB will always appear first — callers can `.next()` to pick the preferred one.
pub fn find_all_controllers(api: &HidApi) -> Vec<ControllerInfo> {
    let mut usb = Vec::new();
    let mut bt = Vec::new();

    for dev in api.device_list() {
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
            let info = ControllerInfo {
                controller_type: ct,
                connection_type: conn,
                path,
            };
            match conn {
                ConnectionType::Usb => usb.push(info),
                ConnectionType::Bluetooth => bt.push(info),
            }
        }
    }

    usb.extend(bt);
    usb
}

/// Quick check: is there a USB controller present?
/// Used by the background USB scanner thread — avoids allocating a Vec.
pub fn has_usb_controller(api: &HidApi) -> bool {
    api.device_list().any(|dev| {
        dev.usage_page() == GAMEPAD_USAGE_PAGE
            && dev.usage() == GAMEPAD_USAGE
            && controller::identify(dev.vendor_id(), dev.product_id()).is_some()
            && controller::detect_connection(&dev.path().to_string_lossy())
                == ConnectionType::Usb
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_sorts_before_bt() {
        let bt = ControllerInfo {
            controller_type: ControllerType::DualSense,
            connection_type: ConnectionType::Bluetooth,
            path: "bt_path".into(),
        };
        let usb = ControllerInfo {
            controller_type: ControllerType::DualSense,
            connection_type: ConnectionType::Usb,
            path: "usb_path".into(),
        };
        // Simulate the two-vec ordering from find_all_controllers
        let mut usb_vec = vec![usb];
        let bt_vec = vec![bt];
        usb_vec.extend(bt_vec);
        assert_eq!(usb_vec[0].connection_type, ConnectionType::Usb);
        assert_eq!(usb_vec[1].connection_type, ConnectionType::Bluetooth);
    }

    #[test]
    fn single_bt_when_no_usb() {
        let bt = ControllerInfo {
            controller_type: ControllerType::DualSense,
            connection_type: ConnectionType::Bluetooth,
            path: "bt_path".into(),
        };
        let mut usb_vec: Vec<ControllerInfo> = Vec::new();
        let bt_vec = vec![bt];
        usb_vec.extend(bt_vec);
        assert_eq!(usb_vec.len(), 1);
        assert_eq!(usb_vec[0].connection_type, ConnectionType::Bluetooth);
    }
}
