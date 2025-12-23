use std::{
    collections::HashMap,
    env,
    fs,
    path::Path,
    path::PathBuf,
};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::platform::DisplayInfo;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// If set, the Windows tray app will add/remove itself from user startup accordingly.
    #[serde(default)]
    pub start_with_windows: Option<bool>,

    #[serde(default)]
    pub default_display: Option<String>,

    #[serde(default)]
    pub inputs: HashMap<String, u16>,

    #[serde(default)]
    pub monitors: Vec<MonitorConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MonitorConfig {
    #[serde(default)]
    pub r#match: MonitorMatch,

    #[serde(default)]
    pub display: Option<String>,

    #[serde(default)]
    pub inputs: HashMap<String, u16>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct MonitorMatch {
    pub contains: Option<String>,
    pub index: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub display_selector: String,
    pub inputs: HashMap<String, u16>,
}

pub fn load_optional() -> Result<Option<Config>> {
    let Some(path) = resolve_config_path() else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).with_context(|| format!("reading config {}", path.display()))?;
    let cfg: Config =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(cfg))
}

pub fn resolve_config_path() -> Option<PathBuf> {
    if let Ok(p) = env::var("MONITORCTL_CONFIG") {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p));
        }
    }

    let local = PathBuf::from("monitorctl.json");
    if local.exists() {
        return Some(local);
    }

    if let Some(appdata) = env::var_os("APPDATA") {
        return Some(PathBuf::from(appdata).join("monitorctl").join("config.json"));
    }

    if let Some(home) = env::var_os("HOME") {
        return Some(PathBuf::from(home).join(".config").join("monitorctl").join("config.json"));
    }

    None
}

pub fn ensure_config_file_exists() -> Result<PathBuf> {
    let Some(path) = resolve_config_path() else {
        return Err(anyhow!(
            "No config path available (set MONITORCTL_CONFIG or ensure APPDATA/HOME is present)"
        ));
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }

    if !path.exists() {
        let template = serde_json::json!({
            "start_with_windows": false,
            "inputs": {}
        });
        let mut s = serde_json::to_string_pretty(&template).context("serialize config template")?;
        s.push('\n');
        fs::write(&path, s.as_bytes()).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(path)
}

pub fn patch_start_with_windows(enabled: bool) -> Result<PathBuf> {
    let Some(path) = resolve_config_path() else {
        return Err(anyhow!(
            "No config path available (set MONITORCTL_CONFIG or ensure APPDATA/HOME is present)"
        ));
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }

    let mut root = read_json_or_empty_object(&path)?;
    let obj = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("config root must be a JSON object"))?;

    obj.insert(
        "start_with_windows".to_string(),
        Value::Bool(enabled),
    );

    let mut s = serde_json::to_string_pretty(&root).context("serialize config")?;
    s.push('\n');
    fs::write(&path, s.as_bytes()).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn read_json_or_empty_object(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(Value::Object(Default::default()));
    }

    let bytes = fs::read(path).with_context(|| format!("reading config {}", path.display()))?;
    let v: Value =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    Ok(v)
}

pub fn resolve(
    config: Option<&Config>,
    displays: &[DisplayInfo],
    display_arg: Option<&str>,
) -> ResolvedConfig {
    let mut inputs: HashMap<String, u16> = HashMap::new();
    let mut display_selector: Option<String> = display_arg.map(|s| s.to_string());

    let Some(cfg) = config else {
        return ResolvedConfig {
            display_selector: display_selector.unwrap_or_else(|| "1".to_string()),
            inputs,
        };
    };

    inputs.extend(cfg.inputs.iter().map(|(k, v)| (k.to_string(), *v)));
    if display_selector.is_none() {
        display_selector = cfg.default_display.clone();
    }

    if display_selector.is_none() {
        for mon_cfg in &cfg.monitors {
            let Some((matched_display, inferred_selector)) = match_display(mon_cfg, displays) else {
                continue;
            };

            if let Some(explicit) = mon_cfg.display.as_deref() {
                display_selector = Some(explicit.to_string());
            } else {
                display_selector = Some(inferred_selector);
            }

            for (k, v) in &mon_cfg.inputs {
                inputs.insert(k.to_string(), *v);
            }

            let _ = matched_display;
            break;
        }
    }

    ResolvedConfig {
        display_selector: display_selector.unwrap_or_else(|| "1".to_string()),
        inputs,
    }
}

fn match_display<'a>(
    mon_cfg: &MonitorConfig,
    displays: &'a [DisplayInfo],
) -> Option<(&'a DisplayInfo, String)> {
    if let Some(idx) = mon_cfg.r#match.index {
        let d = displays.iter().find(|d| d.index == idx)?;
        return Some((d, selector_for_display(d)));
    }

    let needle = mon_cfg.r#match.contains.as_deref()?;
    let needle_lc = needle.to_ascii_lowercase();
    let d = displays.iter().find(|d| {
        d.product_name
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains(&needle_lc)
    })?;
    Some((d, selector_for_display(d)))
}

fn selector_for_display(display: &DisplayInfo) -> String {
    if let Some(uuid) = display.system_uuid.as_deref() {
        return format!("uuid:{uuid}");
    }
    display.index.to_string()
}

pub fn parse_input_value(value: &str, resolved: &ResolvedConfig) -> Result<u16> {
    if let Ok(v) = value.parse::<u16>() {
        return Ok(v);
    }

    if let Some(v) = resolved.inputs.get(value) {
        return Ok(*v);
    }

    let mut known = resolved
        .inputs
        .keys()
        .map(|k| k.as_str())
        .collect::<Vec<_>>();
    known.sort_unstable();
    let hint = if known.is_empty() {
        "No input presets configured.".to_string()
    } else {
        format!("Known presets: {}", known.join(", "))
    };
    Err(anyhow!(
        "Invalid input value '{value}'. Expected a number or a configured preset name. {hint}"
    ))
}
