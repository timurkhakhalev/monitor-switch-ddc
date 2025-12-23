use anyhow::{anyhow, bail, Context, Result};

use super::{DisplayInfo, DisplayListReport, DoctorReport};

#[cfg(target_os = "windows")]
mod win {
    use windows::{
        core::PCWSTR,
        Win32::{
            Devices::Display::{
                DestroyPhysicalMonitors, GetNumberOfPhysicalMonitorsFromHMONITOR,
                GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, SetVCPFeature,
                MC_VCP_CODE_TYPE, PHYSICAL_MONITOR,
            },
            Foundation::{BOOL, LPARAM, RECT},
            Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR},
        },
    };

    pub unsafe fn enum_physical_monitors() -> windows::core::Result<Vec<PHYSICAL_MONITOR>> {
        let mut all: Vec<PHYSICAL_MONITOR> = Vec::new();

        unsafe extern "system" fn cb(
            hmonitor: HMONITOR,
            _hdc: HDC,
            _rc: *mut RECT,
            lparam: LPARAM,
        ) -> BOOL {
            let vec_ptr = lparam.0 as *mut Vec<PHYSICAL_MONITOR>;
            let vec = unsafe { &mut *vec_ptr };

            let mut count: u32 = 0;
            if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count) }.is_err() {
                return BOOL(1);
            }
            if count == 0 {
                return BOOL(1);
            }

            let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
            if unsafe { GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut monitors) }.is_ok() {
                vec.extend(monitors);
            }

            BOOL(1)
        }

        let mut ptr = &mut all as *mut Vec<PHYSICAL_MONITOR>;
        unsafe { EnumDisplayMonitors(HDC(0), None, Some(cb), LPARAM(&mut ptr as *mut _ as isize)) }?;
        Ok(all)
    }

    pub unsafe fn destroy(monitors: &mut [PHYSICAL_MONITOR]) {
        let _ = unsafe { DestroyPhysicalMonitors(monitors) };
    }

    pub fn wide_to_string(w: &[u16]) -> String {
        let nul = w.iter().position(|c| *c == 0).unwrap_or(w.len());
        String::from_utf16_lossy(&w[..nul])
    }

    pub fn to_pcwstr(s: &str) -> Vec<u16> {
        let mut v: Vec<u16> = s.encode_utf16().collect();
        v.push(0);
        v
    }

    pub fn set_vcp(mon: &PHYSICAL_MONITOR, code: u8, value: u32) -> windows::core::Result<()> {
        unsafe { SetVCPFeature(mon.hPhysicalMonitor, code, value) }
    }

    pub fn get_vcp(mon: &PHYSICAL_MONITOR, code: u8) -> windows::core::Result<(u32, u32)> {
        let mut vcp_type = MC_VCP_CODE_TYPE(0);
        let mut current: u32 = 0;
        let mut maximum: u32 = 0;
        unsafe { GetVCPFeatureAndVCPFeatureReply(mon.hPhysicalMonitor, code, Some(&mut vcp_type), &mut current, &mut maximum) }?;
        Ok((current, maximum))
    }

    pub fn monitor_desc(mon: &PHYSICAL_MONITOR) -> String {
        // szPhysicalMonitorDescription is [u16; 128]
        wide_to_string(&mon.szPhysicalMonitorDescription)
    }

    pub fn pcwstr_from_utf16(buf: &Vec<u16>) -> PCWSTR {
        PCWSTR(buf.as_ptr())
    }
}

pub struct WindowsDxva2Backend;

impl WindowsDxva2Backend {
    pub fn new() -> Self {
        Self
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
            let mut mons = win::enum_physical_monitors().context("enumerating physical monitors")?;
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
        unsafe {
            let idx: usize = display_selector
                .parse::<usize>()
                .context("display selector must be a 1-based integer on Windows")?;
            if idx == 0 {
                bail!("display selector must be >= 1");
            }

            let mut mons = win::enum_physical_monitors().context("enumerating physical monitors")?;
            if mons.is_empty() {
                bail!("No physical monitors found via Dxva2.");
            }
            if idx > mons.len() {
                let names = mons
                    .iter()
                    .enumerate()
                    .map(|(i, m)| format!("[{}] {}", i + 1, win::monitor_desc(m)))
                    .collect::<Vec<_>>()
                    .join("\n");
                win::destroy(&mut mons);
                bail!("Display {idx} out of range. Available:\n{names}");
            }

            let mon = &mons[idx - 1];
            win::set_vcp(mon, 0x60, value as u32).context("SetVCPFeature(VCP=0x60)")?;
            win::destroy(&mut mons);
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
        unsafe {
            let idx: usize = display_selector
                .parse::<usize>()
                .context("display selector must be a 1-based integer on Windows")?;
            if idx == 0 {
                bail!("display selector must be >= 1");
            }

            let mut mons = win::enum_physical_monitors().context("enumerating physical monitors")?;
            if mons.is_empty() {
                bail!("No physical monitors found via Dxva2.");
            }
            if idx > mons.len() {
                let names = mons
                    .iter()
                    .enumerate()
                    .map(|(i, m)| format!("[{}] {}", i + 1, win::monitor_desc(m)))
                    .collect::<Vec<_>>()
                    .join("\n");
                win::destroy(&mut mons);
                bail!("Display {idx} out of range. Available:\n{names}");
            }

            let mon = &mons[idx - 1];
            let (cur, _max) = win::get_vcp(mon, 0x60).context("GetVCPFeatureAndVCPFeatureReply(VCP=0x60)")?;
            win::destroy(&mut mons);

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
