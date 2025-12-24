use std::collections::{BTreeMap, HashMap};

pub fn pretty_input_label(key: &str) -> &str {
    match key {
        "dp1" => "DisplayPort 1",
        "dp2" => "DisplayPort 2",
        "usb_c" => "USB-C",
        "usbc" => "USB-C",
        "hdmi1" => "HDMI 1",
        "hdmi2" => "HDMI 2",
        _ => key,
    }
}

pub fn build_inputs(inputs: &HashMap<String, u16>, base_cmd: u16) -> BTreeMap<u16, (String, u16)> {
    if inputs.is_empty() {
        return default_inputs(base_cmd);
    }

    let mut keys = inputs
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect::<Vec<_>>();
    keys.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out: BTreeMap<u16, (String, u16)> = BTreeMap::new();
    let mut next_cmd = base_cmd;
    for (k, v) in keys {
        out.insert(next_cmd, (k, v));
        next_cmd += 1;
    }
    out
}

pub fn default_inputs(base_cmd: u16) -> BTreeMap<u16, (String, u16)> {
    // Defaults for your XG27ACS setup; override with config for other monitors.
    let mut inputs: BTreeMap<u16, (String, u16)> = BTreeMap::new();
    let mut next_cmd = base_cmd;
    for (k, v) in [("dp1", 15u16), ("usb_c", 26u16)] {
        inputs.insert(next_cmd, (k.to_string(), v));
        next_cmd += 1;
    }
    inputs
}

pub fn apply_startup_pref<SetEnabled, IsEnabled, Error>(
    pref: Option<bool>,
    mut set_enabled: SetEnabled,
    mut is_enabled: IsEnabled,
) -> (bool, Option<String>)
where
    SetEnabled: FnMut(bool) -> Result<(), Error>,
    IsEnabled: FnMut() -> Result<bool, Error>,
    Error: std::fmt::Display,
{
    match pref {
        Some(enabled) => match set_enabled(enabled) {
            Ok(()) => (enabled, None),
            Err(e) => (enabled, Some(e.to_string())),
        },
        None => match is_enabled() {
            Ok(enabled) => (enabled, None),
            Err(e) => (false, Some(e.to_string())),
        },
    }
}
