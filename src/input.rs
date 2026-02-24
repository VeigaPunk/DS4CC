/// Input report parsing: raw HID bytes → UnifiedInput.
///
/// Report formats (from DS4Windows research):
///
/// DualSense USB: Report ID 0x01, 64 bytes total
///   Byte 0: left stick X
///   Byte 1: left stick Y
///   Byte 2: right stick X
///   Byte 3: right stick Y
///   Byte 4: L2 analog
///   Byte 5: R2 analog
///   Byte 7: buttons byte 0 (hat + square/cross/circle/triangle)
///   Byte 8: buttons byte 1 (L1/R1/L2btn/R2btn/share/options/L3/R3)
///   Byte 9: buttons byte 2 (PS/touchpad/mute)
///
/// DualSense BT: Report ID 0x31, 78 bytes total (extended mode)
///   Same layout but offset by +1 byte (report ID prefix on BT)
///   Last 4 bytes are CRC-32
///
/// DS4 USB: Report ID 0x01, 64 bytes total
///   Same stick/trigger layout as DualSense at same offsets
///   Byte 4: buttons byte 0 (hat + square/cross/circle/triangle)  [NOTE: offset differs]
///   Byte 5: buttons byte 1
///   Byte 6: buttons byte 2
///
/// DS4 BT: Report ID 0x11, 78 bytes (extended mode)
///   Offset by +2 bytes from USB layout
///   Last 4 bytes are CRC-32

use crate::controller::{ConnectionType, ControllerType};
use crate::crc32;

/// A single capacitive touch contact on the DualSense touchpad.
///
/// Coordinates are in touchpad units:
///   X: 0 (left) – 1919 (right)
///   Y: 0 (top)  – 1079 (bottom)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TouchPoint {
    pub active: bool,
    pub x: u16,
    pub y: u16,
}

/// D-pad direction decoded from the 4-bit hat field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DPad {
    #[default]
    Neutral,
    Up,
    UpRight,
    Right,
    DownRight,
    Down,
    DownLeft,
    Left,
    UpLeft,
}

/// All button states in a single struct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ButtonState {
    pub cross: bool,
    pub circle: bool,
    pub square: bool,
    pub triangle: bool,
    pub l1: bool,
    pub r1: bool,
    pub l2: bool,
    pub r2: bool,
    pub share: bool,   // "Create" on DualSense
    pub options: bool,
    pub l3: bool,
    pub r3: bool,
    pub ps: bool,
    pub touchpad: bool,
    pub mute: bool, // DualSense only
    pub dpad: DPad,
}

/// Normalized input from any supported controller.
#[derive(Debug, Clone, Copy)]
pub struct UnifiedInput {
    pub left_stick: (u8, u8),
    pub right_stick: (u8, u8),
    pub l2_analog: u8,
    pub r2_analog: u8,
    pub buttons: ButtonState,
    /// Touchpad contacts (DualSense only; DS4 always returns [default; 2]).
    pub touchpad: [TouchPoint; 2],
}

impl Default for UnifiedInput {
    fn default() -> Self {
        Self {
            left_stick: (128, 128),  // center
            right_stick: (128, 128), // center
            l2_analog: 0,
            r2_analog: 0,
            buttons: ButtonState::default(),
            touchpad: [TouchPoint::default(); 2],
        }
    }
}

/// Parse result.
#[derive(Debug)]
pub enum ParseError {
    TooShort { expected: usize, got: usize },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::TooShort { expected, got } => {
                write!(f, "report too short: expected {expected}, got {got}")
            }
        }
    }
}

/// Decode the 4-bit hat switch value into a DPad direction.
fn decode_hat(hat: u8) -> DPad {
    match hat & 0x0F {
        0 => DPad::Up,
        1 => DPad::UpRight,
        2 => DPad::Right,
        3 => DPad::DownRight,
        4 => DPad::Down,
        5 => DPad::DownLeft,
        6 => DPad::Left,
        7 => DPad::UpLeft,
        _ => DPad::Neutral, // 8+ = released
    }
}

