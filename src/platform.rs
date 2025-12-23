use anyhow::Result;

#[derive(Debug, Clone)]
pub struct DisplayInfo {
    pub index: u32,
    pub product_name: Option<String>,
    pub system_uuid: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DisplayListReport {
    pub displays: Vec<DisplayInfo>,
    pub raw: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub ok: bool,
    pub message: String,
}

pub trait Backend {
    fn list_displays(&self) -> Result<DisplayListReport>;
    fn set_input(&self, display_selector: &str, value: u16) -> Result<()>;
    fn get_input(&self, display_selector: &str) -> Result<u16>;
    fn doctor(&self) -> Result<DoctorReport>;
}

#[cfg(target_os = "macos")]
mod macos_m1ddc;
#[cfg(target_os = "windows")]
mod windows_dxva2;

pub fn backend() -> Result<Box<dyn Backend>> {
    #[cfg(target_os = "macos")]
    {
        return Ok(Box::new(macos_m1ddc::M1DdcBackend::new()));
    }

    #[cfg(target_os = "windows")]
    {
        return Ok(Box::new(windows_dxva2::WindowsDxva2Backend::new()));
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        use anyhow::bail;
        bail!("Unsupported OS (this PoC supports macOS and Windows).");
    }
}
