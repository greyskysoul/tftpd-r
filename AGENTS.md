# AGENTS.md — 后续开发指南

> 面向后续 AI 编码代理与维护者。开始修改前，先阅读本文件、[README.md](README.md)、[docs/architecture.md](docs/architecture.md)、[docs/security.md](docs/security.md)、[docs/testing.md](docs/testing.md) 和 [docs/adr/](docs/adr/)。
>
> **重要：** 仓库根目录的《Rust + Slint 无 WebView 依赖解耦式 TFTP 服务端架构设计方案.md》是**本地规划资料**，不是版本库内容。它被 `.git/info/exclude` 忽略，**禁止以任何方式提交进 git**（包括 `git add -f`、复制到 docs、或把内容粘贴进其他受跟踪文件）。需要更新计划时直接编辑该本地文件，不要提交。

## 1. 项目定位与当前状态

纯 Rust 的**只读** TFTP（RFC 1350）服务端，Cargo Workspace 分为协议核心、无头 daemon 和 Slint GUI（另有仅构建期使用的 `tftpd-r-build-support`，不链接进产物）。GUI 使用 winit + 软件渲染器；协议核心不依赖 GUI、daemon 或 IPC。

| 阶段 | 状态 | 说明 |
| --- | --- | --- |
| Phase 0–5 | ✅ 已完成 | workspace、codec/沙箱、RRQ 引擎、daemon、GUI 面板（无边框标题栏、工具栏、状态栏、日志/传输列表、设置/关于、最近根目录）、文档/ADR/CI |
| Phase 6 目标机验证/发布收尾 | 🟡 部分完成 | 0.1.0、0.2.0、0.2.1、0.2.2 已发布（Windows/Linux × x64/arm64）；Windows/Linux CI 已通过；真实/老式客户端、LAN/防火墙、Linux GUI、RDP、IME、其他 DPI、目标机运行仍待完成 |
| Phase 7 IPC | ⏳ 未开始 | GUI 仅 Embedded 模式，无 named pipe/daemon 控制 |
| Phase 8 WRQ 上传 | ⏳ 未开始 | 只读是默认且唯一行为，`allow_upload=true` 被校验拒绝 |
| 0.2.0 R1/R2 | ✅ 已发布（2026-11-20） | 默认端口 69（ADR 0007）；`port` + `bind_addresses` 多监听、GUI 多选地址与目录选择器（ADR 0008）；首次启动自动设置、空闲时自动重启（ADR 0009） |
| release 体积优化 | ✅ 已完成（GNU x64） | `opt-level="z"` + fat LTO + `panic="abort"` + strip（ADR 0010） |
| Windows exe 图标 | ✅ 已完成（GNU/MSVC x64） | `tftpd-r-build-support` 生成 `.res`；7 帧图标（ADR 0011） |
| 0.2.1 发布 | ✅ 已发布 | release workflow 44882456726；MSVC/Linux 测试门与四目标构建全绿 |
| 0.2.2 发布 | ✅ 已发布（2026-11-22） | release workflow 44976411968；测试门与四目标构建全绿；MSVC x64 归档 SHA-256 与 `SHA256SUMS`/GitHub digest 一致 |
| GUI 根目录未指定/记忆 | ✅ 已发布（0.2.2） | 空根目录、`last-root.txt`、`config.toml` 优先（ADR 0012） |
| 根目录菜单 + 日志 | ✅ 已发布（0.2.2） | 菜单「选择…」立即应用；日志本地时钟/单行/级别下拉筛选/导出（ADR 0012 修订、ADR 0013 及其 2026-11-22 更新） |
| 保存提示/诊断/便携状态 | ✅ 已发布（0.2.2） | 设置页内联提示；`LastRoot` 写入失败/失效/覆盖 WARN/INFO；per-user 目录不可写时回退 `.tftpd-r` |

已验证环境：Windows 11（10.0.26300）+ Rust 1.100.0 `x86_64-pc-windows-gnu` + MinGW-w64 GCC 15.3 + Slint 1.19.0；CI 另在 `windows-latest`（MSVC x64）与 `ubuntu-latest`（GNU x64）跑 fmt/clippy/test，并在原生 arm64 runner 构建。**未验证**：MSVC 交互式 GUI/长期服务、Linux/arm64 产物运行、Linux 桌面 GUI、真实/外部 TFTP 客户端、Windows Service、RDP、69 端口权限、真实键盘/IME、其他 DPI/语言渲染与无障碍。不要把未验证项写成已通过。