/// Parse buttons from the standard 3-byte button block.
/// `b0`: hat(low nibble) + square/cross/circle/triangle(high nibble)
/// `b1`: L1/R1/L2btn/R2btn/share/options/L3/R3
/// `b2`: PS/touchpad/mute(DualSense only)
fn parse_buttons(b0: u8, b1: u8, b2: u8) -> ButtonState {
    ButtonState {
        dpad: decode_hat(b0 & 0x0F),
        square: b0 & 0x10 != 0,
        cross: b0 & 0x20 != 0,
        circle: b0 & 0x40 != 0,
        triangle: b0 & 0x80 != 0,
        l1: b1 & 0x01 != 0,
        r1: b1 & 0x02 != 0,
        l2: b1 & 0x04 != 0,
        r2: b1 & 0x08 != 0,
        share: b1 & 0x10 != 0,
        options: b1 & 0x20 != 0,
        l3: b1 & 0x40 != 0,
        r3: b1 & 0x80 != 0,
        ps: b2 & 0x01 != 0,
        touchpad: b2 & 0x02 != 0,
        mute: b2 & 0x04 != 0,
    }
}

/// Parse the two DualSense touchpad contact points starting at `data[off + 32]`.
///
/// HID layout (4 bytes per contact, from Linux `hid-playstation.c`):
///   byte 0: contact byte — bit 7 = inactive (0 = active), bits 0–6 = finger ID
///   byte 1: x_lo  — lower 8 bits of X (range 0–1919)
///   byte 2: bits 0–3 = x_hi (upper 4 bits of X), bits 4–7 = y_lo (lower 4 bits of Y)
///   byte 3: y_hi  — upper 8 bits of Y (range 0–1079)
///
/// X = x_lo | (x_hi << 8)
/// Y = y_lo | (y_hi << 4)
///
/// Returns `[TouchPoint::default(); 2]` silently when the buffer is too short.
fn parse_touch_points(data: &[u8], off: usize) -> [TouchPoint; 2] {
    // Need off+32 .. off+39 inclusive (8 bytes for 2 contacts)
    if data.len() < off + 40 {
        return [TouchPoint::default(); 2];
    }

    let decode = |base: usize| -> TouchPoint {
        let contact  = data[base];
        let x_lo     = data[base + 1] as u16;
        let mid      = data[base + 2];
        let y_hi     = data[base + 3] as u16;

        let active = (contact & 0x80) == 0;
        let x = x_lo | (((mid & 0x0F) as u16) << 8);
        let y = ((mid >> 4) as u16) | (y_hi << 4);

        if active {
            log::debug!("Touchpad contact@{base}: active x={x} y={y}");
        }

        TouchPoint { active, x, y }
    };

    [decode(off + 32), decode(off + 36)]
}

/// Parse a DualSense USB input report.
/// Expected: report ID 0x01 already stripped by hidapi on Windows, so `data` starts at byte 0 = LX.
/// Total read length from hidapi: 64 bytes.
fn parse_dualsense_usb(data: &[u8]) -> Result<UnifiedInput, ParseError> {
    // Detect whether hidapi included the report ID byte.
    // If data[0] == 0x01 and len == 64, report ID is present → offset by 1.
    let off = if data.len() == 64 && data[0] == 0x01 { 1 } else { 0 };
    let min_len = off + 10;
    if data.len() < min_len {
        return Err(ParseError::TooShort { expected: min_len, got: data.len() });
    }
    Ok(UnifiedInput {
        left_stick: (data[off], data[off + 1]),
        right_stick: (data[off + 2], data[off + 3]),
        l2_analog: data[off + 4],
        r2_analog: data[off + 5],
        // off+7 = buttons[0], off+8 = buttons[1], off+9 = buttons[2]
        // (off+6 is a counter)
        buttons: parse_buttons(data[off + 7], data[off + 8], data[off + 9]),
        touchpad: parse_touch_points(data, off),
    })
}

