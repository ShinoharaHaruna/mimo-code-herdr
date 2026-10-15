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
/// existing `plugin/` dir so we do not scatter files.
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

/// Escape a string for embedding inside a double-quoted JavaScript string
/// literal. A path from the filesystem must never be able to break out of the
/// template and inject code into the MiMo plugin host.
fn js_string_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

/// POSIX single-quote a path for embedding in a shell script (`'…'\''…`).
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Reject paths containing control characters (newline, NUL, …). Space and
/// other printable characters are fine: they are handled by js_string_escape
/// / shell_single_quote below. A control character in a filesystem path is a
/// hostile signal, not a normal install location.
fn has_control_chars(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

/// Build the plugin file content with the watchdog binary path baked in.
fn plugin_content() -> Result<String> {
    let bin = std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "mimo-herdr".to_string());
    if has_control_chars(&bin) {
        bail!(
            "mimo-herdr path {bin:?} contains control characters; refusing to embed it"
        );
    }
    Ok(PLUGIN_SRC.replace("__MIMO_HERDR_BIN__", &js_string_escape(&bin)))
}

fn is_ours(content: &str) -> bool {
    content.contains("custom:mimo-herdr")
}

pub fn install(install_shim: bool, force: bool) -> Result<()> {
    let config_dir = mimo_config_dir()?;
    fs::create_dir_all(&config_dir).context("failed to create mimo config dir")?;
    let dir = plugin_dir(&config_dir)?;
    fs::create_dir_all(&dir).context("failed to create mimo plugin dir")?;
    let dest = dir.join(PLUGIN_FILE);

    let content = plugin_content()?;
    let existing = fs::read_to_string(&dest).ok();
    match existing {
        Some(old) if old == content => {
            println!("plugin up to date: {}", dest.display());
        }
        Some(old) => {
            if is_ours(&old) {
                println!("replacing {} (a previous mimo-herdr install)", dest.display());
            } else if force {
                println!(
                    "replacing {} (foreign plugin; --force given)",
                    dest.display()
                );
            } else {
                bail!(
                    "{} already exists and is not managed by mimo-herdr.\n\
                     Refusing to overwrite it (it may be herdr's own integration).\n\
                     Re-run with --force to overwrite.",
                    dest.display()
                );
            }
            fs::write(&dest, &content).context("failed to write plugin")?;
        }
        None => {
            fs::write(&dest, &content).context("failed to write plugin")?;
            println!("installed plugin: {}", dest.display());
        }
    }

    // Claim experimental leftovers from the research phase: the herdr TUI
    // session plugin and its registration are not needed by this bridge.
    // Only remove the leftover plugin file itself — never the whole tui.jsonc,
    // which may hold unrelated user preferences.
    let tui_jsonc = config_dir.join("tui.jsonc");
    if let Ok(c) = fs::read_to_string(&tui_jsonc)
        && c.contains("herdr-tui-session.js")
    {
        println!(
            "note: {} still references herdr-tui-session.js; \
             remove that entry manually if you do not want the leftover plugin",
            tui_jsonc.display()
        );
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
    // Without a real opencode binary to pass through to, the shim would bake
    // the bare name "opencode" and re-enter itself when the shim dir is first
    // on PATH (fork loop). Fail closed instead.
    let real_opencode = resolve_executable("opencode")
        .context("real opencode not found in PATH; install opencode or use custom mode without --shim")?;
    if !real_opencode.starts_with('/') {
        bail!("resolved opencode must be an absolute path, got {real_opencode:?}");
    }
    if has_control_chars(&mimo_bin) || has_control_chars(&real_opencode) {
        bail!("resolved executable path contains control characters; refusing to write the shim");
    }

    let shim = SHIM_SRC
        .replace("__MIMO_BIN_Q__", &shell_single_quote(&mimo_bin))
        .replace("__OPENCODE_BIN_Q__", &shell_single_quote(&real_opencode));
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
