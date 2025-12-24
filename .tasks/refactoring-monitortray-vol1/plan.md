# Refactor `monitortray.rs` — 1‑day SWE task list

Scope: one engineer, one day. No migrations or backwards compatibility required. Refactor for readability/maintainability; OK to change command IDs, menu construction, and internal APIs as long as the tray app still works on macOS + Windows.

Primary goal: remove the “God `State`” by separating:
- platform UI glue (Win32/Cocoa)
- platform-agnostic tray logic (model + command dispatch + menu spec)

Constraints:
- No new dependencies.
- Keep cross‑platform behavior (macOS + Windows) working.

---

## Deliverables (end of day)
- `src/bin/monitortray.rs` is a thin OS dispatcher (no big `unsafe` blocks).
- Shared tray logic lives under `src/tray/` and is used by both platforms.
- Windows tray code is isolated from macOS tray code (separate files/modules).
- `cargo fmt --check`, `cargo clippy --all-targets --all-features`, `cargo test` pass.

---

## Task breakdown (do in order)

### 1) Create shared “command + menu model” modules
- Add `src/tray/commands.rs`
  - Define command IDs/constants (feel free to redesign IDs if it simplifies code).
  - Provide a typed command enum, e.g. `enum Command { Input(u16), Reload, Quit, ToggleStartup, EditConfig, OpenConfigFolder }`.
  - Provide a decoder `fn decode(cmd_id: u16, inputs: &InputsMap) -> Option<Command>`.
- Add `src/tray/menu.rs`
  - Define a pure menu spec rendered by both platforms:
    - `MenuSpec`, `MenuItem` (`Header`, `Separator`, `Action { id, title, checked, enabled }`).
- Add `src/tray/model.rs`
  - Define `TrayModel` containing platform-agnostic state:
    - `inputs`, `display_selector`, `backend`, `last_error`
    - startup state flag (`start_enabled`)
  - Implement:
    - `new() -> Result<Self>`
    - `menu_spec(&self) -> MenuSpec`
    - `handle(cmd: Command, startup: &dyn StartupManager) -> Result<ModelUpdate>` where `ModelUpdate` tells UI whether to refresh menu/tooltip/quit.
- Add `src/tray/startup.rs`
  - Define `trait StartupManager { fn is_enabled(&self) -> Result<bool>; fn set_enabled(&self, enabled: bool) -> Result<()>; }`

Acceptance:
- Shared modules compile (even if platform code not yet migrated).

### 2) Move platform tray code into separate files
Pick one of these layouts and implement it (either is fine, no compatibility requirement):
- Option A (library ownership): `src/tray/platform/windows.rs` and `src/tray/platform/macos.rs`
- Option B (binary ownership): `src/bin/monitortray/windows.rs` and `src/bin/monitortray/macos.rs`

Acceptance:
- `src/bin/monitortray.rs` just calls `windows::run()` / `macos::run()`.

### 3) Windows: replace God `State` with `{ui, model, startup}`
- In Windows module:
  - Keep Win32 window lifecycle and `wndproc` in platform code.
  - Introduce:
    - `struct WinTrayUi { hwnd, tray, menu, ... }` (Win32-only fields)
    - `struct WinStartupManager` implementing `StartupManager` using current registry code.
    - `struct WinApp { ui: WinTrayUi, model: TrayModel, startup: WinStartupManager }`
  - Implement UI helpers:
    - `rebuild_menu(&mut self, spec: &MenuSpec) -> Result<()>`
    - `set_tooltip(&mut self, text: &str)` (best-effort)
  - Route menu clicks:
    - `cmd_id -> Command` (via shared decoder)
    - `model.handle(...) -> ModelUpdate`
    - apply `ModelUpdate` (rebuild menu, update tooltip, quit)

Acceptance:
- Windows build works; tray opens; selecting an input calls backend; reload + toggle startup + edit/open config still functional (manual smoke check).

### 4) macOS: replace God `State` with `{ui, model, startup}`
- In macOS module:
  - Keep ObjC target class + callback glue in platform code.
  - Introduce:
    - `struct MacTrayUi { status_item, menu, target, ... }`
    - `struct MacStartupManager` implementing `StartupManager` using current LaunchAgent code.
    - `struct MacApp { ui: MacTrayUi, model: TrayModel, startup: MacStartupManager }`
  - Implement UI helpers:
    - `rebuild_menu(&mut self, spec: &MenuSpec) -> Result<()>`
    - `set_tooltip(&mut self, text: &str)` (best-effort)
  - Route menu events the same way as Windows (typed command → model update → UI apply).

Acceptance:
- macOS build works; status item shows; menu actions function (manual smoke check).

### 5) Delete old inline logic + tighten error handling
- Remove duplicated:
  - per-platform `CMD_*` constants
  - per-platform `load_display_and_inputs` (replace with one shared loader in `TrayModel::new()` / `TrayModel::reload()`)
- Keep error messages actionable via `anyhow::Context` where it helps (especially backend selection, config operations, startup manager operations).
- Ensure “error → tooltip” behavior remains clear and non-promising.

Acceptance:
- `src/bin/monitortray.rs` no longer contains two huge nested modules.

### 6) Run validations and do a quick manual smoke check
- Run:
  - `cargo fmt --check`
  - `cargo clippy --all-targets --all-features`
  - `cargo test`
- Manual:
  - macOS: launch tray, open menu, click an input, reload config, toggle start at login.
  - Windows: launch tray, open menu, click an input, reload config, toggle start with Windows.

Acceptance:
- All commands pass; tray usable on both OSes.

---

## Suggested “definition of done” checklist
- [ ] `monitortray.rs` is a small dispatcher.
- [ ] Shared `TrayModel` has no platform types.
- [ ] Both platforms render from `MenuSpec`.
- [ ] Both platforms route events through `Command` + `ModelUpdate`.
- [ ] `fmt`/`clippy`/`test` green.
