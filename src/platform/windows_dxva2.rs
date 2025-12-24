use anyhow::{anyhow, bail, Context, Result};

use super::{DisplayInfo, DisplayListReport, DoctorReport};

#[cfg(target_os = "windows")]
mod win {
    use windows::{
        core::Error,
        Win32::{
            Devices::Display::{
                DestroyPhysicalMonitors, GetNumberOfPhysicalMonitorsFromHMONITOR,
                GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, SetVCPFeature,
                MC_VCP_CODE_TYPE,
            },
            Foundation::{LPARAM, RECT},
            Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR},
        },
    };

    pub use windows::Win32::Devices::Display::PHYSICAL_MONITOR;

    pub unsafe fn enum_physical_monitors() -> windows::core::Result<Vec<PHYSICAL_MONITOR>> {
        let mut all: Vec<PHYSICAL_MONITOR> = Vec::new();

        unsafe extern "system" fn cb(
            hmonitor: HMONITOR,
            _hdc: HDC,
            _rc: *mut RECT,
            lparam: LPARAM,
        ) -> windows::core::BOOL {
            let vec_ptr = lparam.0 as *mut Vec<PHYSICAL_MONITOR>;
            let vec = unsafe { &mut *vec_ptr };

            let mut count: u32 = 0;
            if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count) }.is_err() {
                return windows::core::BOOL(1);
            }
            if count == 0 {
                return windows::core::BOOL(1);
            }

            let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
            if unsafe { GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut monitors) }.is_ok() {
                vec.extend(monitors);
            }

            windows::core::BOOL(1)
        }

        let vec_ptr = &mut all as *mut Vec<PHYSICAL_MONITOR>;
        let ok = unsafe { EnumDisplayMonitors(None, None, Some(cb), LPARAM(vec_ptr as isize)) };
        if !ok.as_bool() {
            return Err(Error::from_thread());
        }
        Ok(all)
    }

    pub unsafe fn destroy(monitors: &mut [PHYSICAL_MONITOR]) {
        let _ = unsafe { DestroyPhysicalMonitors(monitors) };
    }

    pub fn wide_to_string(w: &[u16]) -> String {
        let nul = w.iter().position(|c| *c == 0).unwrap_or(w.len());
        String::from_utf16_lossy(&w[..nul])
    }

    pub fn set_vcp(mon: &PHYSICAL_MONITOR, code: u8, value: u32) -> windows::core::Result<()> {
        let ok = unsafe { SetVCPFeature(mon.hPhysicalMonitor, code, value) };
        if ok == 0 {
            return Err(Error::from_thread());
        }
        Ok(())
    }

    pub fn get_vcp(mon: &PHYSICAL_MONITOR, code: u8) -> windows::core::Result<(u32, u32)> {
        let mut vcp_type = MC_VCP_CODE_TYPE(0);
        let mut current: u32 = 0;
        let mut maximum: u32 = 0;
        let ok = unsafe {
            GetVCPFeatureAndVCPFeatureReply(
                mon.hPhysicalMonitor,
                code,
                Some(&mut vcp_type),
                &mut current,
                Some(&mut maximum),
            )
        };
        if ok == 0 {
            return Err(Error::from_thread());
        }
        Ok((current, maximum))
    }

    pub fn monitor_desc(mon: &PHYSICAL_MONITOR) -> String {
        // szPhysicalMonitorDescription is [u16; 128] on a packed struct.
        let desc: [u16; 128] = unsafe {
            std::ptr::read_unaligned(std::ptr::addr_of!(mon.szPhysicalMonitorDescription))
        };
        wide_to_string(&desc)
    }
}

pub struct WindowsDxva2Backend;

impl WindowsDxva2Backend {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_os = "windows")]
struct MonitorList {
    mons: Vec<win::PHYSICAL_MONITOR>,
}

#[cfg(target_os = "windows")]
impl MonitorList {
    fn new() -> Result<Self> {
        let mons =
            unsafe { win::enum_physical_monitors().context("enumerating physical monitors")? };
        Ok(Self { mons })
    }

    fn is_empty(&self) -> bool {
        self.mons.is_empty()
    }
}

#[cfg(target_os = "windows")]
impl std::ops::Deref for MonitorList {
    type Target = [win::PHYSICAL_MONITOR];

    fn deref(&self) -> &Self::Target {
        &self.mons
    }
}

#[cfg(target_os = "windows")]
impl Drop for MonitorList {
    fn drop(&mut self) {
        if !self.mons.is_empty() {
            unsafe { win::destroy(&mut self.mons) };
        }
    }
}

