# Contributing

Thanks for helping improve `monitorctl`.

## Development setup

- Install Rust via `rustup`
- macOS: install `m1ddc` with `brew install m1ddc` (used for input switching today)

## Common commands

```sh
cargo build
cargo test
cargo fmt
cargo clippy --all-targets --all-features
```

## Reporting issues

When reporting a bug, please include:

- OS version (macOS/Windows)
- Monitor model(s)
- Connection type (USB‑C/DP/HDMI, direct vs dock/hub)
- Output of `monitorctl doctor`
- Steps to reproduce

## Pull requests

- Keep PRs focused and small when possible
- Prefer adding a short note to `README.md` when behavior changes
- Avoid adding new dependencies unless there’s a clear need
