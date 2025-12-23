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

Target a specific display:

```sh
cargo run -- set-input --display 1 26
```

Diagnostics:

```sh
cargo run -- doctor
```

## Notes

- This PoC uses `m1ddc` on macOS, which can **set** input but does not reliably **read** the current raw VCP `0x60` value.
- Cargo build artifacts are redirected via `.cargo/config.toml` to `/tmp/monitorctl-target` because the workspace folder name contains `:` (which breaks default macOS runtime linker path handling).
