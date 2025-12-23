# Agent notes (monitorctl / monitortray)

## Scope

These instructions apply to the whole repo.

## Goals

- Keep changes small and focused.
- Prefer fixing root causes over workarounds.
- Maintain cross-platform behavior (macOS + Windows).

## Code style

- Run `cargo fmt` after Rust changes.
- Prefer `anyhow::Context` for actionable errors.
- Avoid new dependencies unless clearly justified.

## Validation

- Minimum: `cargo fmt --check`, `cargo clippy --all-targets --all-features`, `cargo test`.
- macOS packaging: `./scripts/build-macos-app.sh` (produces `dist/monitorctl.app`).

## Platform notes

- macOS backend currently shells out to `m1ddc` (external dependency); avoid breaking CLI UX if `m1ddc` is missing.
- DDC/CI can stop working once the video link is inactive; keep messaging clear and non-promising.

## Repo hygiene

- Don’t commit build artifacts (`dist/`, `target/`).
- When touching docs, keep `README.md` authoritative and concise.