## 2. 仓库地图

```text
.
├── Cargo.toml                     # workspace、共享依赖、lint、release profile
├── Cargo.lock                     # 应用 workspace，必须提交
├── config.example.toml            # 唯一安全默认配置示例（daemon 内嵌同一份）
├── README.md
├── AGENTS.md
├── docs/
│   ├── architecture.md            # crate 边界、生命周期、数据流、线程模型
│   ├── security.md                # 威胁模型、沙箱、残余风险
│   ├── testing.md                 # 实际执行记录与未验证项
│   ├── roadmap.md                 # 已发布项与后续未实现项
│   └── adr/0001–0014              # 已接受决策
├── crates/
│   ├── tftpd-r-build-support/    # 构建期图标资源，不链接进产物
│   ├── tftpd-r-core/             # 无 GUI/daemon/IPC 依赖
│   │   ├── src/{lib,config,error,events,filesystem,server,session}.rs
│   │   ├── src/protocol/{mod,packet,codec,options}.rs
│   │   └── tests/rrq.rs           # 真实 loopback UDP 集成测试
│   ├── tftpd-r-daemon/           # CLI + TOML + tracing + 信号处理
│   │   ├── src/{main,cli,config_file,logging,shutdown}.rs
│   │   ├── build.rs               # exe 图标
│   │   └── tests/cli.rs
│   └── tftpd-r-gui-slint/        # Slint 面板 + EmbeddedController
│       ├── src/{main,controller,recent,netif,folder_picker,user_state,logging,log_export}.rs
│       ├── ui/{app,titlebar,toolbar,statusbar,logs,transfers,settings,about,
│       │       root-menu,widgets,theme,icons,models}.slint
│       ├── ui/icons/              # Material 图标来源、产品图标
│       └── build.rs               # Slint 编译 + exe 图标
├── .github/workflows/{ci,release}.yml
├── .github/release-notes/
└── （未跟踪）本地计划文档 .md      # 禁止提交
```

依赖方向保持：`tftpd-r-core` ← `tftpd-r-daemon`；`tftpd-r-core` ← `tftpd-r-gui-slint`。`tftpd-r-build-support` 只作为两个二进制 crate 的 build-dependency。GUI 不依赖 daemon；未来 IPC 的 DTO/协议放在独立 crate（如 `tftpd-r-control-protocol`），不要污染 core。

## 3. 不可破坏的契约

### 3.1 协议行为

- 只服务 RFC 1350 **RRQ**；WRQ 返回 ERROR 2（"Uploads are disabled"），不得悄悄实现写入。
- 仅 `octet`。`netascii`/`mail` 显式拒绝（ERROR 0 + 明确消息）；未知 mode 在 codec 层 `UnsupportedMode`，主监听端口回 ERROR 4。**不得把 netascii 静默当 octet。**
- 每个会话独立 UDP socket/TID；只接受预期 peer（IP+端口）的包；其他来源回 ERROR 5，每个 block 交换最多 8 次。
- 初始请求在监听端口处理；同一 peer 的重复 RRQ 必须 `SessionSignal::Resend` 给已有会话，**不能新建第二个会话/TID**。
- DATA/ACK：等待匹配 ACK；重复/过期 ACK 忽略；重传必须发送与上次完全相同的已编码包；重传次数受 `max_retries`，总时长受 `max_transfer_duration_secs`。
- 最终块规则：payload < block size 即结束；文件长度为块大小整数倍时必须发送零长度 DATA 终止块；块号 16 位回绕（65535 → 0）必须正确。
- 选项：`blksize` 8..=65464 且不超过配置 cap；`timeout` 1..=255；`tsize` 返回大小；未知选项忽略；重复/非法已知选项回 ERROR 8。请求 datagram 上限 1024 字节，超长/畸形回 ERROR 4。
- 客户端可见 ERROR 文本必须是固定、脱敏常量；**不得包含本地路径、OS 原始错误或内部结构**。文件名进入事件/日志前必须 `sanitize_display_name`。
- 默认只读、默认绑定回环、默认不开放上传；任何默认值改动需要 ADR 与安全评审。

