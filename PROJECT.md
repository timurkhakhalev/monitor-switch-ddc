# Monitor Input Switcher (DDC/CI) — Project Notes

Personal tray app to switch an external monitor’s active input using DDC/CI (VESA MCCS).

## Target setup (known)

- Monitor: ASUS ROG Strix XG27ACS
- macOS: MacBook Pro 14" (M4 Pro, late 2024), macOS 15, connected directly via the monitor’s USB‑C port
- Windows: separate machine connected via DisplayPort (DP)
- Confirmed behavior (Windows via ClickMonitorDDC): selecting input value `26` switches the monitor to USB‑C

## Goal

- macOS menu bar action: “Switch monitor to USB‑C” (writes VCP `0x60 = 26`)
- Windows tray action: “Switch monitor to DisplayPort1” (writes VCP `0x60 = <dp1_value>`)
- Optional: HDMI1 support (same mechanism)

## Non-goals

- “Switch-back” guarantee from the same machine (once you switch away, the initiating machine may lose the DDC/CI channel)
- Automatic input switching rules, presence detection, etc. (can be added later if desired)
- Comprehensive monitor control suite (brightness/contrast/etc.)

## Technical background

- DDC/CI is an I²C-based control channel typically carried over the active video link.
- Input switching is usually done via VESA MCCS “Input Source”:
  - VCP code: `0x60`
  - Operation: **Set VCP** to a numeric input value (model/firmware dependent in practice)

For this monitor we already know:

- USB‑C input value: `26` (for VCP `0x60`)

We still need to learn:

- DisplayPort1 input value: `dp1_value`
- HDMI1 input value (optional): `hdmi1_value`

## Cross-platform approach

Use a shared core with OS-specific backends.

- UI shell: Tauri (system tray on Windows, menu bar on macOS)
- Core/backends: Rust

Rationale:

- Tauri is a small, popular tray-capable cross-platform wrapper.
- Rust is well-suited for calling native DDC/CI APIs on Windows and IOKit/I²C on macOS.

## Architecture

- `core` (Rust)
  - CLI commands: `list`, `get-input`, `set-input`
  - Model: monitor identity (EDID manufacturer/model/serial where available), per-monitor config, retries/timeouts
- `backend_windows` (Rust)
  - Enumerate physical monitors
  - Read VCP features and write VCP `0x60`
- `backend_macos` (Rust)
  - Enumerate external displays + obtain stable identifiers (EDID/serial if possible)
  - Send DDC/CI “Set VCP Feature” messages over the display’s I²C/DDC interface
- `ui` (Tauri)
  - Tray/menu actions calling the Rust core

## Windows backend plan (Dxva2)

Use Microsoft’s High-Level Monitor Configuration API (DDC/CI wrapper), typically via:

- Enumerate monitors (`EnumDisplayMonitors`, `GetPhysicalMonitorsFromHMONITOR`)
- Read current VCP feature (`GetVCPFeatureAndVCPFeatureReply`)
- Set VCP feature (`SetVCPFeature`)

This aligns with what ClickMonitorDDC relies on and should be reliable.

## macOS backend plan (IOKit + DDC/CI)

macOS has no official “set VCP feature” public API; implementations typically:

- Enumerate displays via CoreGraphics/IOKit
- Locate the IOKit service for the display
- Open the DDC/I²C interface and send MCCS frames (including Set VCP `0x60`)

Notes/risks:

- DDC/CI passthrough can break on some hubs/docks/adapters (current setup is direct USB‑C, which is ideal).
- Some monitors stop responding to DDC when asleep or immediately after switching inputs; include retries and clear error reporting.

## Minimal UX (PoC)

macOS menu bar:

- `Switch to USB‑C`
- (Optional) `Switch to HDMI1`
- `Quit`

Windows tray:

- `Switch to DisplayPort1`
- (Optional) `Switch to HDMI1`
- `Quit`

If multiple external monitors are detected, show a “Target monitor” submenu (by model/serial), or default to the first matching XG27ACS EDID.

## Configuration

Store a small per-monitor config (JSON/TOML), keyed by EDID identity when possible.

Example:

```json
{
  "monitors": [
    {
      "match": { "model": "XG27ACS" },
      "inputs": { "usb_c": 26, "dp1": 15, "hdmi1": 17 }
    }
  ]
}
```

(Numbers above are placeholders except `usb_c: 26`.)

## How we’ll determine `dp1_value` (and `hdmi1_value`)

Since ClickMonitorDDC doesn’t show the raw value, we’ll do one of:

1) Use our own PoC CLI on Windows to:
   - Switch to DP1 using an existing tool
   - Run `get-input` to read VCP `0x60` and record the numeric value

2) Use an alternative Windows monitor tool that displays “Input Source (0x60)” with the raw current value (then copy that value into config).

Once values are known, the tray actions are just “Set VCP `0x60` to <value>”.

## Milestones

1) Record input values:
   - Confirm `dp1_value`
   - Confirm `hdmi1_value` (optional)
2) CLI PoC:
   - List monitors and show identity
   - `get-input` (VCP `0x60`)
   - `set-input` (VCP `0x60`)
3) Tray PoC:
   - macOS menu bar app calls `set-input 26`
   - Windows tray app calls `set-input dp1_value`
4) Packaging:
   - Per-OS release builds, simple autostart instructions

