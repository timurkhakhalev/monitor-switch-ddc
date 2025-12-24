use anyhow::Result;

pub trait StartupManager {
    fn is_enabled(&self) -> Result<bool>;
    fn set_enabled(&self, enabled: bool) -> Result<()>;
}
