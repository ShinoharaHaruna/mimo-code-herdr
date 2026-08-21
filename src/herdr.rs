// Thin wrapper around the herdr CLI. All agent reporting goes through the
// official custom-integration path (`pane report-agent --source custom:...`)
// per https://herdr.dev/docs/integrations/ instead of the raw socket API,
// which keeps the bridge stable across herdr versions.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// Canonical source label for the bridge (documentation constant; the plugin
/// is the authority and passes it to `watch` at runtime).
#[allow(dead_code)]
pub const SOURCE: &str = "custom:mimo-herdr";
/// Agent label shown in the Herdr sidebar.
pub const AGENT: &str = "mimo";
pub const PLUGIN_FILE: &str = "herdr-agent-state.js";

/// Resolve the herdr binary: HERDR_BIN_PATH first (injected into panes),
/// then PATH.
pub fn herdr_bin() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("HERDR_BIN_PATH")
        && !p.trim().is_empty()
    {
        return Ok(PathBuf::from(p));
    }
    let mut cmd = Command::new("herdr");
    cmd.arg("--version");
    let out = cmd
        .output()
        .context("herdr not found in PATH; is herdr installed?")?;
    if !out.status.success() {
        bail!("herdr --version failed");
    }
    Ok(PathBuf::from("herdr"))
}

pub fn herdr_version() -> Result<String> {
    let bin = herdr_bin()?;
    let out = Command::new(&bin)
        .arg("--version")
        .output()
        .context("failed to run herdr --version")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Run a herdr CLI command; returns (stdout, stderr). Errors on non-zero exit.
pub fn run(args: &[&str]) -> Result<(String, String)> {
    let bin = herdr_bin()?;
    let out = Command::new(&bin)
        .args(args)
        .output()
        .with_context(|| format!("failed to run herdr {}", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        bail!("herdr {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok((stdout, stderr))
}

/// Parse a herdr CLI JSON response like {"id":..., "result": {...}}.
pub fn parse_json<T: serde::de::DeserializeOwned>(stdout: &str) -> Result<T> {
    let v: serde_json::Value = serde_json::from_str(stdout).context("invalid herdr JSON output")?;
    serde_json::from_value(
        v.get("result")
            .cloned()
            .context("herdr response missing result field")?,
    )
    .context("unexpected herdr result shape")
}

pub fn release_agent(pane: &str, source: &str, agent: &str, seq: u64) -> Result<()> {
    run(&[
        "pane",
        "release-agent",
        pane, // positional: herdr pane release-agent <PANE_ID> --source ...
        "--source",
        source,
        "--agent",
        agent,
        "--seq",
        &seq.to_string(),
    ])
    .map(|_| ())
}

#[derive(serde::Deserialize, Debug)]
pub struct AgentEntry {
    pub agent: String,
    pub pane_id: String,
    #[serde(default)]
    pub agent_status: String,
}

pub fn agent_list() -> Result<Vec<AgentEntry>> {
    let (stdout, _) = run(&["agent", "list"])?;
    let v: serde_json::Value =
        serde_json::from_str(&stdout).context("invalid herdr JSON output")?;
    let agents = v
        .get("result")
        .and_then(|r| r.get("agents"))
        .context("agent list response missing agents field")?;
    serde_json::from_value(agents.clone()).context("unexpected agents shape")
}

pub fn tab_create(cwd: &str, label: &str, env: Option<&str>) -> Result<(String, String)> {
    let mut args = vec![
        "tab",
        "create",
        "--cwd",
        cwd,
        "--label",
        label,
        "--no-focus",
    ];
    if let Some(e) = env {
        args.push("--env");
        args.push(e);
    }
    let (stdout, _) = run(&args)?;
    let v: serde_json::Value = parse_json(&stdout)?;
    let tab = v
        .get("tab")
        .and_then(|t| t.get("tab_id"))
        .and_then(|t| t.as_str())
        .context("tab create response missing tab_id")?
        .to_string();
    let pane = v
        .get("root_pane")
        .and_then(|p| p.get("pane_id"))
        .and_then(|p| p.as_str())
        .context("tab create response missing root pane id")?
        .to_string();
    Ok((tab, pane))
}

pub fn tab_close(tab: &str) -> Result<()> {
    run(&["tab", "close", tab]).map(|_| ())
}

pub fn pane_run(pane: &str, command: &str) -> Result<()> {
    run(&["pane", "run", pane, command]).map(|_| ())
}

pub fn pane_send_text(pane: &str, text: &str) -> Result<()> {
    run(&["pane", "send-text", pane, text]).map(|_| ())
}

pub fn pane_send_enter(pane: &str) -> Result<()> {
    run(&["pane", "send-keys", pane, "enter"]).map(|_| ())
}

pub fn agent_rename(target: &str, name: &str) -> Result<()> {
    run(&["agent", "rename", target, name]).map(|_| ())
}

pub fn agent_start(name: &str, kind: &str, pane: &str, timeout_ms: u64) -> Result<()> {
    run(&[
        "agent",
        "start",
        name,
        "--kind",
        kind,
        "--pane",
        pane,
        "--timeout",
        &timeout_ms.to_string(),
    ])
    .map(|_| ())
}

/// Poll the pane output until it contains `match_text` (more reliable than
/// `pane wait-output` for TUIs rendering on the alternate screen).
pub fn pane_wait_text(pane: &str, match_text: &str, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok((out, _)) = run(&["pane", "read", pane, "--source", "recent", "--lines", "300"])
            && out.contains(match_text)
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    anyhow::bail!("timed out waiting for {match_text:?} in pane {pane}")
}
