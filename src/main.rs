use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use monitorctl::{config, platform};

#[derive(Parser, Debug)]
#[command(name = "monitorctl", version, about = "DDC/CI monitor input switcher")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Prints detected external displays (best-effort).
    List {
        /// Show raw backend output too.
        #[arg(long)]
        raw: bool,
    },
    /// Reads the current input source as raw VCP 0x60 value (Windows-only at the moment).
    GetInput {
        /// Display selector. On Windows this is a 1-based monitor index from `list`.
        /// If omitted, `monitorctl.json` / config defaults may be used.
        #[arg(long)]
        display: Option<String>,
    },
    /// Sets input source to a raw VCP 0x60 value (e.g., 26 for USB-C on XG27ACS).
    SetInput {
        /// Display selector. On macOS this is passed through to `m1ddc display <selector> ...`.
        /// Common values: "1", "uuid:<UUID>", "edid:<UUID>".
        /// If omitted, `monitorctl.json` / config defaults may be used.
        #[arg(long)]
        display: Option<String>,
        /// Raw input value to set (VCP 0x60) OR a configured preset name (e.g. "dp1").
        value: String,
    },
    /// Checks local prerequisites and prints guidance.
    Doctor,
    /// Prints the config path that would be used (if any).
    ConfigPath,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::List { raw } => {
            let backend = platform::backend()?;
            let report = backend.list_displays().context("list displays")?;
            if raw {
                if let Some(raw) = report.raw {
                    println!("{raw}");
                }
            }
            for d in report.displays {
                println!(
                    "[{}] {} (system_uuid={})",
                    d.index,
                    d.product_name.as_deref().unwrap_or("<unknown>"),
                    d.system_uuid.as_deref().unwrap_or("<unknown>")
                );
            }
        }
        Command::SetInput { display, value } => {
            let backend = platform::backend()?;
            let report = backend
                .list_displays()
                .context("list displays (for config)")?;
            let cfg = config::load_optional()?;
            let resolved = config::resolve(cfg.as_ref(), &report.displays, display.as_deref());
            let value = config::parse_input_value(&value, &resolved)?;
            backend
                .set_input(&resolved.display_selector, value)
                .with_context(|| {
                    format!(
                        "set input to {value} on display '{}'",
                        resolved.display_selector
                    )
                })?;
            println!("{value}");
        }
        Command::GetInput { display } => {
            let backend = platform::backend()?;
            let report = backend
                .list_displays()
                .context("list displays (for config)")?;
            let cfg = config::load_optional()?;
            let resolved = config::resolve(cfg.as_ref(), &report.displays, display.as_deref());
            let value = backend
                .get_input(&resolved.display_selector)
                .with_context(|| format!("get input on display '{}'", resolved.display_selector))?;
            println!("{value}");
        }
        Command::Doctor => {
            let backend = platform::backend()?;
            let notes = backend.doctor().context("doctor")?;
            if !notes.ok {
                bail!(notes.message);
            }
            println!("{}", notes.message);
        }
        Command::ConfigPath => {
            if let Some(path) = config::resolve_config_path() {
                println!("{}", path.display());
            }
        }
    }

    Ok(())
}
