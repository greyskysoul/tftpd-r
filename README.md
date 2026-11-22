<div align="center">
  <img src="crates/tftpd-r-gui-slint/ui/icons/app-icon.png" alt="TftpdR 图标" width="180" height="180">
  <h1>TftpdR</h1>
  <p>纯 Rust 实现的只读 TFTP（RFC 1350）服务器，提供无头 daemon 与原生 Slint 桌面面板。</p>
  <p>
    <a href="https://github.com/greyskysoul/tftpd-r/actions/workflows/ci.yml"><img src="https://github.com/greyskysoul/tftpd-r/actions/workflows/ci.yml/badge.svg" alt="CI 状态"></a>
    <a href="https://github.com/greyskysoul/tftpd-r/releases/latest"><img src="https://img.shields.io/github/v/release/greyskysoul/tftpd-r?display_name=tag&label=release" alt="最新 Release"></a>
    <a href="https://github.com/greyskysoul/tftpd-r/releases"><img src="https://img.shields.io/github/downloads/greyskysoul/tftpd-r/total?label=downloads" alt="GitHub 下载量"></a>
  </p>
</div>

> [English README](https://github.com/greyskysoul/tftpd-r/blob/main/README.md) · 本文档为中文介绍；配置示例和 CLI 参数保留英文。

TftpdR 是一个**只读** TFTP（RFC 1350）服务器，使用 Rust 编写，包含无头 daemon 和原生 [Slint](https://slint.dev/) 桌面面板。

> **当前版本：0.3.0。** 已实现并测试 RRQ 下载、选项协商、路径沙箱、daemon CLI 与嵌入式 GUI。桌面面板支持英文/简体中文及浅色/深色主题。WRQ 上传、daemon IPC 和 Windows Service 打包明确**不在**此版本范围内。详见[功能范围](#功能范围与非目标)和 [0.3.0 发布说明](.github/release-notes/0.3.0.md)。

## 下载

Windows 和 Linux 的 x64、arm64 预编译压缩包可在[最新 Release](https://github.com/greyskysoul/tftpd-r/releases/latest)中获取。每个压缩包包含 `tftpd-r` daemon、`tftpd-r-gui` 面板、`config.example.toml`、`README.md` 和两个许可证文件。Windows 可执行文件内嵌产品图标，Windows 压缩包还附带 `tftpd-r-gui.ico`；`SHA256SUMS` 覆盖全部发布文件。

协议引擎和 daemon 可跨平台运行；桌面面板需要图形会话（Linux 下为 X11 或 Wayland）。

## 功能范围与非目标

已实现：

- 通过 UDP 提供 RFC 1350 RRQ（读取）传输；每个会话使用独立 TID socket。
- 支持 RFC 2347/2348/2349 选项协商：`blksize`、`timeout`、`tsize`，仅支持 `octet` 模式。明确拒绝 `netascii` 和 `mail`，不会静默按 `octet` 处理。
- 支持块编号回绕、整块文件的零长度终止 DATA、有限重传、重复 ACK 容错、未知 TID 的 ERROR 5，以及重复请求重发而不创建第二个会话。
- 根目录文件沙箱：拒绝路径穿越、盘符、设备路径、NTFS ADS、Windows 保留设备名和 reparse point（符号链接/联接点）。兼容旧客户端 `get /file.bin` 形式的前导 `/` 或 `\\`，映射到配置根目录，仍执行完整路径校验。
- 单端口监听一个或多个 UDP 地址（默认 `127.0.0.1:69`）；绑定全有或全无。特权端口绑定失败会提示选择非特权端口，不会自动提权或切换端口。
- 会话总数及单 peer 并发数、传输时长、文件大小和事件环均有上限。
- 无头 daemon：TOML 配置、`check-config`、`print-default-config`、结构化日志以及 Ctrl-C / Ctrl-Break / SIGTERM 优雅关闭。
- 原生 Slint GUI：服务状态、启动/停止、设置、活动日志、传输列表和最近目录选择；使用相同配置校验规则。面板支持英文和简体中文（启动时按系统语言选择），并提供跟随系统/浅色/深色主题；GUI 默认不预设根目录，并记忆最近成功启动或保存的根目录。
- 产品图标采用 Material `dns` 图形；用于面板、任务栏及 Windows 可执行文件。图标来源与许可说明见 [`ui/icons/README.md`](crates/tftpd-r-gui-slint/ui/icons/README.md)。

本版本刻意不实现（详见 `docs/adr/`）：

- **WRQ 上传**：配置 `allow_upload = true` 会被拒绝，服务端对 WRQ 返回 ERROR 2。上传需先完成单独安全评审与原子临时文件设计。
- **IPC / GUI 托管 daemon**：GUI 以嵌入模式运行服务，尚无 named-pipe 协议（[ADR 0004](docs/adr/0004-embedded-only.md)）。
- Windows Service 注册、系统托盘、自动更新、远程管理与用户/角色管理。
- `netascii` 模式（[ADR 0006](docs/adr/0006-octet-only.md)）。

默认端口 69、多地址监听及 GUI 的首次运行行为分别见 [ADR 0007](docs/adr/0007-default-port-69.md)、[ADR 0008](docs/adr/0008-multiple-bind-addresses.md)、[ADR 0009](docs/adr/0009-gui-first-run-and-auto-restart.md)。GUI 根目录记忆、日志导出、目录选择器预热以及多语言/主题决策分别见 [ADR 0012](docs/adr/0012-gui-root-selection-and-memory.md)、[ADR 0013](docs/adr/0013-gui-activity-log.md)、[ADR 0014](docs/adr/0014-prewarmed-folder-picker-helper.md)、[ADR 0015](docs/adr/0015-gui-language-and-theme.md)。

## 系统要求

- Rust **1.100.0** 是本工作区开发与验证使用的工具链；尚未声明 MSRV，未测试更旧的编译器。
- Windows 10/11 为主要目标。协议引擎跨平台，完整测试也在 CI 的 Linux 环境运行；桌面面板目前仅在 Windows 手动运行验证。
- Windows 构建需要 linker：默认 `x86_64-pc-windows-msvc` 工具链使用 MSVC Build Tools（`link.exe`），`x86_64-pc-windows-gnu` 使用 MinGW-w64。GNU 构建还需要 MinGW-w64 binutils 的 `windres` 来编译图标资源。验证记录见[测试文档](docs/testing.md)。

## 构建

```powershell
cargo build --workspace            # debug
cargo build --workspace --release  # release binaries in target\release
```

Release 使用优先减小体积的配置（`opt-level = "z"`、fat LTO、`panic = "abort"` 和符号剥离）。GNU x64 验证构建的 daemon 约 1.01 MiB、GUI 约 8.62 MiB（含约 110 KiB Windows 图标资源）。Release 中 panic 会终止进程，Debug 和测试构建仍使用默认 unwind。daemon 的日志过滤避免引入 `regex` 引擎，详见 [ADR 0010](docs/adr/0010-size-first-release-profile.md)、[ADR 0011](docs/adr/0011-windows-executable-icon.md) 和 [ADR 0016](docs/adr/0016-log-filter-without-env-filter.md)。

工作区包含三个运行时 crate 和一个构建期辅助 crate：

| Crate | 用途 |
| --- | --- |
| `tftpd-r-core` | 协议、会话引擎、沙箱、配置、事件；不依赖 GUI/daemon/IPC。 |
| `tftpd-r-daemon` | 无头服务端程序：CLI、TOML 配置、日志、优雅关闭。 |
| `tftpd-r-gui-slint` | 原生 Slint 桌面面板；嵌入 `tftpd-r-core`，无需 daemon。 |
| `tftpd-r-build-support` | 构建期生成 Windows 图标资源，不链接进可执行文件。 |

## 快速开始：daemon

```powershell
# 1. 准备共享目录（示例配置默认使用此目录）
New-Item -ItemType Directory -Force tftp-root | Out-Null
Set-Content tftp-root\hello.txt "hello over tftp"

# 2. 校验配置（不会绑定 socket）
Copy-Item config.example.toml config.toml
cargo run -p tftpd-r-daemon -- check-config --config config.toml

# 3. 前台启动；Ctrl-C 或 Ctrl-Break 可优雅停止
# Linux 绑定 69 端口需要 root/CAP_NET_BIND_SERVICE；Windows 通常允许普通用户绑定，除非该端口已被保留。
# 可通过高端口进行试运行：
cargo run -p tftpd-r-daemon -- serve --config config.toml --bind 127.0.0.1:6969
```

然后让 TFTP 客户端连接 `127.0.0.1:69`（使用上述覆盖参数时为 `127.0.0.1:6969`），请求 `hello.txt`。服务端只读，不接受上传。若权限不足导致端口 69 绑定失败，程序会提示可用的非特权端口；不会自动切换端口或提权。

CLI 命令概览：

```text
tftpd-r serve --config config.toml [--bind 0.0.0.0:6969] [--root DIR]
tftpd-r check-config --config config.toml
tftpd-r print-default-config
```

退出码：`0` 成功，`2` 配置/CLI 校验失败，`1` 启动或运行时服务失败。`--bind`、`--root` 可覆盖配置文件并会再次校验。`TFTPD_R_LOG`（或 `--log`）设置日志级别，例如 `TFTPD_R_LOG=debug`。支持级别（`off`/`error`/`warn`/`info`/`debug`/`trace` 或 `0`–`5`）以及逗号分隔的 `target=level` 指令，例如 `TFTPD_R_LOG=tftpd_r_core=debug,info`；裸 `target` 表示 `target=trace`，目标前缀匹配其下级名称。不支持 span/field 级指令。若未指定裸级别，默认级别仍为 `info`，因此仅有目标指令时会收窄对应目标，不会静默其他日志。非法指令会报告到 stderr 并忽略。详见 [ADR 0016](docs/adr/0016-log-filter-without-env-filter.md)。

兼容旧客户端的 `get /hello.txt`：请求文件名前导 `/` 或 `\\` 会映射到配置根目录，而不是被当作主机绝对路径；`/../secret` 等路径穿越仍会被拒绝。

## 快速开始：GUI

```powershell
cargo run -p tftpd-r-gui-slint
```

如果当前目录存在 `config.toml`，GUI 会加载但不会改写它。首次启动且没有可用配置时，设置面板会自动打开一次，供用户选择根目录和监听地址；表单默认选中“所有网络”（`0.0.0.0`）。daemon 和 `ServerConfig::default()` 仍采用更保守的回环地址默认值。

GUI **没有默认根目录**。选择目录前，状态栏显示 `No root directory` / `未指定根目录`，设置栏位保持空白；启动或保存时会要求先选择目录，而不是使用 `tftp-root` 之类占位目录。最近一次成功启动或在设置中保存的根目录，会记录到 Windows `%APPDATA%\TftpdR\last-root.txt`（其他系统使用 XDG 配置目录），下次启动时在没有有效 `config.toml` 根目录配置的情况下恢复。有效 `config.toml` 始终优先，适合部署时固定根目录。

### 语言

面板支持**英文和简体中文**，启动时根据系统 locale 选择：简体中文 locale（`zh`、`zh-CN`、`zh_CN`、`zh-Hans*`、`zh-SG`）显示中文；其他语言（包括繁体中文 `zh-TW`、`zh-HK`、`zh-MO`、`zh-Hant*`）和无法读取 locale 时显示**英文**。没有语言选择器；切换语言需要重启面板。英文为 `@tr(...)` 源文案，简体中文翻译内嵌于程序，不需要在可执行文件旁部署翻译文件。关于窗口会显示当前语言。`TFTPD_R_GUI_LANGUAGE=en|zh-CN` 仅用于开发测试，不会显示在界面上。

### 外观

设置 → **外观**提供“跟随系统”“浅色”“深色”。默认跟随系统：启动时读取 Windows `AppsUseLightTheme` 或 Linux GNOME `color-scheme` / `GTK_THEME`，跟随系统时每 30 秒重新检查并即时应用，无需重启。手动选择浅色或深色会立即生效并保存到 `ui-prefs.txt`，不受系统主题影响。外观是纯显示偏好，不会改变服务配置，也不会重启服务；偏好写入失败时会记录 WARN，但当前会话仍应用所选主题。

### 面板布局与行为

- **窗口与工具栏**：无边框窗口，标题栏含应用名和最小化/最大化/关闭控件。工具栏左侧显示服务状态，提供启动/停止、设置和关于按钮；运行时“启动”切换为“重启”。停止过程异步执行，不阻塞 UI。
- **主区域**：活动日志（最新事件在前）和当前传输列表（文件、客户端、进度、大小、状态）。
- **状态栏**：显示提示或错误、监听地址和当前根目录。点击根目录名可从最近目录中选择；最近目录菜单的“选择…”会立即应用选定目录，“清空”只清除历史，不影响当前根目录。
- **设置面板**：使用与 daemon 相同的 `ServerConfig::validate` 规则。提供端口、多选监听地址、根目录选择和传输限制。浏览选择目录只填入表单，需点击保存设置才生效；监听地址共用端口设置，“所有网络”与具体地址互斥。
- **自动重启**：服务运行且无进行中传输时，保存配置或切换根目录会重启服务；若传输仍在进行，则仅保存新配置并提示手动重启，避免中断下载。切换外观不会重启服务。
- **活动日志**：内存最多保留 500 条，按本地时间显示，消息会合并为单行；级别筛选只隐藏条目，不会删除数据，导出仍包含所有保留记录。导出需用户主动选择保存路径；导出文件包含本地路径和 peer 地址，请谨慎分享。

最近使用的目录最多保存 8 条，存放于 Windows `%APPDATA%\TftpdR\recent-dirs.txt` 或 Unix `$XDG_CONFIG_HOME/tftpd-r/recent-dirs.txt`（默认 `~/.config/tftpd-r/`）。根目录记忆、首次运行标记、外观选择分别记录于 `last-root.txt`、`ui-state.txt` 和 `ui-prefs.txt`。若用户配置目录不可写，程序会依次回退到可执行文件旁或当前工作目录下的 `.tftpd-r`，并记录 WARN 及实际位置。关闭窗口时会停止嵌入式服务，并在 `shutdown_grace_secs` 限制时间内等待活动传输退出。

## 图标

工具栏、面板和对话框图标来自 [google/material-design-icons](https://github.com/google/material-design-icons)，采用 24×24 `materialicons` 样式，许可证为 Apache-2.0。向量路径以内嵌 Slint `Path` 命令绘制，可在不同尺寸保持清晰。产品图标与各图标的来源及完整许可证见 [`crates/tftpd-r-gui-slint/ui/icons/README.md`](crates/tftpd-r-gui-slint/ui/icons/README.md) 和 [`crates/tftpd-r-gui-slint/ui/icons/LICENSE`](crates/tftpd-r-gui-slint/ui/icons/LICENSE)。

## 配置

`config.example.toml` 是唯一权威且带注释的配置示例，也被内嵌到 daemon 中（`print-default-config` 输出与该文件完全一致）：

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `port` | `69` | 所有监听地址共用的 UDP 端口。Linux 绑定 69 需要 root / `CAP_NET_BIND_SERVICE`；Windows 通常允许普通用户绑定，除非端口已被保留。权限不足时程序会提示选择其他端口（例如 `6969`），不会自动更改。 |
| `bind_addresses` | `["127.0.0.1"]` | 1–16 个监听地址。`0.0.0.0` / `::` 表示监听该地址族的所有网卡；同族其他地址会被视为冗余。拒绝多播/广播。除非确实要向局域网提供服务，否则建议保持回环地址。 |
| `root_dir` | `tftp-root` | 必须存在且为目录。这是 TOML/daemon 示例默认值；GUI 初始状态不选择根目录。 |
| `allow_upload` | `false` | 当前版本必须为 `false`；设为 `true` 会导致校验失败。 |
| `overwrite` | `false` | 依赖上传功能；当前版本不支持，会被拒绝。 |
| `max_file_bytes` | 64 MiB | 超过限制的文件会被拒绝。 |
| `block_size` | `512` | 协商 `blksize` 的上限（8..=65464）；可为局域网提高。 |
| `timeout_secs` | `3` | 默认重传超时（1..=255）；客户端可用 `timeout` 选项覆盖。 |
| `max_retries` | `5` | 传输失败前的重传次数（1..=20）。 |
| `max_sessions` | `16` | 最大并发传输数（1..=1024）；每个 peer IP 同时最多 8 个会话。 |
| `max_transfer_duration_secs` | `3600` | 单次传输硬时限（秒）。 |
| `shutdown_grace_secs` | `5` | 关闭服务时等待传输完成的最长时间（1..=60 秒）。 |
| `event_buffer` | `1024` | 内存事件环容量（16..=65536）。 |

未知配置字段会被拒绝，避免拼写错误时静默使用默认值。0.1.0 配置中的 `bind_address = "127.0.0.1:6969"` 已被 `port` + `bind_addresses` 替代，旧字段会触发 unknown-field 错误。详见 [docs/roadmap.md](docs/roadmap.md) 和 [ADR 0008](docs/adr/0008-multiple-bind-addresses.md)。

## 端口与防火墙

- 服务在所有 `bind_addresses` 地址的 `port` 上监听 **UDP**（默认 `127.0.0.1:69`）。Linux 绑定端口 69 需要 root 或 `CAP_NET_BIND_SERVICE`；Windows 通常允许普通用户绑定，除非端口被系统保留。权限不足时会提示高端口（如 `6969`），不会自动更改配置。监听多个地址时，防火墙需为可访问的地址配置规则。
- 每个传输还会使用一个额外的**临时 UDP 端口**（TID），地址族与接收请求的监听 socket 相同。防火墙需要允许访问监听端口及临时端口返回的流量。Windows Defender Firewall 对有状态/回环流量通常会自动放行，但限制策略可能需要显式规则；本仓库未验证这一点。
- 绑定 `0.0.0.0` 或 `[::]` 会向网络开放服务。TFTP 本身没有身份验证，文件访问范围仅受共享目录与只读策略限制。指定多个具体地址不会额外开放未列出的网卡；任一地址绑定失败都会导致整体启动失败，不会仅部分监听。

## 安全默认值

- 只读，不允许上传；默认绑定回环地址；手写代码不含 `unsafe`。
- 路径沙箱拒绝穿越、盘符、设备名、NTFS ADS 和 reparse point；打开后对 canonical path 做组件级校验。旧客户端的 `/file.bin` 映射至根目录下的文件，不会被视为主机绝对路径。
- 并发数、文件大小、重传次数、传输时长与事件内存均受限。
- 发给客户端的 ERROR 文本不会包含本机路径或原始系统错误；日志文件名会清理并限制长度。
- TFTP 无加密/认证；路径检查存在 TOCTOU 等残余风险，UDP 源地址也可伪造。请阅读[安全说明](docs/security.md)，不要将服务直接暴露到公网。

## 测试

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

测试覆盖协议 codec、配置和路径沙箱、29 个 loopback UDP RRQ 集成测试（包括块编号回绕、关闭取消、旧客户端 `/file` 请求、多监听同端口和全有或全无绑定）、daemon CLI/配置集成测试，以及 GUI 控制器、根目录记忆、接口地址解析、目录选择器 UTF-8 协议、地址选择模型、自动重启判定、多语言翻译目录和系统主题侦测等。**不要将构建通过等同于目标机运行验证**；实际已执行检查及未覆盖项目见[测试文档](docs/testing.md)。

## 运行时依赖

两个二进制只依赖 Windows 系统 DLL 和 Rust 工具链提供的 CRT，不需要额外安装运行时或 redistributable。验证构建使用 GNU 工具链并链接 `msvcrt.dll`；MSVC 构建通常链接 Visual C++ runtime。

验证机器测得的 Release 二进制大小记录于[测试文档](docs/testing.md)，仅作参考，不是验收指标。

## 文档

- [架构说明](docs/architecture.md) — crate 边界、生命周期、数据流与线程模型。
- [安全说明](docs/security.md) — 威胁模型、沙箱和资源限制。
- [测试与验证](docs/testing.md) — 自动化/手动验证记录及未完成验证项。
- [ADR 决策记录](docs/adr/) — 已接受的设计决策。
- [开发指南](AGENTS.md) — 后续维护、开发工作流与不变量（中文）。

## 许可证

本项目依照 Apache License 2.0 或 MIT License（由使用者选择）授权，详见 [LICENSE-APACHE](LICENSE-APACHE) 与 [LICENSE-MIT](LICENSE-MIT)。内嵌 Material Design 图标路径数据使用 Apache-2.0；图标许可与来源说明见 [`crates/tftpd-r-gui-slint/ui/icons/`](crates/tftpd-r-gui-slint/ui/icons/)。
