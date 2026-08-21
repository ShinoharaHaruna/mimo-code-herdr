// Spawn a MiMo Code agent in a new Herdr tab.
//
// Custom mode (default): launch mimo in a fresh pane; the plugin reports the
// agent over the official custom-source path, then we optionally rename it.
// Communication with a custom-mode agent uses pane primitives
// (`pane send-text` + `agent wait` + `pane wait-output`) — `agent prompt`
// requires an `agent start`-registered agent.
//
// Shim mode (--shim): launch mimo through the opencode-identity shim and
// register it with `agent start --kind opencode`, which unlocks the full
// agent surface (`agent prompt/wait/read`).

use anyhow::{Context, Result, bail};
use std::time::{Duration, Instant};

use crate::herdr;
use crate::install;

const MIMO_CMD: &str = "mimo";

fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    if !ok || !name.as_bytes()[0].is_ascii_lowercase() {
        bail!("invalid agent name {name:?}: must match [a-z][a-z0-9_-]{{0,31}}");
    }
    Ok(())
}

fn resolve_workspace(explicit: Option<&str>) -> Result<String> {
    if let Some(w) = explicit {
        return Ok(w.to_string());
    }
    if let Ok(w) = std::env::var("HERDR_WORKSPACE_ID")
        && !w.is_empty()
    {
        return Ok(w);
    }
    // Fall back to the focused workspace.
    let (stdout, _) = herdr::run(&["workspace", "list"])?;
    let v: serde_json::Value = herdr::parse_json(&stdout)?;
    for ws in v.as_array().context("unexpected workspace list shape")? {
        if ws.get("focused").and_then(|f| f.as_bool()).unwrap_or(false) {
            return Ok(ws
                .get("workspace_id")
                .and_then(|i| i.as_str())
                .context("workspace entry missing id")?
                .to_string());
        }
    }
    bail!("no focused workspace found; pass --workspace explicitly")
}

pub fn spawn(
    name: Option<&str>,
    cwd: Option<&str>,
    workspace: Option<&str>,
    shim: bool,
) -> Result<()> {
    if let Some(n) = name {
        validate_name(n)?;
    }
    let cwd = cwd.map(|c| c.to_string()).unwrap_or_else(|| {
        std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|_| ".".into())
    });
    let ws = resolve_workspace(workspace)?;

    let env = if shim {
        // Prepend the shim dir so `opencode` resolves to the shim inside herdr panes.
        let shim = install::shim_dir();
        let path = std::env::var("PATH").unwrap_or_default();
        Some(format!("PATH={shim}:{path}"))
    } else {
        None
    };
    let (tab, pane) = herdr::tab_create(
        &cwd,
        if shim { "mimo (shim)" } else { "mimo" },
        env.as_deref(),
    )?;
    println!("created tab {tab}, pane {pane}");

    if shim {
        spawn_shim(&ws, &tab, &pane, name, &cwd)
    } else {
        spawn_custom(&ws, &tab, &pane, name)
    }
}

fn spawn_custom(ws: &str, tab: &str, pane: &str, name: Option<&str>) -> Result<()> {
    herdr::pane_run(pane, MIMO_CMD).context("failed to launch mimo in pane")?;

    // Wait until the plugin claims the label (appears in agent list as mimo@this pane).
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut found = false;
    while Instant::now() < deadline {
        if let Ok(agents) = herdr::agent_list()
            && agents
                .iter()
                .any(|a| a.agent == herdr::AGENT && a.pane_id == pane)
        {
            found = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if !found {
        bail!(
            "mimo did not report as an agent within 30s (is the plugin installed? run `mimo-herdr install`)"
        );
    }

    if let Some(n) = name {
        herdr::agent_rename(pane, n).with_context(|| format!("failed to rename agent to {n}"))?;
        println!("agent ready: {n} (pane {pane}, workspace {ws})");
    } else {
        println!("agent ready: mimo (pane {pane}, workspace {ws})");
    }
    println!(
        "note: custom-mode agents cannot use `agent prompt`; use `herdr pane send-text {pane} ...` + `herdr agent wait {pane}` instead"
    );
    let _ = tab;
    Ok(())
}

fn spawn_shim(ws: &str, tab: &str, pane: &str, name: Option<&str>, cwd: &str) -> Result<()> {
    if install::shim_state().is_none() {
        let _ = herdr::tab_close(tab);
        bail!("shim not installed; run `mimo-herdr install --shim` first");
    }
    let agent_name = name.unwrap_or("mimo");
    herdr::agent_start(agent_name, "opencode", pane, 60_000).with_context(|| {
        format!(
            "agent start failed; is {} on PATH in the new pane?",
            shim_dir()
        )
    })?;
    println!("agent ready: {agent_name} (pane {pane}, workspace {ws}, cwd {cwd})");
    println!("full agent surface available: `herdr agent prompt {agent_name} ... --wait`");
    Ok(())
}

fn shim_dir() -> String {
    install::shim_dir()
}
