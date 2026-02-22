/// Controller identification: VID/PID matching and connection type detection.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerType {
    DualSense,
    DualSenseEdge,
    Ds4V1,
    Ds4V2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionType {
    Usb,
    Bluetooth,
}

/// Known VID/PID pairs.
const SONY_VID: u16 = 0x054C;
const DUALSENSE_PID: u16 = 0x0CE6;
const DUALSENSE_EDGE_PID: u16 = 0x0DF2;
const DS4_V1_PID: u16 = 0x05C4;
const DS4_V2_PID: u16 = 0x09CC;

/// HID usage page and usage for gamepad collections.
pub const GAMEPAD_USAGE_PAGE: u16 = 0x01; // Generic Desktop
pub const GAMEPAD_USAGE: u16 = 0x05; // Game Pad

/// Identify controller type from VID/PID. Returns None for unknown devices.
pub fn identify(vid: u16, pid: u16) -> Option<ControllerType> {
    if vid != SONY_VID {
        return None;
    }
    match pid {
        DUALSENSE_PID => Some(ControllerType::DualSense),
        DUALSENSE_EDGE_PID => Some(ControllerType::DualSenseEdge),
        DS4_V1_PID => Some(ControllerType::Ds4V1),
        DS4_V2_PID => Some(ControllerType::Ds4V2),
        _ => None,
    }
}

/// Detect connection type from HID device path.
/// DS4Windows heuristic: Bluetooth paths on Windows contain "&col02" or similar
/// patterns and the interface number differs from USB.
/// More reliably: USB paths contain "usb#" or "hid#vid_", BT paths contain
/// "&0005" (HID over Bluetooth) or "{00001124" (Bluetooth HID GUID).
pub fn detect_connection(path: &str) -> ConnectionType {
    let lower = path.to_ascii_lowercase();
    // USB HID paths on Windows typically contain "usb#" or "\\?\hid#vid_"
    // Bluetooth paths contain bluetooth-specific GUIDs or "&0005"
    if lower.contains("&0005") || lower.contains("{00001124") {
        ConnectionType::Bluetooth
    } else {
        // Default to USB — if in doubt, USB is safer (no CRC needed)
        ConnectionType::Usb
    }
}

impl ControllerType {
    /// Returns true if this is a DualSense-family controller.
    pub fn is_dualsense(self) -> bool {
        matches!(self, ControllerType::DualSense | ControllerType::DualSenseEdge)
    }

    /// Returns true if this is a DS4-family controller.
    pub fn is_ds4(self) -> bool {
        matches!(self, ControllerType::Ds4V1 | ControllerType::Ds4V2)
    }

    pub fn name(self) -> &'static str {
        match self {
            ControllerType::DualSense => "DualSense",
            ControllerType::DualSenseEdge => "DualSense Edge",
            ControllerType::Ds4V1 => "DualShock 4 v1",
            ControllerType::Ds4V2 => "DualShock 4 v2",
        }
    }
}

impl std::fmt::Display for ControllerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionType::Usb => f.write_str("USB"),
            ConnectionType::Bluetooth => f.write_str("Bluetooth"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identify_known_controllers() {
        assert_eq!(identify(0x054C, 0x0CE6), Some(ControllerType::DualSense));
        assert_eq!(identify(0x054C, 0x0DF2), Some(ControllerType::DualSenseEdge));
        assert_eq!(identify(0x054C, 0x05C4), Some(ControllerType::Ds4V1));
        assert_eq!(identify(0x054C, 0x09CC), Some(ControllerType::Ds4V2));
    }

    #[test]
    fn identify_unknown() {
        assert_eq!(identify(0x054C, 0x0000), None);
        assert_eq!(identify(0x0001, 0x0CE6), None);
    }

    #[test]
    fn detect_usb_path() {
        let path = r"\\?\hid#vid_054c&pid_0ce6&mi_03#8&hash&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(detect_connection(path), ConnectionType::Usb);
    }

    #[test]
    fn detect_bt_path() {
        let path = r"\\?\hid#{00001124-0000-1000-8000-00805f9b34fb}_vid&0002054c_pid&0ce6#8&hash&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(detect_connection(path), ConnectionType::Bluetooth);
    }
}
