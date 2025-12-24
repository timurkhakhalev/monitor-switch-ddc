use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::platform::Backend;
use crate::tray::commands::{
    Command, InputsMap, CMD_BASE_INPUT, CMD_EDIT_CONFIG, CMD_OPEN_CONFIG_FOLDER, CMD_QUIT,
    CMD_RELOAD, CMD_TOGGLE_STARTUP,
};
use crate::tray::menu::{MenuItem, MenuSpec};
use crate::tray::startup::StartupManager;
use crate::{config, platform, tray::common};

pub struct TrayModel {
    inputs: InputsMap,
    display_selector: String,
    backend: Box<dyn Backend>,
    last_error: Option<String>,
    start_enabled: bool,
    start_pref: Option<bool>,
}

#[derive(Debug, Default, Clone)]
pub struct ModelUpdate {
    pub refresh_menu: bool,
    pub refresh_tooltip: bool,
    pub quit: bool,
    pub open_path: Option<PathBuf>,
}

impl TrayModel {
    pub fn new() -> Result<Self> {
        let backend = platform::backend().context("select backend")?;
        let (display_selector, inputs, start_pref, load_error) = load_display_and_inputs(&*backend);

        Ok(Self {
            inputs,
            display_selector,
            backend,
            last_error: load_error,
            start_enabled: start_pref.unwrap_or(false),
            start_pref,
        })
    }

    pub fn inputs(&self) -> &InputsMap {
        &self.inputs
    }

    pub fn display_selector(&self) -> &str {
        &self.display_selector
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn start_enabled(&self) -> bool {
        self.start_enabled
    }

    pub fn menu_spec(&self) -> MenuSpec {
        let mut items = Vec::new();
        items.push(MenuItem::Header("Inputs".to_string()));

        for (cmd, (name, value)) in &self.inputs {
            let label = format!("{} ({value})", common::pretty_input_label(name));
            items.push(MenuItem::Action {
                id: *cmd,
                title: label,
                checked: false,
                enabled: true,
            });
        }

        items.push(MenuItem::Separator);
        items.push(MenuItem::Header("Actions".to_string()));
        items.push(MenuItem::Action {
            id: CMD_TOGGLE_STARTUP,
            title: "Start at login".to_string(),
            checked: self.start_enabled,
            enabled: true,
        });
        items.push(MenuItem::Action {
            id: CMD_EDIT_CONFIG,
            title: "Edit config".to_string(),
            checked: false,
            enabled: true,
        });
        items.push(MenuItem::Action {
            id: CMD_OPEN_CONFIG_FOLDER,
            title: "Open config folder".to_string(),
            checked: false,
            enabled: true,
        });
        items.push(MenuItem::Action {
            id: CMD_RELOAD,
            title: "Reload config".to_string(),
            checked: false,
            enabled: true,
        });
        items.push(MenuItem::Action {
            id: CMD_QUIT,
            title: "Quit".to_string(),
            checked: false,
            enabled: true,
        });

        MenuSpec::new(items)
    }

    pub fn handle(&mut self, cmd: Command, startup: &dyn StartupManager) -> Result<ModelUpdate> {
        let update = match cmd {
            Command::Input(value) => self
                .set_input(value)
                .map(|_| ModelUpdate {
                    refresh_tooltip: true,
                    ..Default::default()
                })
                .unwrap_or_else(|err| self.note_error(err)),
            Command::Reload => self
                .reload_config(startup)
                .unwrap_or_else(|err| self.note_error(err)),
            Command::ToggleStartup => self
                .toggle_startup(startup)
                .unwrap_or_else(|err| self.note_error(err)),
            Command::EditConfig => self
                .edit_config()
                .map(|path| ModelUpdate {
                    open_path: Some(path),
                    ..Default::default()
                })
                .unwrap_or_else(|err| self.note_error(err)),
            Command::OpenConfigFolder => self
                .open_config_folder()
                .map(|path| ModelUpdate {
                    open_path: Some(path),
                    ..Default::default()
                })
                .unwrap_or_else(|err| self.note_error(err)),
            Command::Quit => ModelUpdate {
                quit: true,
                ..Default::default()
            },
        };

        Ok(update)
    }

    fn note_error(&mut self, err: anyhow::Error) -> ModelUpdate {
        self.last_error = Some(err.to_string());
        ModelUpdate {
            refresh_tooltip: true,
            ..Default::default()
        }
    }

    fn set_input(&mut self, value: u16) -> Result<()> {
        self.backend
            .set_input(&self.display_selector, value)
            .with_context(|| format!("set input {value} on '{}'", self.display_selector))?;
        self.last_error = None;
        Ok(())
    }

    fn reload_config(&mut self, startup: &dyn StartupManager) -> Result<ModelUpdate> {
        let (display_selector, inputs, start_pref, load_error) =
            load_display_and_inputs(&*self.backend);
        self.display_selector = display_selector;
        self.inputs = inputs;
        self.start_pref = start_pref;

        let (start_enabled, startup_error) = common::apply_startup_pref(
            self.start_pref,
            |enabled| {
                startup
                    .set_enabled(enabled)
                    .context("update startup setting")
            },
            || startup.is_enabled().context("read startup setting"),
        );
        self.start_enabled = start_enabled;
        self.last_error = load_error.or(startup_error);

        Ok(ModelUpdate {
            refresh_menu: true,
            refresh_tooltip: true,
            ..Default::default()
        })
    }

    fn toggle_startup(&mut self, startup: &dyn StartupManager) -> Result<ModelUpdate> {
        let next = !self.start_enabled;
        startup
            .set_enabled(next)
            .context("update startup setting")?;
        let _path = config::patch_start_with_windows(next).context("update config")?;
        self.start_enabled = next;
        self.last_error = None;
        Ok(ModelUpdate {
            refresh_menu: true,
            refresh_tooltip: true,
            ..Default::default()
        })
    }

    fn edit_config(&mut self) -> Result<PathBuf> {
        let path = config::ensure_config_file_exists().context("ensure config exists")?;
        Ok(path)
    }

    fn open_config_folder(&mut self) -> Result<PathBuf> {
        let Some(path) = config::resolve_config_path() else {
            return Err(anyhow::anyhow!("No config path available"));
        };
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("No parent directory for config path"))?;
        Ok(parent.to_path_buf())
    }
}

fn load_display_and_inputs(
    backend: &dyn Backend,
) -> (String, InputsMap, Option<bool>, Option<String>) {
    let cfg = match config::load_optional() {
        Ok(v) => v,
        Err(e) => {
            return (
                "1".to_string(),
                common::default_inputs(CMD_BASE_INPUT),
                None,
                Some(e.to_string()),
            )
        }
    };

    let start_pref = cfg.as_ref().and_then(|c| c.start_with_windows);

    let (displays, load_error) = match backend.list_displays() {
        Ok(report) => (report.displays, None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };

    let resolved = config::resolve(cfg.as_ref(), &displays, None);
    let inputs = common::build_inputs(&resolved.inputs, CMD_BASE_INPUT);

    (resolved.display_selector, inputs, start_pref, load_error)
}
