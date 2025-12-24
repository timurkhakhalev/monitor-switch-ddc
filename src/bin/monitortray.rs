#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use anyhow::Result;

#[cfg(target_os = "windows")]
fn main() -> Result<()> {
    monitorctl::tray::platform::windows::run()
}

#[cfg(target_os = "macos")]
fn main() -> Result<()> {
    monitorctl::tray::platform::macos::run()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn main() -> Result<()> {
    anyhow::bail!("monitortray is only supported on Windows and macOS");
}