/// Parse a DualSense Bluetooth input report (extended mode, report ID 0x31).
/// hidapi windows-native includes the report ID, so data[0] == 0x31.
/// Then there's a 1-byte BT header, then the same payload as USB.
fn parse_dualsense_bt(data: &[u8]) -> Result<UnifiedInput, ParseError> {
    // Detect report ID presence
    let off = if data.len() >= 2 && data[0] == 0x31 { 2 } else { 1 };
    let min_len = off + 10;
    if data.len() < min_len {
        return Err(ParseError::TooShort { expected: min_len, got: data.len() });
    }
    Ok(UnifiedInput {
        left_stick: (data[off], data[off + 1]),
        right_stick: (data[off + 2], data[off + 3]),
        l2_analog: data[off + 4],
        r2_analog: data[off + 5],
        buttons: parse_buttons(data[off + 7], data[off + 8], data[off + 9]),
        touchpad: parse_touch_points(data, off),
    })
}

/// Parse a DS4 USB input report.
/// hidapi windows-native includes the report ID (0x01).
fn parse_ds4_usb(data: &[u8]) -> Result<UnifiedInput, ParseError> {
    // Detect report ID presence
    let off = if data.len() == 64 && data[0] == 0x01 { 1 } else { 0 };
    let min_len = off + 9;
    if data.len() < min_len {
        return Err(ParseError::TooShort { expected: min_len, got: data.len() });
    }
    Ok(UnifiedInput {
        left_stick: (data[off], data[off + 1]),
        right_stick: (data[off + 2], data[off + 3]),
        // DS4: buttons at bytes 4,5,6 then triggers at 7,8
        buttons: parse_buttons(data[off + 4], data[off + 5], data[off + 6]),
        l2_analog: data[off + 7],
        r2_analog: data[off + 8],
        // DS4 touchpad has a different layout — not yet implemented.
        touchpad: [TouchPoint::default(); 2],
    })
}

/// Parse a DS4 Bluetooth input report (extended mode, report ID 0x11).
/// hidapi windows-native includes the report ID.
/// After report ID there's a 2-byte BT header before the USB-like layout.
fn parse_ds4_bt(data: &[u8]) -> Result<UnifiedInput, ParseError> {
    // Detect report ID presence
    let off = if data.len() >= 3 && data[0] == 0x11 { 3 } else { 2 };
    let min_len = off + 9;
    if data.len() < min_len {
        return Err(ParseError::TooShort { expected: min_len, got: data.len() });
    }
    Ok(UnifiedInput {
        left_stick: (data[off], data[off + 1]),
        right_stick: (data[off + 2], data[off + 3]),
        buttons: parse_buttons(data[off + 4], data[off + 5], data[off + 6]),
        l2_analog: data[off + 7],
        r2_analog: data[off + 8],
        // DS4 touchpad has a different layout — not yet implemented.
        touchpad: [TouchPoint::default(); 2],
    })
}

/// Top-level parse dispatcher.
pub fn parse(
    ct: ControllerType,
    conn: ConnectionType,
    data: &[u8],
) -> Result<UnifiedInput, ParseError> {
    match (ct, conn) {
        (ControllerType::DualSense | ControllerType::DualSenseEdge, ConnectionType::Usb) => {
            parse_dualsense_usb(data)
        }
        (ControllerType::DualSense | ControllerType::DualSenseEdge, ConnectionType::Bluetooth) => {
            parse_dualsense_bt(data)
        }
        (ControllerType::Ds4V1 | ControllerType::Ds4V2, ConnectionType::Usb) => {
            parse_ds4_usb(data)
        }
        (ControllerType::Ds4V1 | ControllerType::Ds4V2, ConnectionType::Bluetooth) => {
            parse_ds4_bt(data)
        }
    }
}

