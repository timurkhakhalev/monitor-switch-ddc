# monitorctl (PoC)

Minimal macOS-first proof-of-concept for switching a monitor input via DDC/CI.

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

Read current raw input value (Windows only in this PoC):

```powershell
monitorctl.exe get-input --display 1
```

## Notes

- This PoC uses `m1ddc` on macOS, which can **set** input but does not reliably **read** the current raw VCP `0x60` value.
- Cargo build artifacts are redirected via `.cargo/config.toml` to `/tmp/monitorctl-target` because the workspace folder name contains `:` (which breaks default macOS runtime linker path handling).

## Windows (PoC)

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

### Config (optional)

`monitorctl` can also map friendly preset names (like `dp1`, `usb_c`) to raw VCP `0x60` values.

- See the path it will use: `monitorctl.exe config-path`
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
