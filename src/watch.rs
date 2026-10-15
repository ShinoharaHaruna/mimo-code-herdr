// Watchdog process spawned by the plugin with stdin attached to a pipe held
// by the plugin. When the plugin process dies (quit, crash, SIGKILL — any
// mode), the kernel closes the pipe, stdin hits EOF, and we release the agent
// label.
//
// The plugin writes each seq it is about to use as a line on the pipe. On EOF
// we release with `last_seq + 1`, which outranks every report the dead
// process made while losing to reports from any mimo started later. The
// release seq is persisted to a floor file that the next plugin process
// starts above, so a restart inside the same second is not outranked.

use anyhow::{Context, Result};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::herdr;

/// Shared with the plugin (plugin/herdr-agent-state.js `seqFloorPath`).
fn seq_floor_path() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME")
        && !xdg.trim().is_empty()
    {
        return Ok(PathBuf::from(xdg).join("mimo-herdr").join("watch-seq"));
    }
    let home = dirs::home_dir().context("cannot resolve home directory")?;
    Ok(home
        .join(".local")
        .join("state")
        .join("mimo-herdr")
        .join("watch-seq"))
}

pub fn watch(pane: &str, source: &str, agent: &str) -> Result<()> {
    // Block until the plugin's pipe closes, remembering the highest seq it
    // announced. Each report is a single ascii line (see plugin nextSeq).
    let mut last_seq: u64 = 0;
    {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF: plugin process is gone
                Ok(_) => {
                    if let Ok(n) = line.trim().parse::<u64>()
                        && n > last_seq
                    {
                        last_seq = n;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before epoch")?
        .as_millis() as u64;
    // Beat every seq the dead process announced; if it died before announcing
    // anything, fall back to a wall-clock stamp so old reports still lose.
    let seq = if last_seq > 0 {
        last_seq + 1
    } else {
        (now_ms + 1000) * 1000
    };

    // Persist the floor BEFORE releasing so the next plugin starts above this
    // seq even if the release itself fails.
    let floor_path = seq_floor_path()?;
    if let Some(parent) = floor_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut f = std::fs::File::create(&floor_path)
        .with_context(|| format!("failed to write {}", floor_path.display()))?;
    write!(f, "{seq}")?;

    herdr::release_agent(pane, source, agent, seq)?;
    eprintln!("mimo-herdr watch: released agent {agent} on {pane} (seq {seq})");
    Ok(())
}
