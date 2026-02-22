# DS4Windows HID Format Research

Reference notes from DS4Windows source code analysis, used to build GamePadCC v2.

## Controller Identification

| Controller | VID | PID | Usage Page | Usage |
|---|---|---|---|---|
| DualSense | 0x054C | 0x0CE6 | 0x01 | 0x05 |
| DualSense Edge | 0x054C | 0x0DF2 | 0x01 | 0x05 |
| DS4 v1 | 0x054C | 0x05C4 | 0x01 | 0x05 |
| DS4 v2 | 0x054C | 0x09CC | 0x01 | 0x05 |

**Important**: On Windows, each USB HID device exposes multiple HID collections. Only the Game Pad collection (usage page 0x01, usage 0x05) supports output reports. Filtering by usage page/usage is critical.

## Connection Type Detection

- **USB**: Device path contains `usb#` or standard HID path patterns
- **Bluetooth**: Device path contains `{00001124` (Bluetooth HID GUID) or `&0005`

## Bluetooth Extended Mode

Before reading full input reports over Bluetooth, the host must activate extended mode by reading a feature report:

- **DualSense**: Feature report 0x05
- **DS4**: Feature report 0x02

Without this, BT reports are basic 8-byte reports with limited button data.

## Input Report Formats

### DualSense USB (Report ID 0x01, 64 bytes)
```
Offset  Size  Description
0       1     Left stick X (0=left, 255=right)
1       1     Left stick Y (0=top, 255=bottom)
2       1     Right stick X
3       1     Right stick Y
4       1     L2 analog trigger
5       1     R2 analog trigger
6       1     Counter (increments each report)
7       1     Buttons[0]: hat switch (low nibble) + face buttons (high nibble)
8       1     Buttons[1]: shoulder/trigger buttons + share/options/stick clicks
9       1     Buttons[2]: PS + touchpad + mute
```

### Button Byte Layout
```
Buttons[0] (byte 7):
  Bits 0-3: Hat switch (0=N, 1=NE, 2=E, 3=SE, 4=S, 5=SW, 6=W, 7=NW, 8+=released)
  Bit 4: Square
  Bit 5: Cross
  Bit 6: Circle
  Bit 7: Triangle

Buttons[1] (byte 8):
  Bit 0: L1
  Bit 1: R1
  Bit 2: L2 (digital)
  Bit 3: R2 (digital)
  Bit 4: Share/Create
  Bit 5: Options
  Bit 6: L3
  Bit 7: R3

Buttons[2] (byte 9):
  Bit 0: PS button
  Bit 1: Touchpad click
  Bit 2: Mute (DualSense only)
```

### DualSense Bluetooth (Report ID 0x31, 78 bytes)
Same layout as USB but offset by +1 byte (BT header). Last 4 bytes are CRC-32.

### DS4 USB (Report ID 0x01, 64 bytes)
Same stick layout. Buttons at bytes 4-6, triggers at bytes 7-8.

### DS4 Bluetooth (Report ID 0x11, 78 bytes)
Same as USB but offset by +2 bytes. Last 4 bytes are CRC-32.

## Output Report Formats

### DualSense USB (Report ID 0x02, 48 bytes)
```
Offset  Size  Description
0       1     Report ID (0x02)
1       1     Valid flags 0 (bit 0=rumble, bit 1=right trigger, bit 2=left trigger)
2       1     Valid flags 1 (bit 2=lightbar)
3       1     Right motor rumble (0-255)
4       1     Left motor rumble (0-255)
44      1     Lightbar Red
45      1     Lightbar Green
46      1     Lightbar Blue
```

### DualSense Bluetooth (Report ID 0x31, 78 bytes)
```
Offset  Size  Description
0       1     Report ID (0x31)
1       1     Sequence number (high nibble, 0-15)
2       1     Tag (0x10)
3-47    45    Same as USB bytes 1-47 (offset by +2)
74-77   4     CRC-32 (seed 0xA2)
```

### DS4 USB (Report ID 0x05, 32 bytes)
```
Offset  Size  Description
0       1     Report ID (0x05)
1       1     Flags (0x07 = rumble + lightbar)
4       1     Right motor rumble
5       1     Left motor rumble
6       1     Lightbar Red
7       1     Lightbar Green
8       1     Lightbar Blue
```

### DS4 Bluetooth (Report ID 0x11, 79 bytes)
```
Offset  Size  Description
0       1     Report ID (0x11)
1       1     0x80 (HID output flag)
3       1     0xF7 (enable rumble + lightbar + flash)
6       1     Right motor rumble
7       1     Left motor rumble
8       1     Lightbar Red
9       1     Lightbar Green
10      1     Lightbar Blue
75-78   4     CRC-32 (seed 0xA2)
```

## CRC-32

Both DualSense and DS4 Bluetooth reports use CRC-32 (polynomial 0xEDB88320):

- **Input reports**: Seed byte 0xA1 prepended before computing CRC
- **Output reports**: Seed byte 0xA2 prepended before computing CRC
- CRC is stored as little-endian u32 in the last 4 bytes of the report

## Key DS4Windows Patterns

1. **HidHide**: DS4Windows uses HidHide kernel driver for exclusive access. GamePadCC avoids this dependency — users should disable Steam Input instead.
2. **Report size matching**: Output report size MUST match the HID descriptor's OutputReportByteLength exactly, or Windows returns ERROR_INVALID_PARAMETER.
3. **Non-blocking reads**: Use short timeouts (5ms) to avoid blocking the event loop.
4. **Write errors are non-fatal**: The controller may occasionally reject writes; log and continue.

## References

- DS4Windows source: https://github.com/ds4windowsapp/DS4Windows
- DualSense HID format: https://controllers.fandom.com/wiki/Sony_DualSense
