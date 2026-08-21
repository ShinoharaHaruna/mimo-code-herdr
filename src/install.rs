// Idempotent install/uninstall of the MiMo Code plugin and the optional
// opencode-identity shim. The plugin is embedded in the binary (include_str!)
// so a single `mimo-herdr` release can install everything.

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

use crate::herdr::PLUGIN_FILE;

const PLUGIN_SRC: &str = include_str!("../plugin/herdr-agent-state.js");
const SHIM_SRC: &str = include_str!("../shim/opencode");

/// Resolve the MiMo Code config dir: $XDG_CONFIG_HOME/mimocode or ~/.config/mimocode.
pub fn mimo_config_dir() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.trim().is_empty()
    {
        return Ok(PathBuf::from(xdg).join("mimocode"));
    }
    let home = dirs::home_dir().context("cannot resolve home directory")?;
    Ok(home.join(".config").join("mimocode"))
}

/// MiMo Code (an opencode fork) scans both `plugin/` and `plugins/` dirs on
/// current versions. Prefer `plugins/` (opencode convention) but respect an
/// existing `plugin/` dir so we don't scatter files.
pub fn plugin_dir(config_dir: &Path) -> Result<PathBuf> {
    let plural = config_dir.join("plugins");
    let singular = config_dir.join("plugin");
    if plural.is_dir() {
        return Ok(plural);
    }
    if singular.is_dir() {
        return Ok(singular);
    }
    Ok(plural)
}

/// Build the plugin file content with the watchdog binary path baked in.
fn plugin_content() -> String {
    let bin = std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "mimo-herdr".to_string());
    PLUGIN_SRC.replace("__MIMO_HERDR_BIN__", &bin)
}

fn is_ours(content: &str) -> bool {
    content.contains("custom:mimo-herdr")
}

pub fn install(install_shim: bool) -> Result<()> {
    let config_dir = mimo_config_dir()?;
    fs::create_dir_all(&config_dir).context("failed to create mimo config dir")?;
    let dir = plugin_dir(&config_dir)?;
    fs::create_dir_all(&dir).context("failed to create mimo plugin dir")?;
    let dest = dir.join(PLUGIN_FILE);

    let content = plugin_content();
    let existing = fs::read_to_string(&dest).ok();
    match existing {
        Some(old) if old == content => {
            println!("plugin up to date: {}", dest.display());
        }
        Some(old) => {
            let origin = if is_ours(&old) {
                "a previous mimo-herdr install"
            } else {
                "a foreign plugin (e.g. herdr's opencode integration copy)"
            };
            println!("replacing {} ({})", dest.display(), origin);
            fs::write(&dest, &content).context("failed to write plugin")?;
        }
        None => {
            fs::write(&dest, &content).context("failed to write plugin")?;
            println!("installed plugin: {}", dest.display());
        }
    }

    // Claim experimental leftovers from the research phase: the herdr TUI
    // session plugin and its registration are not needed by this bridge.
    let tui_jsonc = config_dir.join("tui.jsonc");
    if let Ok(c) = fs::read_to_string(&tui_jsonc)
        && c.contains("herdr-tui-session.js")
    {
        fs::remove_file(&tui_jsonc).ok();
        println!("removed leftover: {}", tui_jsonc.display());
    }
    let tui_plugin = config_dir.join("herdr-tui-session.js");
    if tui_plugin.exists() {
        fs::remove_file(&tui_plugin).ok();
        println!("removed leftover: {}", tui_plugin.display());
    }

    if install_shim {
        install_shim_files()?;
    }
    println!("restart any running `mimo` inside herdr to pick up the plugin");
    Ok(())
}

/// Deploy the opencode-identity shim so `agent start --kind opencode` can
/// launch mimo with native Herdr semantics (full `agent prompt` support).
/// The shim is environment-aware: inside a Herdr pane it execs mimo under the
/// name "opencode"; anywhere else it passes through to the real opencode.
fn install_shim_files() -> Result<()> {
    let home = dirs::home_dir().context("cannot resolve home directory")?;
    let shim_dir = home.join(".local").join("bin").join("herdr-shim");
    fs::create_dir_all(&shim_dir).context("failed to create shim dir")?;

    let mimo_bin = resolve_executable("mimo").context("mimo not found in PATH")?;
    let real_opencode = resolve_executable("opencode").ok();

    let shim = SHIM_SRC.replace("__MIMO_BIN__", &mimo_bin).replace(
        "__OPENCODE_BIN__",
        real_opencode.as_deref().unwrap_or("opencode"),
    );
    let dest = shim_dir.join("opencode");
    fs::write(&dest, &shim).context("failed to write shim")?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))
        .context("failed to make shim executable")?;
    println!("installed shim: {}", dest.display());
    println!(
        "  (add {} to PATH to enable `agent start --kind opencode` mode)",
        shim_dir.display()
    );
    Ok(())
}

/// Find an executable on PATH, skipping the herdr-shim dir itself.
fn resolve_executable(name: &str) -> Result<String> {
    let path = std::env::var_os("PATH").context("PATH not set")?;
    let shim_dir = dirs::home_dir()
        .map(|h| h.join(".local").join("bin").join("herdr-shim"))
        .unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        if dir == shim_dir {
            continue;
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            #[allow(unused_imports)]
            use std::os::unix::fs::PermissionsExt;
            let ok = candidate
                .metadata()
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false);
            if ok {
                return Ok(candidate.to_string_lossy().to_string());
            }
        }
    }
    bail!("{name} not found in PATH")
}

pub fn uninstall(remove_shim: bool) -> Result<()> {
    let config_dir = mimo_config_dir()?;
    let dir = plugin_dir(&config_dir)?;
    let dest = dir.join(PLUGIN_FILE);
    match fs::read_to_string(&dest) {
        Ok(content) if is_ours(&content) => {
            fs::remove_file(&dest).context("failed to remove plugin")?;
            println!("removed plugin: {}", dest.display());
        }
        Ok(_) => {
            println!("not removing foreign plugin: {}", dest.display());
        }
        Err(_) => {
            println!("no plugin installed at {}", dest.display());
        }
    }
    if remove_shim {
        let home = dirs::home_dir().context("cannot resolve home directory")?;
        let shim_dir = home.join(".local").join("bin").join("herdr-shim");
        if shim_dir.exists() {
            fs::remove_dir_all(&shim_dir).context("failed to remove shim dir")?;
            println!("removed shim dir: {}", shim_dir.display());
        }
    }
    Ok(())
}

/// Report whether the plugin is installed and ours, plus the target path.
pub fn plugin_state() -> (Option<PathBuf>, bool) {
    let Ok(config_dir) = mimo_config_dir() else {
        return (None, false);
    };
    let Ok(dir) = plugin_dir(&config_dir) else {
        return (None, false);
    };
    let dest = dir.join(PLUGIN_FILE);
    match fs::read_to_string(&dest) {
        Ok(c) => (Some(dest), is_ours(&c)),
        Err(_) => (None, false),
    }
}

pub fn shim_state() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let p = home
        .join(".local")
        .join("bin")
        .join("herdr-shim")
        .join("opencode");
    p.is_file().then_some(p)
}

pub fn shim_dir() -> String {
    dirs::home_dir()
        .map(|h| {
            h.join(".local")
                .join("bin")
                .join("herdr-shim")
                .display()
                .to_string()
        })
        .unwrap_or_else(|| "~/.local/bin/herdr-shim".into())
}