### 3.2 路径与文件系统沙箱

实现核心在 [`crates/tftpd-r-core/src/filesystem.rs`](crates/tftpd-r-core/src/filesystem.rs)。必须保持：

1. **老客户端根目录前缀兼容**：文件名前导的任意 `/` 或 `\` 被剥离后相对根目录解析（`get /firmware.bin` → `<root>/firmware.bin`）。只剥离**前导**分隔符；`a//b`、尾随分隔符仍拒绝；仅由分隔符组成的 `/` 表示根目录本身，作为无效文件名拒绝（TFTP 不传目录）。
2. 剥离后仍拒绝：`..`、`.`、空组件、盘符/`:`（阻断 NTFS ADS）、Windows 保留设备名（`CON`/`NUL`/`COM1`…）、尾随点/空格、`< > " | ? *`、控制字符和 NUL。
3. 根目录在 `Sandbox::new` 时 canonicalize；逐组件 `symlink_metadata` 检查，拒绝 reparse point/symlink；只允许普通文件；大小上限在 open 前后都检查。
4. open 后再次 canonicalize，并按**路径组件**（不是字符串前缀）与 canonical root 比较；Windows 大小写不敏感。`starts_with("C:\\root")` 永远不算安全。
5. 残余风险（TOCTOU、根目录内硬链接）见 [docs/security.md](docs/security.md)；**不要声称沙箱绝对安全**。上传/写入路径必须重新设计、ADR、原子临时文件与安全测试。
6. 相关测试：`root_prefixed_names_map_to_the_root`、`opens_root_prefixed_file_without_leading_slash_in_display_name`、`rejects_traversal_and_drive_paths`、`root_prefixed_filename_is_served`、`root_prefix_does_not_escape_the_sandbox`、symlink/reparse。修改命名逻辑必须同时保留正反向用例。

### 3.3 配置与 GUI 状态

- `ServerConfig` 使用 serde `deny_unknown_fields` + 安全默认值；`validate()` 是 daemon、GUI 和测试共用的唯一规则来源。
- `allow_upload=true`、`overwrite=true` 当前必须是**配置错误**（上传未实现，不能给运维错误暗示）。
- 监听配置是 `port` + `bind_addresses`（1..=16 个）。默认 `127.0.0.1:69`（Linux 需 root/CAP_NET_BIND_SERVICE；Windows 低端口通常允许但可能保留）；端口 0 仅测试；重复地址去重；同族通配 `0.0.0.0`/`::` 折叠；多播/广播拒绝（ADR 0008）。绑定**全有或全无**，不得静默只服务部分地址。GUI 无有效 `config.toml` 时表单默认选中 `0.0.0.0`（ADR 0009），核心/daemon 默认仍是回环；不要把 GUI 表单默认写进 `ServerConfig::default()`。
- GUI 根目录默认**未指定**（空 `PathBuf`，仅 GUI 内部约定）：状态栏「未指定根目录」、设置页留空、启动/保存提示「请先选择根目录。」。`last-root.txt` 记录上次成功启动或显式保存的根目录，启动时在无有效 `config.toml` 时恢复；`config.toml` 的 `root_dir` 永远优先。设置页「浏览…」只改表单，选中后必须在表单内显示「点击保存设置后生效」，不能只写进被模态框遮挡的日志；`last-root.txt` 写入失败、启动时失效条目、`config.toml` 覆盖记忆值都必须有可见日志。
- 三个状态文件优先写入 per-user 配置目录（Windows `%APPDATA%\TftpdR`，Unix `$XDG_CONFIG_HOME/tftpd-r` 或 `~/.config/tftpd-r`）。该目录不可创建/不可写（受限 profile、Low 完整性、沙箱）时，依次回退到 exe 旁 `.tftpd-r`、工作目录 `.tftpd-r`，启动日志 WARN 说明实际目录；不得因 `%APPDATA%` 写入失败放弃根目录记忆/最近历史/首次启动标记。
- 不要把空根目录写进 `ServerConfig::default()`/daemon；不要把 `tftp-root` 当作 GUI 占位值显示或写入（ADR 0012）。
- 绑定特权端口失败必须给出可操作提示，**不得**静默回退、提权或改写配置（ADR 0007）。
- GUI 网卡枚举与目录/保存对话框通过固定绝对路径的 PowerShell/.NET（Unix 为 `hostname`/`zenity`/`kdialog`）辅助进程完成，后台线程执行；不得引入 shell、用户可控参数、提权或手写 unsafe。辅助失败时必须降级或给出可操作错误，不得崩溃。Windows 目录选择器使用**按需预热的常驻 helper**（ADR 0014）：设置页或最近目录菜单打开后预热，路径经 UTF-8 stdin/stdout 传递（不得用 `[Console]::Out` 输出路径，OEM 代码页会破坏非 ASCII 名称）；helper 启动失败或退出时回退一次性 helper；面板关闭时必须 kill helper（包括仍打开的对话框），不得遗留 PowerShell 进程。预热只在用户实际打开相关对话框时发生，不要在启动时无条件常驻。
- `root_dir` 必须存在且为目录；低端口权限按平台处理，README/security 与 ADR 0007 的表述必须一致，不要笼统写成“Windows 需要管理员”。
- `config.example.toml` 由 daemon `include_str!` 内嵌，`print-default-config` 必须与文件完全一致；修改示例时同步测试。

