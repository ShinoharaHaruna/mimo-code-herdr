// End-to-end smoke test:
//   spawn mimo in a throwaway tab -> agent appears -> state transitions ->
//   pane-level prompt -> response read back -> exit -> row cleanup (watchdog).

use anyhow::{Context, Result, bail};
use std::time::{Duration, Instant};

use crate::herdr;
use crate::install;

const LABEL: &str = "mimo-herdr-verify";
const PROBE: &str = "Reply with exactly: VRFY-OK";
const MATCH: &str = "VRFY-OK";

fn wait_until<F>(what: &str, timeout: Duration, mut f: F) -> Result<()>
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("timeout waiting for {what}")
}

pub fn verify() -> Result<()> {
    // Preconditions.
    let (_, ours) = install::plugin_state();
    if !ours {
        bail!("plugin not installed; run `mimo-herdr install` first");
    }
    herdr::herdr_version().context("herdr not available")?;

    // Use a scratch dir and pre-trust it so mimo never shows its first-run
    // "do you trust this folder?" dialog (which blocks agent reporting).
    let scratch = std::env::temp_dir().join(format!("mimo-herdr-verify-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).context("failed to create scratch dir")?;
    trust_workspace(&scratch)?;

    let (tab, pane) = herdr::tab_create(scratch.to_str().unwrap(), LABEL, None)?;
    println!("[1/6] created tab {tab}, pane {pane}");

    let fail = |_tab: &str, pane: &str, step: &str| -> Result<()> {
        // Keep the pane for inspection instead of silently destroying evidence.
        let (stdout, _) =
            herdr::run(&["pane", "read", pane, "--source", "visible", "--lines", "30"])
                .unwrap_or_default();
        eprintln!("--- pane snapshot ({step}) ---\n{stdout}");
        eprintln!("--- run `herdr pane read {pane}` and `herdr agent get {pane}` for details ---");
        bail!("verify failed at {step}")
    };

    // [2] Launch mimo and wait for the plugin to claim the agent label.
    herdr::pane_run(&pane, "mimo").context("failed to launch mimo")?;
    if let Err(e) = wait_until(
        "mimo agent to appear in agent list",
        Duration::from_secs(30),
        || {
            herdr::agent_list()
                .map(|a| {
                    a.iter()
                        .any(|e| e.agent == herdr::AGENT && e.pane_id == pane)
                })
                .unwrap_or(false)
        },
    ) {
        let _ = fail(&tab, &pane, "agent claim");
        return Err(e);
    }
    println!("[2/6] mimo reported as agent (custom:mimo-herdr)");

    // [3] Wait until the TUI home screen is rendered (input box ready),
    // then send a prompt at the pane level and poll for the reply.
    // Polling `pane read` is more reliable than `pane wait-output` for
    // TUIs that render on the alternate screen.
    wait_until("mimo TUI to be ready", Duration::from_secs(30), || {
        herdr::run(&["pane", "read", &pane, "--source", "recent", "--lines", "60"])
            .map(|(out, _)| out.contains("Type your message"))
            .unwrap_or(false)
    })
    .context("mimo TUI did not become ready")?;
    herdr::pane_send_text(&pane, PROBE)?;
    herdr::pane_send_enter(&pane)?;
    if let Err(e) = herdr::pane_wait_text(&pane, MATCH, Duration::from_secs(120)) {
        let _ = fail(&tab, &pane, "prompt reply");
        return Err(e);
    }
    println!("[3/6] mimo replied with {MATCH}");

    // [4] Wait for mimo to settle (reply fully processed), then exit and
    // wait for the watchdog to release the label.
    wait_until(
        "mimo to settle after reply",
        Duration::from_secs(30),
        || {
            herdr::agent_list()
                .map(|a| {
                    a.iter()
                        .find(|e| e.agent == herdr::AGENT && e.pane_id == pane)
                        .map(|e| matches!(e.agent_status.as_str(), "idle" | "done" | "unknown"))
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        },
    )
    .context("mimo did not settle after reply")?;
    herdr::pane_send_text(&pane, "/exit")?;
    herdr::pane_send_enter(&pane)?;
    if let Err(_e) = wait_until(
        "watchdog to release the agent row",
        Duration::from_secs(30),
        || {
            herdr::agent_list()
                .map(|a| {
                    !a.iter()
                        .any(|e| e.agent == herdr::AGENT && e.pane_id == pane)
                })
                .unwrap_or(false)
        },
    ) {
        // Fallback: force-quit the TUI, then re-check for the release.
        let _ = herdr::run(&["pane", "send-keys", &pane, "ctrl+c"]);
        let _ = herdr::run(&["pane", "send-keys", &pane, "ctrl+c"]);
        if let Err(e2) = wait_until(
            "watchdog to release the agent row after force-quit",
            Duration::from_secs(15),
            || {
                herdr::agent_list()
                    .map(|a| {
                        !a.iter()
                            .any(|e| e.agent == herdr::AGENT && e.pane_id == pane)
                    })
                    .unwrap_or(false)
            },
        ) {
            let _ = fail(&tab, &pane, "watchdog release");
            return Err(e2);
        }
    }
    println!("[4/6] agent row released after exit (watchdog OK)");

    // [5] Cleanup.
    let _ = herdr::tab_close(&tab);
    let _ = untrust_workspace(&scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    println!("[5/6] cleanup done");

    println!("[6/6] verify PASSED");
    Ok(())
}

/// Add a path to mimo's trusted-workspaces.json so a fresh directory does not
/// trigger the first-run trust dialog.
fn trust_workspace(path: &std::path::Path) -> Result<()> {
    let file = trusted_workspaces_file()?;
    let mut data = read_or_default(&file);
    let list = data
        .as_object_mut()
        .and_then(|o| o.get_mut("trustedPaths"))
        .and_then(|v| v.as_array_mut())
        .context("trusted-workspaces.json has an unexpected shape")?;
    let s = path.to_string_lossy().to_string();
    if !list.iter().any(|v| v.as_str() == Some(&s)) {
        list.push(serde_json::Value::String(s));
        write_json(&file, &data)?;
    }
    Ok(())
}

/// Remove a path previously added by `trust_workspace` (best effort).
fn untrust_workspace(path: &std::path::Path) -> Result<()> {
    let file = trusted_workspaces_file()?;
    if !file.exists() {
        return Ok(());
    }
    let mut data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
    let s = path.to_string_lossy().to_string();
    if let Some(list) = data
        .as_object_mut()
        .and_then(|o| o.get_mut("trustedPaths"))
        .and_then(|v| v.as_array_mut())
    {
        list.retain(|v| v.as_str() != Some(&s));
        write_json(&file, &data)?;
    }
    Ok(())
}

fn trusted_workspaces_file() -> Result<std::path::PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".local")
                .join("share")
        });
    Ok(base.join("mimocode").join("trusted-workspaces.json"))
}

fn read_or_default(file: &std::path::Path) -> serde_json::Value {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_else(|| serde_json::json!({ "version": 1, "trustedPaths": [] }))
}

fn write_json(file: &std::path::Path, data: &serde_json::Value) -> Result<()> {
    let dir = file
        .parent()
        .context("trusted-workspaces path has no parent")?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(file, serde_json::to_string_pretty(data)?)?;
    Ok(())
}
