use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mimo-herdr",
    version,
    about = "MiMo Code <-> herdr custom-agent bridge",
    long_about = "Makes MiMo Code a first-class custom agent in Herdr: lifecycle \
                  state in the sidebar, exit cleanup via a crash-proof watchdog, \
                  and one-command spawn. Uses herdr's official custom-integration \
                  path (pane report-agent --source custom:...)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Install the plugin into the MiMo Code config dir (idempotent)
    Install {
        /// Also deploy the optional opencode-identity shim for full `agent prompt` support
        #[arg(long)]
        shim: bool,
    },
    /// Remove the plugin (and optionally the shim)
    Uninstall {
        /// Also remove the shim installed with `install --shim`
        #[arg(long)]
        shim: bool,
    },
    /// Health check: herdr, mimo, plugin file, watchdog wiring
    Status {
        /// Machine-readable JSON output
        #[arg(long)]
        json: bool,
    },
    /// Spawn a MiMo Code agent in a new Herdr tab
    Spawn {
        /// Agent display name (renamed in Herdr once the agent is live)
        #[arg(long)]
        name: Option<String>,
        /// Working directory for the new tab (default: current directory)
        #[arg(long)]
        cwd: Option<String>,
        /// Workspace to spawn in (default: current/focused workspace)
        #[arg(long)]
        workspace: Option<String>,
        /// Use the opencode-identity shim mode (requires `install --shim`)
        #[arg(long)]
        shim: bool,
    },
    /// End-to-end smoke test: spawn, state, prompt, exit cleanup
    Verify,
    /// Internal watchdog: releases the agent label when stdin hits EOF
    /// (i.e. the plugin process died, including SIGKILL)
    Watch {
        /// Herdr pane id the plugin runs in
        #[arg(long)]
        pane: String,
        /// Report source, must match the plugin's source
        #[arg(long)]
        source: String,
        /// Agent label, must match the plugin's label
        #[arg(long)]
        agent: String,
    },
}