### 3.4 资源、事件与日志

- 全局 `max_sessions` 信号量 + 每 IP 最多 8 个并发会话；请求大小、文件大小、重传次数、总时长、事件环、GUI 日志行数都有上限；新增资源必须有明确上限。
- `EventSink::emit` 必须短小、非阻塞、不得 panic；GUI 通过 `EventBus::drain_since(seq)` 轮询，不直接由 Tokio task 改 UI。
- 生命周期：`Starting → Running { local_addrs } → Stopping → Stopped`（或 `Failed`）；每个监听地址一个 accept loop，peer 表/会话上限/会话 ID 全局共享，会话 socket 绑定该监听地址（非通配时）；`ServerHandle::stop()` 幂等，先取消 accept loop，再在 `shutdown_grace_secs` 内并行 drain，最后 abort 剩余任务并释放 socket/permit/文件。
- GUI `stop()` 必须异步/轮询，不能阻塞 UI 线程等待宽限期；关闭窗口后 `EmbeddedController::shutdown()` 负责最终清理。
- GUI 配置/根目录变更自动重启只在 `Starting`/`Running`、配置确实变化且 `state.transfers` 为空时发生；有传输时只保存并 WARN，不得中断传输或排队自动重启。`Stopping` 绝不能被变更改回启动。`poll_status` 在 handle 被 `stop()` 取走后不得继续报告陈旧的 `Running`。
- GUI 活动日志只驻留内存（≤500 行，最新在前），时间用本地 `HH:MM:SS`；级别筛选只影响显示，不得删除条目；消息入库前必须折叠为单行；打开/取消目录选择器和保存对话框不得写日志；导出必须由用户显式点击并通过原生保存对话框选择路径，Rust 侧只写文本文件，不自动落盘。状态目录回退到便携 `.tftpd-r` 时必须在启动日志 WARN 并给出实际目录。详见 ADR 0013。

### 3.5 代码与依赖

- 全 workspace `unsafe_code = "deny"`；手写代码禁止新增 `unsafe`。Slint 生成代码的 `#[allow(unsafe_code)]` 是生成器例外，不要借此放宽手写代码。
- 不新增无必要依赖；新增依赖需核对许可证、维护状态、MSRV、Windows 支持、features 与传递依赖，并提交更新后的 `Cargo.lock`。
- 不引入 WebView；GUI 保持 Slint + software renderer（除非有 ADR 和实测依据）。
- release profile 体积优先：`opt-level="z"`、`lto="fat"`、`codegen-units=1`、`panic="abort"`、`strip="symbols"`（ADR 0010）。release 中 panic 终止进程，debug/test 仍 unwind；改 profile 必须先重测并同步 ADR/`docs/testing.md`。
- Windows exe 图标由 [`tftpd-r-build-support`](crates/tftpd-r-build-support/src/lib.rs) 从 `app-icon.ico` 生成 `.res`：MSVC 交给 `link.exe`，GNU 用 `windres` 转 COFF，非 Windows no-op。不要提交 `.res`/`.o` 生成物，不要手改生成内容；GNU 缺少 `windres` 必须明确报错。更换图标时同步更新 build-support 单测中的固定帧尺寸/字节数。
- `Cargo.lock` 属于应用 workspace，必须提交。不要提交 `target/`、`.dsh-toolchain/`、`.bootstrap/`、本地计划文档。

