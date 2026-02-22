/// CRC-32 with seed byte for Bluetooth HID reports.
///
/// Both DualSense and DS4 Bluetooth reports use CRC-32 with a seed byte
/// prepended to the data before computing:
/// - Input reports:  seed = 0xA1
/// - Output reports: seed = 0xA2

const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

pub const SEED_INPUT: u8 = 0xA1;
pub const SEED_OUTPUT: u8 = 0xA2;

/// Compute CRC-32 over `data` with a seed byte prepended.
pub fn calc(seed: u8, data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    // Feed the seed byte first
    crc = (crc >> 8) ^ CRC32_TABLE[((crc as u8) ^ seed) as usize];
    // Then the actual data
    for &b in data {
        crc = (crc >> 8) ^ CRC32_TABLE[((crc as u8) ^ b) as usize];
    }
    crc ^ 0xFFFF_FFFF
}

/// Validate that the last 4 bytes of `report` match the CRC-32 of the preceding bytes.
pub fn validate(seed: u8, report: &[u8]) -> bool {
    if report.len() < 4 {
        return false;
    }
    let (data, crc_bytes) = report.split_at(report.len() - 4);
    let expected = u32::from_le_bytes([crc_bytes[0], crc_bytes[1], crc_bytes[2], crc_bytes[3]]);
    calc(seed, data) == expected
}

/// Append CRC-32 (little-endian) to the given buffer at `crc_offset`.
/// The CRC is computed over `report[0..crc_offset]` with the seed.
pub fn stamp(seed: u8, report: &mut [u8], crc_offset: usize) {
    let crc = calc(seed, &report[..crc_offset]);
    let bytes = crc.to_le_bytes();
    report[crc_offset..crc_offset + 4].copy_from_slice(&bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_value() {
        // CRC-32 of "123456789" (no seed) should be 0xCBF43926
        let data = b"123456789";
        let mut crc: u32 = 0xFFFF_FFFF;
        for &b in data.iter() {
            crc = (crc >> 8) ^ CRC32_TABLE[((crc as u8) ^ b) as usize];
        }
        crc ^= 0xFFFF_FFFF;
        assert_eq!(crc, 0xCBF4_3926);
    }

    #[test]
    fn stamp_and_validate_roundtrip() {
        let mut buf = [0u8; 10];
        buf[0] = 0x31; // fake report ID
        buf[1] = 0x02;
        buf[2] = 0xFF;
        let crc_offset = 6;
        stamp(SEED_OUTPUT, &mut buf, crc_offset);
        assert!(validate(SEED_OUTPUT, &buf[..crc_offset + 4]));
    }

    #[test]
    fn validate_detects_corruption() {
        let mut buf = [0u8; 10];
        buf[0] = 0x31;
        let crc_offset = 6;
        stamp(SEED_OUTPUT, &mut buf, crc_offset);
        buf[1] = 0xFF; // corrupt data
        assert!(!validate(SEED_OUTPUT, &buf[..crc_offset + 4]));
    }
}