impl super::Backend for WindowsDxva2Backend {
    fn list_displays(&self) -> Result<DisplayListReport> {
        #[cfg(not(target_os = "windows"))]
        {
            bail!("Windows backend can only run on Windows.");
        }

        #[cfg(target_os = "windows")]
        unsafe {
            let mut mons =
                win::enum_physical_monitors().context("enumerating physical monitors")?;
            if mons.is_empty() {
                return Err(anyhow!("No physical monitors found via Dxva2."));
            }

            let displays = mons
                .iter()
                .enumerate()
                .map(|(i, m)| DisplayInfo {
                    index: (i + 1) as u32,
                    product_name: Some(win::monitor_desc(m)),
                    system_uuid: None,
                })
                .collect::<Vec<_>>();

            let raw = displays
                .iter()
                .map(|d| format!("[{}] {}", d.index, d.product_name.as_deref().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\n");

            win::destroy(&mut mons);

            Ok(DisplayListReport {
                displays,
                raw: Some(raw),
            })
        }
    }

    fn set_input(&self, display_selector: &str, value: u16) -> Result<()> {
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (display_selector, value);
            bail!("Windows backend can only run on Windows.");
        }

        #[cfg(target_os = "windows")]
        {
            let mons = MonitorList::new()?;
            if mons.is_empty() {
                bail!("No physical monitors found via Dxva2.");
            }

            let idx = resolve_selector(display_selector, &mons)?;
            let mon = &mons[idx];
            win::set_vcp(mon, 0x60, value as u32).context("SetVCPFeature(VCP=0x60)")?;
            Ok(())
        }
    }

    fn get_input(&self, display_selector: &str) -> Result<u16> {
        #[cfg(not(target_os = "windows"))]
        {
            let _ = display_selector;
            bail!("Windows backend can only run on Windows.");
        }

        #[cfg(target_os = "windows")]
        {
            let mons = MonitorList::new()?;
            if mons.is_empty() {
                bail!("No physical monitors found via Dxva2.");
            }

            let idx = resolve_selector(display_selector, &mons)?;
            let mon = &mons[idx];
            let (cur, _max) =
                win::get_vcp(mon, 0x60).context("GetVCPFeatureAndVCPFeatureReply(VCP=0x60)")?;

            Ok(u16::try_from(cur).unwrap_or(u16::MAX))
        }
    }

    fn doctor(&self) -> Result<DoctorReport> {
        #[cfg(not(target_os = "windows"))]
        {
            return Ok(DoctorReport {
                ok: false,
                message: "Windows backend can only run on Windows.".to_string(),
            });
        }

        #[cfg(target_os = "windows")]
        unsafe {
            let mut mons = match win::enum_physical_monitors() {
                Ok(m) => m,
                Err(e) => {
                    return Ok(DoctorReport {
                        ok: false,
                        message: format!("Failed to enumerate monitors: {e}"),
                    })
                }
            };

            if mons.is_empty() {
                return Ok(DoctorReport {
                    ok: false,
                    message: "No physical monitors found via Dxva2.".to_string(),
                });
            }

            let list = mons
                .iter()
                .enumerate()
                .map(|(i, m)| format!("[{}] {}", i + 1, win::monitor_desc(m)))
                .collect::<Vec<_>>()
                .join("\n");
            win::destroy(&mut mons);

            Ok(DoctorReport {
                ok: true,
                message: format!("Dxva2: OK\n\nDetected monitors:\n{list}"),
            })
        }
    }
}

#[cfg(target_os = "windows")]
fn resolve_selector(display_selector: &str, mons: &[win::PHYSICAL_MONITOR]) -> Result<usize> {
    if let Ok(idx_1based) = display_selector.parse::<usize>() {
        if idx_1based == 0 {
            bail!("display selector must be >= 1");
        }
        if idx_1based > mons.len() {
            bail!(
                "Display {idx_1based} out of range. Available:\n{}",
                format_monitor_list(mons)
            );
        }
        return Ok(idx_1based - 1);
    }

    if let Some(needle) = display_selector.strip_prefix("name:") {
        let needle = needle.trim();
        if needle.is_empty() {
            bail!("display selector 'name:' requires a non-empty substring");
        }
        let needle_lc = needle.to_ascii_lowercase();

        let mut matches = mons
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let desc = win::monitor_desc(m);
                if desc.to_ascii_lowercase().contains(&needle_lc) {
                    Some((i, desc))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        if matches.is_empty() {
            bail!(
                "No monitors matched selector '{display_selector}'. Available:\n{}",
                format_monitor_list(mons)
            );
        }
        if matches.len() > 1 {
            matches.sort_by(|a, b| a.0.cmp(&b.0));
            let list = matches
                .iter()
                .map(|(i, d)| format!("[{}] {}", i + 1, d))
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "Selector '{display_selector}' is ambiguous. Matches:\n{list}\n\nUse `--display <index>` from `list`, or a more specific `name:<substring>`."
            );
        }
        return Ok(matches[0].0);
    }

    bail!(
        "Invalid display selector '{display_selector}'. Expected a 1-based index (e.g. '1') or `name:<substring>`."
    )
}

#[cfg(target_os = "windows")]
fn format_monitor_list(mons: &[win::PHYSICAL_MONITOR]) -> String {
    mons.iter()
        .enumerate()
        .map(|(i, m)| format!("[{}] {}", i + 1, win::monitor_desc(m)))
        .collect::<Vec<_>>()
        .join("\n")
}
