use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

mod platform;

#[derive(Parser, Debug)]
#[command(name = "monitorctl", version, about = "DDC/CI monitor input switcher (PoC)")]
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
    /// Reads the current input source as raw VCP 0x60 value (Windows only in this PoC).
    GetInput {
        /// Display selector. On Windows this is a 1-based monitor index from `list`.
        #[arg(long, default_value = "1")]
        display: String,
    },
    /// Sets input source to a raw VCP 0x60 value (e.g., 26 for USB-C on XG27ACS).
    SetInput {
        /// Display selector. On macOS this is passed through to `m1ddc display <selector> ...`.
        /// Common values: "1", "uuid:<UUID>", "edid:<UUID>".
        #[arg(long, default_value = "1")]
        display: String,
        /// Raw input value to set (VCP 0x60).
        value: u16,
    },
    /// Checks local prerequisites and prints guidance.
    Doctor,
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
            backend
                .set_input(&display, value)
                .with_context(|| format!("set input to {value} on display '{display}'"))?;
            println!("{value}");
        }
        Command::GetInput { display } => {
            let backend = platform::backend()?;
            let value = backend
                .get_input(&display)
                .with_context(|| format!("get input on display '{display}'"))?;
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
    }

    Ok(())
}
