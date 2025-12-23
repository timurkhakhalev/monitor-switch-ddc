# Progress

## Done

- Verified monitor input switching values on ASUS ROG Strix XG27ACS:
  - USB‑C = raw VCP `0x60` value `26`
  - DP1 = raw VCP `0x60` value `15` (confirmed switching from macOS)
- Implemented a Rust CLI PoC: `monitorctl`
  - `list` (macOS + Windows)
  - `set-input <value>` (macOS + Windows)
  - `doctor` (macOS + Windows)
  - `get-input` (Windows only; reads raw VCP `0x60`)
- macOS backend (PoC): uses `m1ddc` (`brew install m1ddc`)
- Windows backend (PoC): uses Dxva2 High-Level Monitor Configuration API (DDC/CI wrapper)
- Added optional JSON config + input presets:
  - `monitorctl config-path` prints where config is loaded from
  - `set-input` accepts either a number or a preset name (e.g. `dp1`)
  - `get-input` / `set-input` can use a configured `default_display`
- Windows display selector now supports `--display name:<substring>` (matches monitor description)
- Documented macOS build workaround for `:` in the repo folder name (use `CARGO_TARGET_DIR=/tmp/monitorctl-target`).

## How to run (quick)

macOS:

- `cargo run -- doctor`
- `cargo run -- list`
- `cargo run -- set-input 26` (USB‑C)
- `cargo run -- set-input 15` (DP1)

Windows:

- `cargo build --release`
- `.\target\release\monitorctl.exe doctor`
- `.\target\release\monitorctl.exe list`
- `.\target\release\monitorctl.exe get-input --display 1`
- `.\target\release\monitorctl.exe set-input --display 1 15` (DP1)
- `.\target\release\monitorctl.exe set-input --display 1 26` (USB‑C, only if the Windows machine can still reach DDC when it’s not the active input)

## Open questions / risks

- DDC/CI often works only over the currently active video link. That’s OK for your workflow (“each machine switches away from itself”), but it can prevent switching *to* yourself when you’re not active.
- macOS PoC backend is currently “set-only” (via `m1ddc`); it doesn’t reliably expose reading raw `0x60`.
- Monitor selection on Windows is currently by 1-based index from `list` (no EDID/serial matching yet).

## Next steps (recommended order)

1) Validate on Windows:
   - Run `monitorctl.exe list`, pick the right monitor index.
   - Confirm `set-input 15` works reliably.
   - Use `get-input` to verify what the monitor reports as current `0x60` value.
2) Add configuration:
   - Persist per-monitor input values and selection (EDID/serial matching preferred).
3) Add tray UI (Tauri):
   - macOS menu bar: “Switch to USB‑C”, “Switch to DP1”.
   - Windows tray: “Switch to DP1”, “Switch to USB‑C” (optional).
4) Replace macOS backend:
   - Move from `m1ddc` subprocess to native IOKit/DDC (optional, but preferable long-term).
5) Packaging:
   - Windows: signed exe/msi (optional).
   - macOS: codesign + notarization (if you want a polished app).

