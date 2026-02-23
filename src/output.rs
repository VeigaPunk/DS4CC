/// Output report builder: desired controller state → raw HID bytes.
///
/// Report formats:
///
/// DualSense USB: Report ID 0x02, 48 bytes
///   Byte 0:  report ID (0x02)
///   Byte 1:  valid flag 0 (0x01 = rumble, 0x02 = right trigger effect, 0x04 = left trigger)
///   Byte 2:  valid flag 1 (0x04 = lightbar, 0x01 = mic LED, 0x02 = player LEDs)
///   Byte 3:  right rumble motor
///   Byte 4:  left rumble motor
///   Byte 44: lightbar red
///   Byte 45: lightbar green
///   Byte 46: lightbar blue
///
/// DualSense BT: Report ID 0x31, 78 bytes
///   Byte 0:  report ID (0x31)
///   Byte 1:  seq_tag | 0x10  (sequence number in high nibble)
///   Byte 2:  tag (0x10 for audio haptics tag)
///   Then same layout as USB offset by +1
///   Last 4 bytes: CRC-32 (seed 0xA2)
///
/// DS4 USB: Report ID 0x05, 32 bytes
///   Byte 0:  report ID (0x05)
///   Byte 1:  flags (0x07 = rumble + lightbar)
///   Byte 2:  0x00
///   Byte 3:  0x00
///   Byte 4:  right rumble motor
///   Byte 5:  left rumble motor
///   Byte 6:  lightbar red
///   Byte 7:  lightbar green
///   Byte 8:  lightbar blue
///
/// DS4 BT: Report ID 0x11, 79 bytes
///   Byte 0:  report ID (0x11)
///   Byte 1:  0x80 (HID output flag)
///   Byte 2:  0x00
///   Byte 3:  0xF7 (enable rumble + lightbar + flash)
///   Byte 6:  right rumble motor
///   Byte 7:  left rumble motor
///   Byte 8:  lightbar red
///   Byte 9:  lightbar green
///   Byte 10: lightbar blue
///   Last 4 bytes: CRC-32 (seed 0xA2)

use crate::controller::{ConnectionType, ControllerType};
use crate::crc32;

/// Desired output state to send to the controller.
#[derive(Debug, Clone, Copy, Default)]
pub struct OutputState {
    pub lightbar_r: u8,
    pub lightbar_g: u8,
    pub lightbar_b: u8,
    pub rumble_left: u8,
    pub rumble_right: u8,
    /// Player indicator LED bitmask (DualSense only).
    /// Bits 0-4 = 5 dots left→right. Bit 5 = instant mode (no fade).
    /// e.g. 0x04 = center dot, 0x24 = center dot + instant.
    pub player_leds: u8,
}

/// Build an output report. Returns the report as a Vec<u8> ready to write via HID.
pub fn build_report(
    ct: ControllerType,
    conn: ConnectionType,
    state: &OutputState,
    bt_seq: &mut u8,
) -> Vec<u8> {
    match (ct, conn) {
        (ControllerType::DualSense | ControllerType::DualSenseEdge, ConnectionType::Usb) => {
            build_dualsense_usb(state)
        }
        (ControllerType::DualSense | ControllerType::DualSenseEdge, ConnectionType::Bluetooth) => {
            build_dualsense_bt(state, bt_seq)
        }
        (ControllerType::Ds4V1 | ControllerType::Ds4V2, ConnectionType::Usb) => {
            build_ds4_usb(state)
        }
        (ControllerType::Ds4V1 | ControllerType::Ds4V2, ConnectionType::Bluetooth) => {
            build_ds4_bt(state)
        }
    }
}

/// DualSense USB output report — matches DS4Windows byte layout exactly.
/// Total: 48 bytes. Report ID 0x02.
fn build_dualsense_usb(state: &OutputState) -> Vec<u8> {
    let mut buf = vec![0u8; 48];
    buf[0] = 0x02;  // report ID
    buf[1] = 0x0F;  // valid_flag0: rumble + triggers (bits 0-3)
    buf[2] = 0x55;  // valid_flag1: mic LED + lightbar + player LEDs + power (DS4W value)
    buf[3] = state.rumble_right;
    buf[4] = state.rumble_left;
    buf[39] = 0x02; // valid_flag2: bit 1 = lightbar setup control enable
    buf[42] = 0x02; // lightbar_setup: fade out default blue LED
    buf[43] = 0x00; // led_brightness: 0x00=High
    buf[44] = state.player_leds;
    buf[45] = state.lightbar_r;
    buf[46] = state.lightbar_g;
    buf[47] = state.lightbar_b;
    buf
}

