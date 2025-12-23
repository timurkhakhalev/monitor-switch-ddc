use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, bail, Context, Result};

use super::{DisplayInfo, DisplayListReport, DoctorReport};

pub struct M1DdcBackend;

impl M1DdcBackend {
    pub fn new() -> Self {
        Self
    }

    fn resolve_m1ddc_path() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("MONITORCTL_M1DDC_PATH") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }

        // GUI apps / LaunchAgents often have a minimal PATH. Prefer common Homebrew paths.
        for candidate in ["/opt/homebrew/bin/m1ddc", "/usr/local/bin/m1ddc"] {
            let candidate = Path::new(candidate);
            if candidate.is_file() {
                return Some(candidate.to_path_buf());
            }
        }

        // Fall back to PATH lookup (and add common Homebrew locations just in case).
        let out = Command::new("sh")
            .arg("-lc")
            .arg("PATH=/opt/homebrew/bin:/usr/local/bin:$PATH command -v m1ddc")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8(out.stdout).ok()?;
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let path = PathBuf::from(s);
        path.is_file().then_some(path)
    }

    fn m1ddc_path(&self) -> Result<PathBuf> {
        Self::resolve_m1ddc_path().ok_or_else(|| {
            anyhow!(
                "Missing dependency: `m1ddc`.\n\
Install: `brew install m1ddc`\n\
If you launch the tray app from Finder/LaunchAgent, PATH may not include Homebrew.\n\
Expected locations: /opt/homebrew/bin/m1ddc or /usr/local/bin/m1ddc\n\
Override with: MONITORCTL_M1DDC_PATH=/path/to/m1ddc"
            )
        })
    }

    fn run_m1ddc(&self, args: &[&str]) -> Result<String> {
        let m1ddc = self.m1ddc_path()?;
        let out = Command::new(&m1ddc)
            .args(args)
            .output()
            .with_context(|| format!("running {} {}", m1ddc.display(), args.join(" ")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            bail!(
                "m1ddc failed (exit={}):\nstdout:\n{}\nstderr:\n{}",
                out.status,
                stdout.trim(),
                stderr.trim()
            );
        }
        String::from_utf8(out.stdout).context("m1ddc output was not UTF-8")
    }
}

impl super::Backend for M1DdcBackend {
    fn list_displays(&self) -> Result<DisplayListReport> {
        let raw = self.run_m1ddc(&["display", "list", "detailed"])?;
        let mut displays = Vec::new();

        // Example:
        // [1] XG27ACS (UUID)
        //  - Product name:  XG27ACS
        //  - System UUID:   UUID
        // ...
        for line in raw.lines() {
            let line = line.trim_end();
            if let Some(rest) = line.strip_prefix('[') {
                if let Some((idx_str, after_idx)) = rest.split_once(']') {
                    let index: u32 = idx_str.trim().parse().ok().unwrap_or(0);
                    let mut product_name: Option<String> = None;
                    let mut system_uuid: Option<String> = None;

                    // Try to extract "Name (UUID)" from header line.
                    let header = after_idx.trim();
                    if !header.is_empty() {
                        if let Some((name, uuid_part)) = header.rsplit_once('(') {
                            let name = name.trim();
                            let uuid = uuid_part.trim().trim_end_matches(')');
                            if !name.is_empty() {
                                product_name = Some(name.to_string());
                            }
                            if !uuid.is_empty() {
                                system_uuid = Some(uuid.to_string());
                            }
                        } else {
                            product_name = Some(header.to_string());
                        }
                    }

                    displays.push(DisplayInfo {
                        index,
                        product_name,
                        system_uuid,
                    });
                }
            }
        }

        if displays.is_empty() {
            return Err(anyhow!(
                "No displays parsed from m1ddc output. Raw output:\n{}",
                raw.trim()
            ));
        }

        Ok(DisplayListReport {
            displays,
            raw: Some(raw),
        })
    }

    fn set_input(&self, display_selector: &str, value: u16) -> Result<()> {
        // `m1ddc display <selector> set input <n>`
        let value_str = value.to_string();
        let _ = self.run_m1ddc(&["display", display_selector, "set", "input", &value_str])?;
        Ok(())
    }

    fn get_input(&self, _display_selector: &str) -> Result<u16> {
        bail!("get-input is not implemented for the macOS m1ddc backend (m1ddc does not reliably expose raw VCP 0x60 reads).");
    }

    fn doctor(&self) -> Result<DoctorReport> {
        let mut messages = Vec::new();

        let m1ddc = match self.m1ddc_path() {
            Ok(p) => p,
            Err(e) => {
                messages.push(e.to_string());
                return Ok(DoctorReport {
                    ok: false,
                    message: messages.join("\n"),
                });
            }
        };

        messages.push(format!("m1ddc: OK ({})", m1ddc.display()));

        match self.run_m1ddc(&["display", "list"]) {
            Ok(out) => {
                let out = out.trim();
                if out.is_empty() {
                    messages.push("m1ddc ran but returned no displays.".to_string());
                    return Ok(DoctorReport {
                        ok: false,
                        message: messages.join("\n"),
                    });
                }
                messages.push(format!("Detected displays:\n{}", out));
                messages.push(
                    "Note: m1ddc can set input, but does not expose reading raw VCP 0x60 on all monitors."
                        .to_string(),
                );
                Ok(DoctorReport {
                    ok: true,
                    message: messages.join("\n\n"),
                })
            }
            Err(e) => Ok(DoctorReport {
                ok: false,
                message: format!("m1ddc failed to list displays: {e}"),
            }),
        }
    }
}
