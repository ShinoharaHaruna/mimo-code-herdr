# mimo-code-herdr

> **简体中文** | [English](README.md)

让 [MiMo Code](https://github.com/XiaomiMiMo/MiMo-Code) 成为 [herdr](https://herdr.dev) 的一等公民 agent：在 sidebar 中展示生命周期状态（`idle` / `working` / `blocked`）、崩溃安全的退出清理、一键 spawn，以及可选的 opencode-identity 模式，解锁完整的 agent 能力面（`agent prompt`、`agent wait`、`agent read`、原生 session 身份）。

herdr 本身不原生支持 MiMo Code。本项目通过 herdr 的**官方 custom integration 通道**填补这一缺口——默认模式无需修改 herdr，也无需进程伪装。

## 功能特性

- **Custom-agent 模式（默认）**——通过 `herdr pane report-agent --source custom:mimo-herdr` 上报 `mimo` 生命周期状态（即文档中的 "Integrate your own agent" 方案），sidebar 会显示一个真实的 `mimo` agent，且无需对 herdr 做任何修改。
- **崩溃安全的退出清理**——一个独立运行的 watchdog（不依赖 Go/Rust 运行时）持有插件拥有的管道；无论进程以何种方式退出（正常退出、崩溃、SIGKILL），内核都会关闭管道，watchdog 随即以更高序号（outranking sequence number）释放该 agent 行。不依赖 `nc`（与某些替代方案不同）。
- **一键 spawn**——`mimo-herdr spawn --name runner` 创建 tab、启动 mimo、等待标签出现并重命名 agent。
- **Opencode-identity 模式（可选）**——一个感知环境的 shim 仅在 Herdr pane 内以 `opencode` 进程身份启动 mimo（其他场景透传给真正的 opencode），从而支持 `herdr agent start --kind opencode`，进而获得完整的 `agent prompt` wait/read 语义与原生 session 身份。
- **E2E 冒烟测试**——`mimo-herdr verify` 跑通完整链路：spawn、状态上报、prompt、回复、退出、watchdog 释放。
- **幂等安装（Idempotent install）**，内置健康检查与外部文件检测。

## 安装

### 一键安装（预编译二进制）

```sh
curl -fsSL https://raw.githubusercontent.com/ShinoharaHaruna/mimo-code-herdr/main/install.sh | sh
```

为你的平台（Linux x86_64、macOS arm64）下载 release 二进制，校验 SHA-256 后安装到 `~/.local/bin`，并自动配置 MiMo Code 插件。

### 从源码构建

```sh
git clone https://github.com/ShinoharaHaruna/mimo-code-herdr.git
cd mimo-code-herdr
make install            # cargo build --release + install plugin + copy binary to ~/.local/bin
```

### 环境要求

- Linux 或 macOS
- [herdr](https://herdr.dev) ≥ 0.8
- 支持插件的 MiMo Code（`mimo`）版本（已在 0.1.13 上测试）
- Rust ≥ 1.80（仅构建需要；二进制本身是自包含的）

## 使用方法

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

### Opencode-identity 模式（完整 `agent prompt` 支持）

```sh
mimo-herdr install --shim          # deploys ~/.local/bin/herdr-shim/opencode
mimo-herdr spawn --shim --name evlmimo --cwd ~/Code/my-project
herdr agent prompt evlmimo "design the experiment" --wait --timeout 300000
```

该 shim 能感知环境：在 Herdr pane 内（HERDR_ENV=1）它以 argv[0]=`opencode` 执行 `mimo`；在其他任何地方则透传给真正的 opencode。因此将 shim 目录放在 PATH 最前面是安全的。

## 该选哪种模式？

|  | Custom 模式（默认） | Shim 模式（`--shim`） |
|---|---|---|
| Sidebar 标签 | `mimo`（真实） | `opencode`（身份伪装） |
| 状态生命周期 | ✅ idle/working/blocked | ✅ 外加原生 done/idle 语义 |
| Discover / get / wait / read | ✅ | ✅ |
| `agent prompt`（委托） | ❌ *（改用 pane send-text + agent wait）* | ✅ 完整能力面 |
| Session 身份 | 会上报，但 herdr 对 custom source 不展示 | ✅ 原生（`herdr:opencode`） |
| 退出行清理 | ✅ watchdog（崩溃安全） | ✅ 原生进程检测 |
| 安装配置 | 仅插件 | 插件 + PATH 中的 shim |

**TL;DR**：当你想要真实的 `mimo` 标签、且 pane 级委托够用时，用 custom 模式；当你想让其他 agent 直接对该 mimo 执行带完整 wait 语义的 `agent prompt` 时，用 shim 模式。

## 工作原理

1. `mimo-herdr install` 将 `plugin/herdr-agent-state.js` 复制到 `~/.config/mimocode/plugins/`（MiMo Code 会自动从该目录加载插件）。
2. 插件订阅 mimo 的事件总线（事件面与 opencode 完全一致：`session.status`、`permission.asked`、`question.asked`、……），并将嘈杂的事件流聚合为 `blocked > working > idle` 的状态变更。
3. 每当发生真实变更时，插件调用 herdr CLI（`$HERDR_BIN_PATH pane report-agent … --source custom:mimo-herdr`）——即 https://herdr.dev/docs/integrations/#integrate-your-own-agent 中记载的官方 custom integration 通道。序号（sequence number）按 source 单调递增，因此迟到的上报会被正确忽略。
4. 启动时，插件会派生一个独立运行的 watchdog（`mimo-herdr watch`），并将其 stdin 连接到管道。无论插件进程以何种方式死亡，管道都会 EOF，watchdog 随即以超前 +1s 的序号调用 `pane release-agent`——该序号高于已死进程发出的所有上报，但低于之后启动的任何 mimo 的上报。
5. CLAUDECODE 防护：当 mimo 作为其他 agent 的工具被启动时，插件保持静默，避免劫持该 pane 的标签。

## 已知局限

- `agent prompt` 需要 `agent start` 注册过的 agent；custom 模式下的 agent 可观察、可 wait，但不可 prompt。请改用 pane 原语（`pane send-text` / `pane send-keys enter` / `agent wait` / `pane wait-output`），或切换到 shim 模式。
- herdr 接受 custom source 的 session id，但不会在 sidebar/agent 信息中展示（仅原生集成可见）——因此 custom 模式下的 agent 无法恢复 session。
- MiMo Code 的信任对话框（"do you trust this folder?"）需要在首次使用某个目录前确认；`verify` 会自动预信任其临时目录，`spawn` 不会（每个新项目 mimo 都会询问一次）。
- `mimo-herdr install` 会在安装时将 watchdog 二进制路径写入配置；移动或升级二进制后，请重新运行 `mimo-herdr install`。

## 兼容性

| 组件 | 已测试版本 |
|---|---|
| herdr | 0.8.2 |
| MiMo Code | 0.1.13 |
| 插件目录 | mimo 会同时扫描 `plugins/` 和 `plugin/`；安装器优先使用 `plugins/` |
| 操作系统 | Linux x86_64、macOS arm64（预编译）；其他平台可从源码构建 |

## 致谢

- [herdr](https://github.com/herdrdev/herdr)（Apache-2.0）——opencode 集成插件的事件映射，以及本项目所依赖的 custom integration 文档。
- [junliu-mde/mimo-code-herdr-plugin](https://github.com/junliu-mde/mimo-code-herdr-plugin)（MIT）——busy-set 状态聚合与管道 watchdog 模式。
- MiMo Code（[XiaomiMiMo/MiMo-Code](https://github.com/XiaomiMiMo/MiMo-Code)）。

## 许可证

MIT，详见 [LICENSE](LICENSE)。第三方声明见 [NOTICE](NOTICE)。
