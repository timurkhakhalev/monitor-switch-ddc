use std::collections::BTreeMap;

pub const CMD_BASE_INPUT: u16 = 2000;
pub const CMD_RELOAD: u16 = 5000;
pub const CMD_QUIT: u16 = 5001;
pub const CMD_TOGGLE_STARTUP: u16 = 5002;
pub const CMD_EDIT_CONFIG: u16 = 5003;
pub const CMD_OPEN_CONFIG_FOLDER: u16 = 5004;

pub type InputsMap = BTreeMap<u16, (String, u16)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Input(u16),
    Reload,
    Quit,
    ToggleStartup,
    EditConfig,
    OpenConfigFolder,
}

pub fn decode(cmd_id: u16, inputs: &InputsMap) -> Option<Command> {
    if let Some((_name, value)) = inputs.get(&cmd_id) {
        return Some(Command::Input(*value));
    }

    match cmd_id {
        CMD_RELOAD => Some(Command::Reload),
        CMD_QUIT => Some(Command::Quit),
        CMD_TOGGLE_STARTUP => Some(Command::ToggleStartup),
        CMD_EDIT_CONFIG => Some(Command::EditConfig),
        CMD_OPEN_CONFIG_FOLDER => Some(Command::OpenConfigFolder),
        _ => None,
    }
}
