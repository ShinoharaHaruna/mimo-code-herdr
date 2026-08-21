mod cli;
mod herdr;
mod install;
mod spawn;
mod status;
mod verify;
mod watch;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

fn main() {
    let cli = Cli::parse();
    if let Err(e) = dispatch(cli.command) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn dispatch(cmd: Command) -> Result<()> {
    match cmd {
        Command::Install { shim } => install::install(shim),
        Command::Uninstall { shim } => install::uninstall(shim),
        Command::Status { json } => status::print_status(json),
        Command::Spawn {
            name,
            cwd,
            workspace,
            shim,
        } => spawn::spawn(name.as_deref(), cwd.as_deref(), workspace.as_deref(), shim),
        Command::Verify => verify::verify(),
        Command::Watch {
            pane,
            source,
            agent,
        } => watch::watch(&pane, &source, &agent),
    }
}