## 4. 开发工作流

### 4.1 开始前

1. `git status`，确认没有未提交/未检查的改动；不要覆盖用户工作。
2. 阅读本文件、README、相关 ADR、`docs/security.md` 与代码；先画模块/调用链，再动手。
3. 确认平台、Rust、Slint/Tokio 版本；基线是 Rust 1.100.0（GNU）。MSVC/Linux 未验证，不要假设。
4. 若 `.bootstrap/env.ps1` 存在，先 `Set-ExecutionPolicy -Scope Process Bypass -Force; . .\.bootstrap\env.ps1`，否则用系统 Rust。沙箱内 Cargo 可能因 Schannel 报 `SEC_E_NO_CREDENTIALS`；依赖已缓存时用 `--offline`，不要为环境限制改项目代码。

### 4.2 变更流程

1. **先复现/先写测试**：bug 或行为变更应先有失败测试（单元或 loopback UDP 集成），再实现。
2. 改动尽量小、可审查；保持 crate 依赖方向。
3. 同步更新测试；安全相关必须同时有正向和反向用例。
4. 更新文档：行为/架构变化改 README、architecture、security、ADR；验证结果改 `docs/testing.md`；计划变化只编辑本地计划文档（不提交）。
5. 运行验证命令并记录真实输出；不要声称未运行的测试通过。

### 4.3 必须运行的验证

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

当前自动化基线是 **177 tests passed**：build-support 3、core 单元 61、RRQ UDP 集成 29、daemon 单元 9、daemon CLI 集成 5、GUI 单元 69（控制器 5 + 最近目录 7 + 网卡枚举 4 + 目录选择 4 + 用户状态 12 + 日志 9 + 保存对话框 2 + 根目录/地址模型/首次启动/自动重启/产品图标/日志模型/轮询 26）、doctest 1；另有 1 个 ignored 的 Windows 集成测试（`the_warm_helper_closes_its_dialog_on_shutdown`，需桌面会话，手动运行）。

其中 build-support 覆盖 ICO 解析（7 帧、截断/非法/越界拒绝）与 `.res` 的 null 资源、`RT_ICON`/`RT_GROUP_ICON`、帧字节与对齐；GUI 根目录相关 13 项覆盖 `LastRoot` 持久化/失效/只读降级、状态目录选择/便携回退、启动优先级（`config.toml` > 记忆根目录 > 未指定）、空根目录拒绝与「未指定根目录」；日志相关 14 项覆盖级别/筛选/单行化/500 行上限/导出格式与写文件/轮询目标；目录选择 4 项覆盖目录校验、非 ASCII 目录名与 `PICK:` 协议解析，ignored 集成测试覆盖预热 helper 的启动、真实弹窗与关闭。最新复验见 `docs/testing.md`。

release 体积基线（GNU x64，2026-11-22，含 112,640 B 图标资源、日志级别下拉框与无分隔线表头；ADR 0010/0011/0012/0013/0014）：`target/release/tftpd-r.exe` **1,357,824 B（1.29 MiB）**、`target/release/tftpd-r-gui.exe` **8,980,480 B（8.56 MiB）**。最近一次增量 release 构建 59.04 s（0.2.2 版本号变更后的完整重建；此前 GUI 增量 51.43 s）；首次完整 size-profile 构建约 116.4 s。`v0.2.2` 的 MSVC x64 归档为 daemon 1,193,984 B、GUI 8,641,024 B；GNU 与 MSVC 体积不可直接比较。换工具链/平台必须重测，这不是对其他平台的承诺。

其他有用命令：

```powershell
cargo run -p tftpd-r-daemon -- check-config --config config.toml
cargo run -p tftpd-r-daemon -- serve --config config.toml
cargo run -p tftpd-r-gui-slint -- --smoke-test
```

`.bootstrap/e2e_daemon.py`（若存在，git-ignored）用原始 UDP 客户端做 daemon 端到端验证；不是交付物。