/// Validate CRC on a Bluetooth report. Call this BEFORE parse() with the full
/// raw report bytes (including report ID if present).
pub fn validate_bt_crc(ct: ControllerType, raw: &[u8]) -> bool {
    let _ = ct; // same seed for all BT input reports
    crc32::validate(crc32::SEED_INPUT, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hat_decode() {
        assert_eq!(decode_hat(0), DPad::Up);
        assert_eq!(decode_hat(4), DPad::Down);
        assert_eq!(decode_hat(8), DPad::Neutral);
        assert_eq!(decode_hat(0x0F), DPad::Neutral);
    }

    #[test]
    fn parse_dualsense_usb_basic() {
        let mut data = [0u8; 64];
        data[0] = 128; // LX center
        data[1] = 128; // LY center
        data[2] = 128; // RX center
        data[3] = 128; // RY center
        data[7] = 0x28; // hat=8(neutral) + cross bit (0x20)
        let input = parse_dualsense_usb(&data).unwrap();
        assert_eq!(input.left_stick, (128, 128));
        assert!(input.buttons.cross);
        assert!(!input.buttons.circle);
        assert_eq!(input.buttons.dpad, DPad::Neutral);
    }

    // ── TouchPoint parsing tests ─────────────────────────────────────────

    /// Build a 64-byte DualSense USB report (no report-ID prefix)
    /// with the given touch bytes at off=0.
    fn make_ds_usb_with_touch(contact: u8, x: u16, y: u16) -> [u8; 64] {
        let mut data = [0u8; 64];
        // Minimal valid button bytes
        data[7] = 0x08; // hat=neutral (8)
        // Encode touch point 0 at off+32 = byte 32
        data[32] = contact;
        data[33] = (x & 0xFF) as u8;
        data[34] = ((x >> 8) & 0x0F) as u8 | (((y & 0x0F) as u8) << 4);
        data[35] = ((y >> 4) & 0xFF) as u8;
        // Touch point 1: set bit7=1 (inactive) explicitly
        data[36] = 0x80;
        data
    }

    #[test]
    fn touch_point_active() {
        // contact byte with bit7=0 → active
        let data = make_ds_usb_with_touch(0x00, 100, 200);
        let pts = parse_touch_points(&data, 0);
        assert!(pts[0].active, "bit7=0 should be active");
        assert_eq!(pts[0].x, 100);
        assert_eq!(pts[0].y, 200);
        assert!(!pts[1].active, "second point not set");
    }

    #[test]
    fn touch_point_inactive() {
        // contact byte with bit7=1 → inactive
        let data = make_ds_usb_with_touch(0x80, 500, 300);
        let pts = parse_touch_points(&data, 0);
        assert!(!pts[0].active, "bit7=1 should be inactive");
    }

    #[test]
    fn touch_point_x_max() {
        // X = 1919 = 0x77F → x_lo = 0x7F, x_hi = 0x7
        let data = make_ds_usb_with_touch(0x00, 1919, 0);
        let pts = parse_touch_points(&data, 0);
        assert_eq!(pts[0].x, 1919);
        assert_eq!(pts[0].y, 0);
    }

    #[test]
    fn touch_point_y_max() {
        // Y = 1079 = 0x437 → y_lo = 7, y_hi = 0x43
        let data = make_ds_usb_with_touch(0x00, 0, 1079);
        let pts = parse_touch_points(&data, 0);
        assert_eq!(pts[0].y, 1079);
        assert_eq!(pts[0].x, 0);
    }

    #[test]
    fn touch_points_short_buffer_returns_default() {
        // Buffer smaller than off+40
        let data = [0u8; 35];
        let pts = parse_touch_points(&data, 0);
        assert!(!pts[0].active);
        assert!(!pts[1].active);
    }

    #[test]
    fn parse_dualsense_usb_has_touch_field() {
        // Verify UnifiedInput correctly exposes touch from a USB report
        let mut data = [0u8; 64];
        data[7] = 0x08; // hat neutral
        data[32] = 0x01; // contact active (id=1, bit7=0)
        data[33] = 50;   // x_lo
        data[34] = 0x03; // x_hi=3, y_lo=0
        data[35] = 0;    // y_hi
        let input = parse_dualsense_usb(&data).unwrap();
        assert!(input.touchpad[0].active);
        assert_eq!(input.touchpad[0].x, 50 | (3 << 8)); // = 818
    }

    #[test]
    fn parse_ds4_usb_basic() {
        let mut data = [0u8; 64];
        data[0] = 128;
        data[1] = 128;
        data[4] = 0x40; // circle bit in hat byte
        let input = parse_ds4_usb(&data).unwrap();
        assert!(input.buttons.circle);
        assert_eq!(input.buttons.dpad, DPad::Up); // hat = 0
    }
}