/// DualSense BT output report — matches DS4Windows byte layout exactly.
/// Total: 78 bytes. Report ID 0x31. DS4W uses [1]=0x02 fixed tag (no sequence).
fn build_dualsense_bt(state: &OutputState, _seq: &mut u8) -> Vec<u8> {
    let mut buf = vec![0u8; 78];
    buf[0] = 0x31;  // report ID
    buf[1] = 0x02;  // DS4W: fixed data tag (no sequence numbering)
    buf[2] = 0x0F;  // valid_flag0: rumble + triggers
    buf[3] = 0x55;  // valid_flag1: mic LED + lightbar + player LEDs + power
    buf[4] = state.rumble_right;
    buf[5] = state.rumble_left;
    buf[40] = 0x02; // valid_flag2: bit 1 = lightbar setup control enable
    buf[43] = 0x02; // lightbar_setup: fade out default blue LED
    buf[44] = 0x00; // led_brightness: 0x00=High
    buf[45] = state.player_leds;
    buf[46] = state.lightbar_r;
    buf[47] = state.lightbar_g;
    buf[48] = state.lightbar_b;

    // CRC-32 at last 4 bytes
    let crc_offset = buf.len() - 4;
    crc32::stamp(crc32::SEED_OUTPUT, &mut buf, crc_offset);
    buf
}

fn build_ds4_usb(state: &OutputState) -> Vec<u8> {
    let mut buf = vec![0u8; 32];
    buf[0] = 0x05; // report ID
    buf[1] = 0x07; // flags: rumble + lightbar
    buf[4] = state.rumble_right;
    buf[5] = state.rumble_left;
    buf[6] = state.lightbar_r;
    buf[7] = state.lightbar_g;
    buf[8] = state.lightbar_b;
    buf
}

fn build_ds4_bt(state: &OutputState) -> Vec<u8> {
    let mut buf = vec![0u8; 79];
    buf[0] = 0x11; // report ID
    buf[1] = 0x80; // HID output flag
    buf[3] = 0xF7; // enable rumble + lightbar + flash
    buf[6] = state.rumble_right;
    buf[7] = state.rumble_left;
    buf[8] = state.lightbar_r;
    buf[9] = state.lightbar_g;
    buf[10] = state.lightbar_b;

    // CRC-32 at last 4 bytes
    let crc_offset = buf.len() - 4;
    crc32::stamp(crc32::SEED_OUTPUT, &mut buf, crc_offset);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dualsense_usb_report_size() {
        let state = OutputState {
            lightbar_r: 255,
            lightbar_g: 128,
            lightbar_b: 0,
            rumble_left: 0,
            rumble_right: 0,
            player_leds: 0,
        };
        let mut seq = 0u8;
        let report = build_report(ControllerType::DualSense, ConnectionType::Usb, &state, &mut seq);
        assert_eq!(report.len(), 48);
        assert_eq!(report[0], 0x02);
        assert_eq!(report[45], 255); // red
        assert_eq!(report[46], 128); // green
        assert_eq!(report[47], 0);   // blue
    }

    #[test]
    fn dualsense_player_leds_byte_position() {
        // Center dot + instant mode (0x24) must land at buf[44] (USB) and buf[45] (BT).
        let state = OutputState { player_leds: 0x24, ..Default::default() };
        let mut seq = 0u8;
        let usb = build_report(ControllerType::DualSense, ConnectionType::Usb, &state, &mut seq);
        assert_eq!(usb[44], 0x24);
        let bt = build_report(ControllerType::DualSense, ConnectionType::Bluetooth, &state, &mut seq);
        assert_eq!(bt[45], 0x24);
    }

    #[test]
    fn dualsense_bt_report_size_and_crc() {
        let state = OutputState::default();
        let mut seq = 0u8;
        let report = build_report(ControllerType::DualSense, ConnectionType::Bluetooth, &state, &mut seq);
        assert_eq!(report.len(), 78);
        assert_eq!(report[0], 0x31);
        // Verify CRC is valid
        assert!(crc32::validate(crc32::SEED_OUTPUT, &report));
    }

    #[test]
    fn ds4_usb_report_size() {
        let state = OutputState {
            lightbar_r: 0,
            lightbar_g: 255,
            lightbar_b: 0,
            rumble_left: 128,
            rumble_right: 64,
            player_leds: 0,
        };
        let mut seq = 0u8;
        let report = build_report(ControllerType::Ds4V2, ConnectionType::Usb, &state, &mut seq);
        assert_eq!(report.len(), 32);
        assert_eq!(report[0], 0x05);
        assert_eq!(report[5], 128); // left rumble
        assert_eq!(report[4], 64);  // right rumble
        assert_eq!(report[7], 255); // green
    }

    #[test]
    fn ds4_bt_report_size_and_crc() {
        let state = OutputState::default();
        let mut seq = 0u8;
        let report = build_report(ControllerType::Ds4V2, ConnectionType::Bluetooth, &state, &mut seq);
        assert_eq!(report.len(), 79);
        assert_eq!(report[0], 0x11);
        assert!(crc32::validate(crc32::SEED_OUTPUT, &report));
    }

    #[test]
    fn dualsense_bt_fixed_tag() {
        let state = OutputState::default();
        let mut seq = 0u8;
        let r1 = build_report(ControllerType::DualSense, ConnectionType::Bluetooth, &state, &mut seq);
        let r2 = build_report(ControllerType::DualSense, ConnectionType::Bluetooth, &state, &mut seq);
        // DS4W uses fixed tag 0x02 at byte 1 (no sequence)
        assert_eq!(r1[1], 0x02);
        assert_eq!(r2[1], 0x02);
    }
}
