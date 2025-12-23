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
    fn doctor(&self) -> Result<DoctorReport>;
}

#[cfg(target_os = "macos")]
mod macos_m1ddc;

pub fn backend() -> Result<Box<dyn Backend>> {
    #[cfg(target_os = "macos")]
    {
        return Ok(Box::new(macos_m1ddc::M1DdcBackend::new()));
    }

    #[cfg(not(target_os = "macos"))]
    {
        use anyhow::bail;
        bail!("This PoC currently supports macOS only.");
    }
}
