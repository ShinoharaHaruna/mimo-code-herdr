# mimo-code-herdr

Make [MiMo Code](https://github.com/XiaomiMiMo/MiMo-Code) a first-class agent in
[herdr](https://herdr.dev): lifecycle state in the sidebar (`idle` / `working` /
`blocked`), crash-proof exit cleanup, one-command spawn — and an optional
opencode-identity mode that unlocks the full agent surface (`agent prompt`,
`agent wait`, `agent read`, native session identity).

Herdr has no native MiMo Code support. This project bridges the gap using
herdr's **official custom-integration path** — no herdr changes, no process
spoofing required for the default mode.

## Features

- **Custom-agent mode (default)** — reports `mimo` lifecycle state over
  `herdr pane report-agent --source custom:mimo-herdr` (the documented
  "Integrate your own agent" recipe), so the sidebar shows a real `mimo`
  agent with zero herdr modifications.
- **Crash-proof exit cleanup** — a detached Go/Rust-free watchdog holds a pipe
  owned by the plugin; on *any* death mode (quit, crash, SIGKILL) the kernel
  closes the pipe and the watchdog releases the agent row with an
  outranking sequence number. No `nc` dependency (unlike some alternatives).
- **One-command spawn** — `mimo-herdr spawn --name runner` creates a tab,
  launches mimo, waits for the label, and renames the agent.
- **Opencode-identity mode (optional)** — an environment-aware shim launches
  mimo under the `opencode` process identity *only inside Herdr panes*
  (pass-through to the real opencode elsewhere), enabling
  `herdr agent start --kind opencode` and therefore `agent prompt` with full
  wait/read semantics and native session identity.
- **E2E smoke test** — `mimo-herdr verify` runs the whole loop: spawn, state
  claim, prompt, reply, exit, watchdog release.
- **Idempotent install** with health checks and foreign-file detection.

## Install

### One-liner (prebuilt binaries)

```sh
curl -fsSL https://raw.githubusercontent.com/ShinoharaHaruna/mimo-code-herdr/main/install.sh | sh
```

Downloads the release binary for your platform (Linux x86_64, macOS
arm64/x86_64), verifies its SHA-256, installs it to `~/.local/bin`, and wires
up the MiMo Code plugin automatically.

### From source

```sh
git clone https://github.com/ShinoharaHaruna/mimo-code-herdr.git
cd mimo-code-herdr
make install            # cargo build --release + install plugin + copy binary to ~/.local/bin
```

### Requirements

- Linux or macOS
- [herdr](https://herdr.dev) ≥ 0.8
- MiMo Code (`mimo`) with a plugin-capable version (tested on 0.1.13)
- Rust ≥ 1.80 (only for building; the binary is self-contained)

## Usage

```sh
# 1. Install the plugin (idempotent)
mimo-herdr install

# 2. Health check
mimo-herdr status

# 3. Spawn a mimo agent (custom mode)
mimo-herdr spawn --name runner --cwd ~/Code/my-project

# 4. It appears in the herdr sidebar as `runner` / `mimo`.
#    Observe and interact from any other agent or from the CLI:
herdr agent get runner
herdr agent wait runner --until blocked --timeout 120000
herdr pane send-text w1:pX "run the experiment"
herdr pane send-keys w1:pX enter

# 5. E2E smoke test (optional)
mimo-herdr verify

# 6. Uninstall
mimo-herdr uninstall
```

### Opencode-identity mode (full `agent prompt` support)

```sh
mimo-herdr install --shim          # deploys ~/.local/bin/herdr-shim/opencode
mimo-herdr spawn --shim --name evlmimo --cwd ~/Code/my-project
herdr agent prompt evlmimo "design the experiment" --wait --timeout 300000
```

The shim is environment-aware: inside a Herdr pane (HERDR_ENV=1) it execs
`mimo` with argv[0]=`opencode`; anywhere else it passes through to the real
opencode. Keeping the shim dir at the front of PATH is therefore safe.

## Which mode should I use?

| | Custom mode (default) | Shim mode (`--shim`) |
|---|---|---|
| Sidebar label | `mimo` (honest) | `opencode` (identity spoof) |
| State lifecycle | ✅ idle/working/blocked | ✅ + native done/idle semantics |
| Discover / get / wait / read | ✅ | ✅ |
| `agent prompt` (delegation) | ❌ *(use pane send-text + agent wait)* | ✅ full surface |
| Session identity | reported but not shown by herdr for custom sources | ✅ native (`herdr:opencode`) |
| Exit row cleanup | ✅ watchdog (crash-proof) | ✅ native process detection |
| Setup | plugin only | plugin + shim in PATH |

**TL;DR**: use custom mode when you want an honest `mimo` label and pane-level
delegation is fine; use shim mode when you want other agents to
`agent prompt` this mimo directly with full wait semantics.

## How it works

1. `mimo-herdr install` copies `plugin/herdr-agent-state.js` into
   `~/.config/mimocode/plugins/` (MiMo Code auto-loads plugins from there).
2. The plugin subscribes to mimo's event bus (identical event surface to
   opencode: `session.status`, `permission.asked`, `question.asked`, …) and
   aggregates the noisy stream into `blocked > working > idle` changes.
3. On each real change it invokes the herdr CLI
   (`$HERDR_BIN_PATH pane report-agent … --source custom:mimo-herdr`), the
   official custom-integration channel documented at
   https://herdr.dev/docs/integrations/#integrate-your-own-agent. Sequence
   numbers are monotonic per source, so late reports are ignored correctly.
4. At startup the plugin spawns a detached watchdog (`mimo-herdr watch`) with
   its stdin attached to a pipe. When the plugin process dies — in any way —
   the pipe EOFs and the watchdog calls `pane release-agent` with a seq
   stamped +1s ahead, outranking every report the dead process made while
   losing to reports from any mimo started later.
5. A CLAUDECODE guard keeps the plugin silent when mimo was spawned as a tool
   of another agent, so it cannot hijack that pane's label.

## Limitations

- `agent prompt` requires an `agent start`-registered agent; custom-mode agents
  are observable and waitable but not promptable. Use pane primitives
  (`pane send-text` / `pane send-keys enter` / `agent wait` /
  `pane wait-output`) or switch to shim mode.
- Custom-source session ids are accepted by herdr but not surfaced in the
  sidebar/agent info (native integrations only) — session resume is therefore
  not available for custom-mode agents.
- MiMo Code's trust dialog ("do you trust this folder?") must be answered
  before first use in a directory; `verify` pre-trusts its scratch dir
  automatically, `spawn` does not (mimo will ask once per new project).
- `mimo-herdr install` writes the watchdog binary path at install time; after
  moving/upgrading the binary, re-run `mimo-herdr install`.

## Compatibility

| Component | Tested |
|---|---|
| herdr | 0.8.2 |
| MiMo Code | 0.1.13 |
| plugin dirs | both `plugins/` and `plugin/` are scanned by mimo; installer prefers `plugins/` |
| OS | Linux (macOS expected; Windows untested) |

## Credits

- [herdr](https://github.com/herdrdev/herdr) (Apache-2.0) — the opencode
  integration plugin's event mapping and the custom-integration documentation
  this bridge builds on.
- [junliu-mde/mimo-code-herdr-plugin](https://github.com/junliu-mde/mimo-code-herdr-plugin)
  (MIT) — the busy-set state aggregation and pipe-watchdog patterns.
- MiMo Code ([XiaomiMiMo/MiMo-Code](https://github.com/XiaomiMiMo/MiMo-Code)).

## License

MIT, see [LICENSE](LICENSE). Third-party notices in [NOTICE](NOTICE).