### 4.4 提交规则

- **绝不提交计划文档**：它未跟踪且被 `.git/info/exclude` 忽略；不要 `git add -f`，不要复制进 `docs/` 或其他受跟踪文件。
- 不提交 `target/`、`.dsh-toolchain/`、`.bootstrap/`、日志和临时文件。
- 提交信息说明行为变化、验证命令与结果；一个提交聚焦一个主题。
- 历史重写/强推只在用户明确要求时进行；远程为 `origin`（GitHub），但仍应避免无谓改写。

## 5. 测试要求

- **协议 codec**：每个 opcode、边界长度、缺字段、NUL/UTF-8、未知 opcode、option 重复/非法值/大小写。
- **路径沙箱**：`../`、`/../`、双分隔符、盘符、ADS、设备名、符号链接/reparse、大小写/前缀混淆、root 前缀正向与反向、大小限制。
- **session/server**：重传、重复 ACK、错误 TID、重复 RRQ、零长度终止块、块号回绕、并发、取消/停止、超时耗尽、事件。
- **daemon**：CLI 解析、配置错误退出码（0/1/2）、默认配置与示例一致、二进制冒烟。
- **构建辅助**：ICO 解析、`.res` 结构、当前产品图标固定尺寸/字节数；更换 ICO 必须同步断言。
- **GUI**：控制器契约、`recent.rs`、`user_state.rs`（首次启动标记、`LastRoot`、状态目录选择/便携回退）、根目录启动优先级、空根目录拒绝与状态文本、`store_root`、日志格式/筛选/上限/导出、`log_export.rs`、对话框轮询、`netif.rs`、`folder_picker.rs`、地址选择模型、首次启动与自动重启判定。真实键盘/IME、真实网卡枚举、系统目录/保存对话框、布局与字体只能靠目标机/人工验收，需在 `docs/testing.md` 如实标注。
- 时间相关测试目前使用真实计时器（1 s 级别），避免随机 sleep；新增测试尽量确定性。

## 6. 后续路线与优先级

1. **Phase 6 目标机验证/发布收尾**：验证 MSVC Release、Linux 桌面 GUI、arm64 运行、真实/老式 TFTP 客户端（含 `get /file`）、LAN/防火墙/TID 端口、69 端口权限、RDP、GUI 中文/IME/真实鼠标/其他 DPI、关闭窗口资源释放；核验归档与 SHA-256；完成依赖/许可证审计、支持矩阵、打包及升级/回滚说明；按价值补 codec fuzz/property、事件洪泛、可控丢包测试。所有结果写入 `docs/testing.md`，未做项标记未验证。不在本阶段新增 IPC、上传、服务注册、自动提权或公网默认监听。
2. **Phase 7 本机 IPC + GUI daemon 控制**：先写 ADR，明确生命周期、endpoint、平台边界、授权与断线语义；首个切片连接已运行的 daemon，不默认让 GUI 启动/托管 daemon。新建 `tftpd-r-control-protocol`（版本化 DTO、长度前缀 framing、请求 ID、最大消息、错误码）；Windows named pipe/Unix domain socket 必须校验本机 ACL/权限并拒绝远程客户端；初始命令限于查询状态、订阅有界事件、对已加载配置 start/stop；不得经 IPC 修改 `root_dir`、开放上传或提权；限制连接/速率/在途/订阅；覆盖越权、畸形/超长消息、未知版本、断连、重连与 GUI 不阻塞。
3. **Phase 8 可选 WRQ 上传**：默认及当前唯一行为继续只读。只有明确需要时先更新 ADR 0005/0003 与威胁模型，再实现显式 opt-in；默认禁止覆盖；临时文件独占创建与同目录原子提交，处理取消/超时/磁盘满/flush/rename 失败并清理；持续限制大小/时长/并发；重新审查路径竞态；增加 WRQ 状态机、路径安全、失败注入、已有文件保护、互操作测试；通过安全评审后才开放配置和 GUI。
4. **CI/发布现状**：CI 在 Windows x64 + Linux x64 跑 fmt/clippy/test；release workflow 在 `v*` tag 构建并发布 Windows/Linux × x64/arm64。构建通过不等于目标机运行通过。
5. **Windows Service**：独立后续工作，不混入前台 daemon 生命周期；不得自动注册服务或提权。

