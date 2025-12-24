# monitorctl

Monitor input switcher via DDC/CI (VESA MCCS) for macOS and Windows.

This repo contains:

- `monitorctl`: CLI to list displays and set input (VCP `0x60`)
- `monitortray`: tray/menu-bar app that exposes configured input presets

## Prereqs (macOS)

- Xcode Command Line Tools: `xcode-select --install`
- Rust: install via `rustup`
- DDC helper: `brew install m1ddc`

## Build & run

List detected external displays:

```sh
cargo run -- list
```

Switch input by raw VCP `0x60` value (XG27ACS USB‑C is `26`):

```sh
cargo run -- set-input 26
```

On your XG27ACS, DP1 appears to be `15` (common MCCS value):

```sh
cargo run -- set-input 15
```

Target a specific display:

```sh
cargo run -- set-input --display 1 26
```

Diagnostics:

```sh
cargo run -- doctor
```

Read current raw input value (Windows-only at the moment):

```powershell
monitorctl.exe get-input --display 1
```

## Notes

- On macOS this currently uses `m1ddc`, which can **set** input but does not reliably **read** the current raw VCP `0x60` value on all monitors.
- DDC/CI commonly works only over the currently active video link; once you switch away, the initiating machine may lose the control channel.

## Windows

On Windows, `monitorctl` uses the Dxva2 High-Level Monitor Configuration API (DDC/CI wrapper).

Build on Windows:

```powershell
cargo build
```

Commands are the same:

```powershell
monitorctl.exe doctor
monitorctl.exe list
monitorctl.exe set-input --display 1 15
```

## Windows tray app

Build + run:

```powershell
cargo build --bin monitortray
.\target\debug\monitortray.exe
```

Click the tray icon (left or right click) to pick an input preset.

### Tray config (recommended)

- Show config path: `monitorctl config-path`
- Create JSON config at that path, e.g.:

```json
{
  "start_with_windows": true,
  "default_display": "name:XG27ACS",
  "inputs": { "dp1": 15, "usb_c": 26 }
}
```

Then `monitortray` shows `dp1` / `usb_c` in the menu (and you can add more presets).
If you leave `inputs` empty, the tray defaults to `dp1` (15) and `usb_c` (26).

`monitortray` menu actions:

- Start with Windows: toggles user startup (HKCU Run key) and updates `start_with_windows` in the config.
- Edit config: opens the config file in your default editor (creates a config file with default inputs if missing).
- Open config folder: opens the config directory.
- Reload config: re-reads the config and rebuilds the tray menu.

## macOS tray app

Build + run:

```sh
cargo build --bin monitortray
./target/debug/monitortray
```

The app shows a menu bar item called `monitorctl`; click it to pick an input preset.

`monitortray` menu actions:

- Start at login: toggles a per-user LaunchAgent (`~/Library/LaunchAgents/com.monitorctl.monitorctl.plist`) and updates `start_with_windows` in the config.
- Edit config: opens the config file in your default editor (creates a config file with default inputs if missing).
- Open config folder: opens the config directory.
- Reload config: re-reads the config and rebuilds the tray menu.

### macOS `.app` bundle (recommended)

```sh
./scripts/build-macos-app.sh
open "dist/monitorctl.app"
```

### Config (optional)

`monitorctl` can also map friendly preset names (like `dp1`, `usb_c`) to raw VCP `0x60` values.

- See the path it will use: `monitorctl config-path`
- Create a JSON file at that path, e.g.:

```json
{
  "default_display": "name:XG27ACS",
  "inputs": { "dp1": 15, "usb_c": 26 }
}
```

Then you can run:

```powershell
monitorctl.exe set-input dp1
monitorctl.exe set-input usb_c
monitorctl.exe get-input
```

## Contributing

See `CONTRIBUTING.md`.

## License

MIT OR Apache-2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.
