// Watchdog process spawned by the plugin with stdin attached to a pipe held
// by the plugin. When the plugin process dies (quit, crash, SIGKILL — any
// mode), the kernel closes the pipe, stdin hits EOF, and we release the agent
// label with a seq stamped +1s ahead of now so it outranks every report the
// dead process made (herdr arbitrates per-source by monotonic seq), while
// losing to reports from any mimo started later.

use anyhow::{Context, Result};
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::herdr;

pub fn watch(pane: &str, source: &str, agent: &str) -> Result<()> {
    // Block until the plugin's pipe closes.
    let mut buf = [0u8; 64];
    let mut stdin = std::io::stdin();
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break, // EOF: plugin process is gone
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before epoch")?
        .as_millis() as u64;
    let seq = (now_ms + 1000) * 1000;

    herdr::release_agent(pane, source, agent, seq)?;
    eprintln!("mimo-herdr watch: released agent {agent} on {pane} (seq {seq})");
    Ok(())
}