## 7. 常见错误（禁止）

- 用 `canonicalize + starts_with` 或字符串前缀比较替代组件级沙箱。
- 为兼容老客户端放宽 `..`、盘符/`:`、设备名或 reparse point 检查；根目录前缀只允许“剥离前导分隔符”。
- 静默把 `netascii` 当 `octet`，或对 WRQ 假成功。
- 在客户端 ERROR/日志中输出本地绝对路径、OS 原始错误或内部结构。
- 重复 RRQ 时新建会话；或对错误 TID 无限回 ERROR 5。
- UI 线程同步等待 stop/grace；或让后台任务直接改 Slint 属性。
- 自定义无边框标题栏时让 `WindowMoveArea` 覆盖窗口按钮；最小化/最大化/关闭必须走 `slint::Window` API（`set_minimized`/`set_maximized`/`quit_event_loop`），不要只依赖 Slint 的 `minimized`/`maximized` window property。
- 无上限的事件/日志/会话/请求缓冲。
- 在仍有进行中的传输时自动重启，或在传输结束后排队自动重启；配置变更应保存并提示手动重启（ADR 0009）。
- 把 `Stopping` 因配置变更改回启动，或在 `stop()` 取走 handle 后仍报告陈旧的 `Running`。
- 把 GUI 无配置时的「全部网络」表单默认写进 `ServerConfig::default()` 或 daemon 默认值。
- 在 GUI 未选择根目录时显示/记录 dummy `tftp-root`；`config.toml` 有效时用 `last-root.txt` 覆盖其 `root_dir`；用最近根目录列表代替 `last-root.txt`（「清空」不应忘记上次根目录，取消的浏览也不应被记为已选根目录）。
- 把 `last-root.txt` 写入失败、启动时失效条目或 `config.toml` 覆盖记忆值做成静默；只把「选中目录后还要保存设置」写在日志里而不在设置表单内提示（日志被模态框遮挡）。
- 因 per-user 配置目录不可写就放弃根目录记忆/最近历史/首次启动标记（必须先尝试 exe 旁、再尝试工作目录的便携 `.tftpd-r`）；不得把便携状态写到未经验证的可疑位置或不经日志说明。
- 把日志级别筛选做成删除条目或影响导出；自动写日志文件/轮转；把「打开目录选择器/保存对话框」写入日志；允许带换行的日志消息破坏行高或导出格式；在日志导出中使用未经验证的 helper 输出路径。
- 用 `[Console]::Out` 输出目录路径（OEM 代码页会破坏非 ASCII 名称；必须用 UTF-8 `StreamWriter`）；或忘记在面板退出时 kill 预热的 PowerShell helper（包括仍打开的对话框）。
- 把 `slint_build` 的 `embed_resources` 改成 `EmbedResourcesKind::EmbedForSoftwareRenderer`（该模式会预烘焙字形，使中文文案/运行时日志空白）；窗口图标走 `include_bytes!` + `Window.icon`（见 `src/main.rs` 的 `app_icon()`），不要为此开启 `image-default-formats` 或改用该嵌入模式。
- 未经 ADR 与安全审查新增 `unsafe`、上传、IPC、服务注册或公网默认监听。
- 提交 `.res`/`.o` 等图标生成物、手改 build-support 输出，或在非 Windows 目标强行调用 `windres`；GNU 构建失败时不得静默跳过图标。
- 未重新测量、未更新 ADR 0010 与 `docs/testing.md` 就改 release profile（恢复 unwind、换 `opt-level=3` 等），或为减体积引入 UPX、nightly `build-std`/`panic_immediate_abort`。
- 提交本地计划文档、便携工具链、构建产物；或在 CI/README 中宣称未验证的平台/客户端已通过。

## 8. 交付格式

每个阶段/改动完成后，最终回复至少包含：

1. 变更文件与实现范围；
2. 关键设计选择及与计划的偏差（ADR 链接）；
3. 实际运行的命令与结果（不得虚构）；
4. 已知限制/未覆盖风险；
5. 下一阶段建议。

文档与代码同等重要：行为变化必须反映到 README/security/architecture/ADR/testing；计划变化只更新本地计划文档，且**不提交**。
