// Health check: herdr, mimo, plugin wiring, shim.

use anyhow::Result;
use serde::Serialize;

use crate::herdr;
use crate::install;

#[derive(Serialize, Default)]
pub struct Status {
    herdr: Option<String>,
    mimo: Option<String>,
    in_herdr_pane: bool,
    plugin_installed: bool,
    plugin_is_ours: bool,
    plugin_path: Option<String>,
    shim_installed: bool,
    watchdog_bin: String,
    plugin_dir_scanned: Vec<String>,
}

pub fn collect() -> Status {
    let mut s = Status {
        herdr: herdr::herdr_version().ok(),
        in_herdr_pane: std::env::var("HERDR_ENV").as_deref() == Ok("1"),
        ..Status::default()
    };

    if let Ok(out) = std::process::Command::new("mimo").arg("--version").output()
        && out.status.success()
    {
        s.mimo = Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }

    let (path, ours) = install::plugin_state();
    s.plugin_installed = path.is_some();
    s.plugin_is_ours = ours;
    s.plugin_path = path.map(|p| p.display().to_string());

    s.shim_installed = install::shim_state().is_some();
    s.watchdog_bin = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "mimo-herdr".into());

    // Which dirs does the installed mimo actually scan? Report both candidates.
    if let Ok(dir) = install::mimo_config_dir() {
        for d in ["plugin", "plugins"] {
            if dir.join(d).is_dir() {
                s.plugin_dir_scanned.push(d.to_string());
            }
        }
    }
    s
}

pub fn print_status(json: bool) -> Result<()> {
    let s = collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&s)?);
        return Ok(());
    }
    println!(
        "herdr:              {}",
        s.herdr.as_deref().unwrap_or("NOT FOUND")
    );
    println!(
        "mimo:               {}",
        s.mimo.as_deref().unwrap_or("NOT FOUND")
    );
    println!("inside herdr pane:  {}", s.in_herdr_pane);
    println!(
        "plugin:             {} ({})",
        if s.plugin_installed {
            "installed"
        } else {
            "MISSING"
        },
        if s.plugin_is_ours {
            "mimo-herdr"
        } else {
            "foreign or unknown"
        }
    );
    if let Some(p) = &s.plugin_path {
        println!("plugin path:        {p}");
    }
    println!(
        "shim:               {}",
        if s.shim_installed {
            "installed"
        } else {
            "not installed (optional)"
        }
    );
    println!("watchdog binary:    {}", s.watchdog_bin);
    println!(
        "mimo plugin dirs:   {}",
        if s.plugin_dir_scanned.is_empty() {
            "none yet (plugin dir will be created on install)".into()
        } else {
            s.plugin_dir_scanned.join(", ")
        }
    );
    Ok(())
}
