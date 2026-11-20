# AGENTS.md — 后续开发指南

> 面向后续 AI 编码代理与维护者。开始任何修改前，先阅读本文件、[README.md](README.md)、[docs/architecture.md](docs/architecture.md)、[docs/security.md](docs/security.md)、[docs/testing.md](docs/testing.md) 和 [docs/adr/](docs/adr/)。
>
> **重要：** 仓库根目录的《Rust + Slint 无 WebView 依赖解耦式 TFTP 服务端架构设计方案.md》是**本地规划资料**，不是版本库内容。它被 `.git/info/exclude` 忽略，**禁止以任何方式提交进 git**（包括 `git add -f`、复制到 docs、或把其内容粘贴进其他受跟踪文件）。需要更新计划时，直接编辑该本地文件，不要提交。

## 1. 项目定位与当前状态

纯 Rust 的**只读** TFTP（RFC 1350）服务端，Cargo Workspace 分为协议核心、无头 daemon 和 Slint GUI。GUI 采用 winit + 软件渲染器；协议核心不依赖 GUI、daemon 或 IPC。

当前状态（2026-11-20，已验证）：

| 阶段 | 状态 | 说明 |
| --- | --- | --- |
| Phase 0–5 | ✅ 已完成 | workspace、codec/沙箱、RRQ 引擎、daemon、GUI 面板重设计（无边框自定义标题栏与窗口控件、工具栏状态徽标、状态栏、日志/传输列表、设置/关于、最近根目录）、文档/ADR/CI 文件 |
| Phase 6 IPC | ⏳ 未开始 | GUI 仅 Embedded 模式；无 named pipe / daemon 控制 |
| Phase 7 WRQ 上传 | ⏳ 未开始 | 默认且唯一行为是只读；`allow_upload=true` 被配置校验拒绝 |
| Phase 8 发布/目标机 | 🟡 部分完成 | [0.1.0](https://github.com/greyskysoul/tftpd-r/releases/tag/v0.1.0) 与 [0.2.0](https://github.com/greyskysoul/tftpd-r/releases/tag/v0.2.0) 已发布（Windows/Linux × x64/arm64 四目标原生构建）；CI 在 windows-latest（MSVC x64）与 ubuntu-latest（GNU x64）上 fmt/clippy/test 全绿；目标机人工验收（真实/老式客户端、LAN/防火墙、RDP、IME、其他 DPI）未做 |
| 0.2.0 R1/R2 | ✅ 已发布（2026-11-20） | 默认端口 69 + 非强制提示（ADR 0007）；`port` + `bind_addresses` 多监听、GUI 多选地址选择器与目录选择器（ADR 0008）；首次启动自动打开设置（每用户一次）、选择器「全部网络」独占且不可清空、「全部网络」为首次启动默认、最近根目录可清空、空闲时配置/根目录变更自动重启（ADR 0009）；配置格式破坏性变更，`bind_address` 已删除 |

已验证环境：Windows 11（10.0.26300）+ Rust 1.100.0 `x86_64-pc-windows-gnu` + MinGW-w64 GCC 15.3 + Slint 1.19.0；CI 另在 `windows-latest`（MSVC x64）与 `ubuntu-latest`（GNU x64）跑通 fmt/clippy/test，并在原生 arm64 runner 上构建 arm64 产物。**未验证**：MSVC、Linux 与 arm64 产物的**运行**（CI 只对 x64 跑测试，arm64 仅编译）、GUI 在 Linux 桌面上的运行、真实/外部 TFTP 客户端互操作、Windows Service、RDP、69 端口权限、真实键盘/IME 输入、其他 DPI/语言下的字体渲染与无障碍。不要在文档或回复中把这些写成已通过。

最近一次完整验证结果（2026-11-20 第四轮，GUI 首次启动/地址选择/历史清空/空闲自动重启）：`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（**147 passed**：core 单元 63、RRQ UDP 集成 29、daemon 单元 10、daemon CLI 集成 5、GUI 单元 39、doctest 1）、`cargo build --workspace --release` 全部通过；隔离 `APPDATA` 启动 GUI 确认无 `config.toml` 时自动打开设置且默认选中「全部网络 (0.0.0.0)」，同一 profile 第二次启动不再弹窗；窗口消息点击验证选择器五种切换状态（选具体地址清空「全部网络」、可叠加多个具体地址、选「全部网络」清空其他、再次点击「全部网络」仍保持选中、取消最后一个具体地址回退到「全部网络」）；最近目录「清空」删除 `recent-dirs.txt` 并显示空状态；运行中切换根目录且无传输时自动重启，新根目录可下载 `two.txt`，旧根目录文件返回 file not found；有活跃传输时只记录 WARN 并保持旧根目录/传输不中断。[0.2.0 发布工作流](https://github.com/greyskysoul/tftpd-r/actions/runs/44719989555) 全绿（Windows/Linux 测试门 + 四目标原生构建 + GitHub Release 发布），四个归档的 GitHub SHA-256 digest 与 `SHA256SUMS` 一致；**未验证**：真实鼠标硬件（点击为窗口消息）、其他 DPI、真实/外部 TFTP 客户端、Linux GUI 运行、下载后重新执行 MSVC/arm64 归档。首次 `v0.2.0` tag 运行因 Linux clippy 把 `netif.rs` 的 Windows-only 辅助函数判为 dead code 而失败，修复后移动了 tag（失败运行未发布任何资产）。此前的 0.1.0 人工验收与图标/配色结果见 `docs/testing.md`。执行新改动后必须重新运行并更新真实数字。

此前的完整验证结果（2026-11-20 第二轮，0.2.0 改动）：`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（**131 passed**：core 单元 63、RRQ UDP 集成 29、daemon 单元 10、daemon CLI 集成 5、GUI 单元 23、doctest 1）、`cargo build --workspace --release` 全部通过；多监听 daemon 以 `127.0.0.1` 与 `192.168.3.17` 的**同一临时端口**各下载 5000 字节（`/image.bin`）并 Ctrl-Break 优雅退出；GUI debug/release `--smoke-test` 退出码 0、stderr 为空，PE 子系统为 Windows GUI（daemon 仍为 console）；`PrintWindow` 截图确认状态栏根目录仅显示末级目录名、状态栏横线修复、设置对话框的新端口/多选地址/浏览按钮，以及地址选择器列出 `192.168.3.17` 与两个全局 IPv6 地址。进度条方向用合成传输行（`TFTPD_R_GUI_PREVIEW=transfer`）截图确认左侧起算；**未验证**：真实鼠标点击、原生目录对话框、GUI 中的绑定失败提示、真实多网卡切换、真实 GUI 传输在修复后的渲染、Linux 上的低端口权限路径。此前的 0.1.0 人工验收记录（真实 2 MiB 传输、窗口控件、最近根目录等）见 `docs/testing.md`。执行新改动后必须重新运行并更新真实数字。

## 2. 仓库地图

```text
.
├── Cargo.toml                     # workspace、共享依赖版本、lint 配置
├── Cargo.lock                     # 应用 workspace，必须提交
├── config.example.toml            # 唯一安全默认配置示例（daemon 内嵌同一份）
├── README.md
├── AGENTS.md                      # 本文件
├── docs/
│   ├── architecture.md            # crate 边界、生命周期、数据流、线程模型
│   ├── security.md                # 威胁模型、沙箱、残余风险
│   ├── testing.md                 # 实际执行记录与未验证项
│   ├── roadmap.md                 # 0.2.0 已发布项与后续未实现项
│   └── adr/0001–0009              # 已接受的决策（IPC 延后、上传关闭、octet only、端口 69、多地址绑定、GUI 首次启动/自动重启等）
├── crates/
│   ├── tftpd-r-core/             # 无 GUI/daemon/IPC 依赖
│   │   ├── src/{lib,config,error,events,filesystem,server,session}.rs
│   │   ├── src/protocol/{mod,packet,codec,options}.rs
│   │   └── tests/rrq.rs           # 真实 loopback UDP 集成测试
│   ├── tftpd-r-daemon/           # CLI + TOML + tracing + 信号处理
│   │   ├── src/{main,cli,config_file,logging,shutdown}.rs
│   │   └── tests/cli.rs           # 运行编译后的二进制
│   └── tftpd-r-gui-slint/        # Slint 面板 + EmbeddedController
│       ├── src/{main,controller,recent,netif,folder_picker,user_state}.rs
│       ├── ui/{app,titlebar,toolbar,statusbar,logs,transfers,settings,about,
│       │       root-menu,widgets,theme,icons,models}.slint
│       ├── ui/icons/                           # Material 图标来源、产品图标（SVG/PNG/ICO/RGBA）
│       └── build.rs
├── .github/workflows/ci.yml       # push/PR：fmt、clippy、test、release 构建、GUI 编译
├── .github/workflows/release.yml  # v* tag：四目标原生构建并发布 GitHub Release
├── .github/release-notes/         # 每个 tag 一个发布说明文件（<version>.md）
└── （未跟踪）计划文档 .md          # 只用本地编辑，禁止提交
```

依赖方向必须保持：`tftpd-r-core` ← `tftpd-r-daemon`；`tftpd-r-core` ← `tftpd-r-gui-slint`。GUI 目前不依赖 daemon；若新增 IPC，DTO/客户端协议必须放在独立 crate（例如 `tftpd-r-control-protocol`），不要污染 `tftpd-r-core`。

## 3. 不可破坏的契约

### 3.1 协议行为

- 只服务 RFC 1350 **RRQ**；WRQ 返回 ERROR 2（Access violation，消息 "Uploads are disabled"），不得悄悄实现写入。
- 仅 `octet`。`netascii`、`mail` 显式拒绝（ERROR 0 + 明确消息）；未知 mode 在 codec 层返回 `UnsupportedMode`，主监听端口回 ERROR 4。**绝不能把 netascii 静默当 octet。**
- 每个会话使用独立 UDP socket/TID；只接受来自预期 peer（IP+端口）的包；其他来源回 ERROR 5，且每个 block 交换最多 8 次。
- 初始请求在监听端口处理；来自同一 peer 的重复 RRQ 必须转发 `SessionSignal::Resend` 给已有会话，**不能新建第二个会话/TID**。
- DATA/ACK 状态机：等待匹配 ACK；重复/过期 ACK 忽略；重传必须发送与上次完全相同的已编码包；重传次数受 `max_retries` 限制；总时长受 `max_transfer_duration_secs` 限制。
- 最终块规则：payload < block size 即结束；若文件长度是块大小整数倍，必须发送零长度 DATA 终止块。块号 16 位回绕（65535 → 0）必须正确。
- 选项：`blksize` 8..=65464 且不超过配置 cap；`timeout` 1..=255；`tsize` 返回文件大小；未知选项忽略；重复或非法已知选项回 ERROR 8。请求 datagram 上限 1024 字节，超长/畸形回 ERROR 4。
- 客户端可见 ERROR 文本必须是固定、脱敏的常量；**不得包含本地路径、OS 原始错误或内部结构**。文件名进入事件/日志前必须 `sanitize_display_name`。
- 服务端默认只读、默认绑定回环、默认不开放上传。任何默认值改动都需要 ADR 与安全评审。

### 3.2 路径与文件系统沙箱

实现核心在 [`crates/tftpd-r-core/src/filesystem.rs`](crates/tftpd-r-core/src/filesystem.rs)。必须保持以下不变量：

1. **老客户端根目录前缀兼容**：文件名前导的任意 `/` 或 `\` 必须被剥离，并相对配置根目录解析（`get /firmware.bin` → `<root>/firmware.bin`）。只剥离**前导**分隔符；`a//b`、尾随分隔符仍拒绝；仅由分隔符组成的 `/` 表示根目录本身，作为无效文件名拒绝（TFTP 不传目录）。
2. 剥离后仍拒绝：`..`、`.`、空组件、盘符/`:`（同时阻断 NTFS ADS）、Windows 保留设备名（`CON`/`NUL`/`COM1`…）、尾随点/空格、`< > " | ? *`、控制字符和 NUL。
3. 根目录在 `Sandbox::new` 时 canonicalize；逐组件 `symlink_metadata` 检查，拒绝任何 reparse point / symlink；只允许普通文件；大小上限在 open 前后都检查。
4. open 之后再次 canonicalize，并按**路径组件**（不是字符串前缀）与 canonical root 比较；Windows 大小写不敏感。`starts_with("C:\\root")` 这类写法永远不算安全。
5. 残余风险（TOCTOU、根目录内硬链接）已在 `docs/security.md` 记录；**不要声称沙箱绝对安全**。任何上传/写入路径都必须重新做设计、ADR、原子临时文件与安全测试。
6. 相关测试：`root_prefixed_names_map_to_the_root`、`opens_root_prefixed_file_without_leading_slash_in_display_name`、`rejects_traversal_and_drive_paths`、`root_prefixed_filename_is_served`、`root_prefix_does_not_escape_the_sandbox`、symlink/reparse 测试。修改命名逻辑必须同时保留正反向用例。

### 3.3 配置

- `ServerConfig` 使用 serde `deny_unknown_fields` + 安全默认值；`validate()` 是 daemon、GUI 和测试共用的唯一规则来源。
- `allow_upload=true`、`overwrite=true` 当前必须是**配置错误**（上传尚未实现，不能给运维错误暗示）。
- 监听配置是 `port` + `bind_addresses`（`Vec<IpAddr>`，1..=16 个）。默认 `127.0.0.1:69`（Linux 需 root/CAP_NET_BIND_SERVICE，Windows 通常允许普通用户但端口可能被系统保留）；端口 0 仅用于测试/临时实例，首个成功监听的端口会被其余地址复用。重复地址去重；同族通配 `0.0.0.0`/`::` 折叠该族其他地址；多播/广播拒绝（ADR 0008）。绑定**全有或全无**：任一地址失败即放弃全部已绑定 socket 并返回分类错误，不得静默只服务部分地址。GUI 在没有有效 `config.toml` 时的表单默认选中 `0.0.0.0`（ADR 0009），但核心/daemon 的默认值仍是回环，且服务只有用户点击「启动」后才启动；不要把 GUI 表单默认写进 `ServerConfig::default()`。
- 绑定特权端口失败必须给出可操作提示，**不得**静默回退、自动提权或改写配置（ADR 0007）。
- GUI 的网卡枚举与目录选择通过固定绝对路径的 PowerShell/.NET（Unix 为 `hostname`/`zenity`）辅助进程完成，后台线程执行；不得引入 shell、用户可控参数、提权或手写 unsafe。辅助进程失败时必须降级为两个特殊地址/手动输入，而不是崩溃。
- `root_dir` 必须存在且为目录；低端口权限按平台处理（Linux 需 root/CAP_NET_BIND_SERVICE；Windows 通常允许普通用户，但端口可能被保留），README/security 与 ADR 0007 的表述必须与此一致，不要笼统写成“Windows 需要管理员”。
- `config.example.toml` 由 daemon `include_str!` 内嵌，`print-default-config` 必须与文件完全一致；修改示例时同步测试。

### 3.4 资源、事件与生命周期

- 全局 `max_sessions` 信号量 + 每 IP 最多 8 个并发会话；请求大小、文件大小、重传次数、总时长、事件环、GUI 日志行数都有上限。新增资源必须有明确上限。
- `EventSink::emit` 必须短小、非阻塞、不得 panic；GUI 通过 `EventBus::drain_since(seq)` 轮询，不直接由 Tokio task 改 UI。
- 生命周期：`Starting → Running { local_addrs } → Stopping → Stopped`（或 `Failed`）；每个监听地址一个 accept loop，peer 表/会话上限/会话 ID 全局共享，会话 socket 绑定该监听地址（非通配时）；`ServerHandle::stop()` 幂等，先取消全部 accept loop，再在 `shutdown_grace_secs` 内并行 drain，最后 abort 剩余任务并释放 socket/permit/文件。
- GUI 的 `stop()` 必须异步/轮询，不能阻塞 UI 线程等待宽限期；窗口关闭后 `EmbeddedController::shutdown()` 负责最终清理。
- GUI 的配置/根目录变更自动重启只在 `Starting`/`Running`、配置确实变化且 `state.transfers` 为空时发生；有传输时只保存并 WARN，不得中断传输或排队在传输结束后自动重启。`Stopping` 绝不能被变更改回启动。`poll_status` 在 handle 已被 `stop()` 取走后不得继续报告陈旧的 `Running`。

### 3.5 代码与依赖

- 全 workspace `unsafe_code = "deny"`；手写代码禁止新增 `unsafe`。Slint 生成的代码带有 `#[allow(unsafe_code)]`，这是生成器的例外，不要借此在手写代码中放宽。
- 不新增无必要依赖；新增依赖需核对许可证、维护状态、MSRV、Windows 支持、features 与传递依赖，并提交更新后的 `Cargo.lock`。
- 不引入 WebView；GUI 保持 Slint + software renderer 组合（除非有 ADR 和实测依据变更）。
- `Cargo.lock` 属于应用 workspace，必须提交。不要提交 `target/`、`.dsh-toolchain/`、`.bootstrap/`、本地计划文档。

## 4. 开发工作流

### 4.1 开始前

1. `git status`，确认没有未提交/未检查的改动；不要覆盖用户工作。
2. 阅读本文件、README、相关 ADR、`docs/security.md` 与代码；先画模块/调用链，再动手。
3. 确认当前平台、Rust 版本、Slint/Tokio 版本；本仓库验证基线是 Rust 1.100.0（GNU）。MSVC/Linux 未验证，不要假设。
4. 若 `.bootstrap/env.ps1` 存在（本会话的便携工具链），先 `Set-ExecutionPolicy -Scope Process Bypass -Force; . .\.bootstrap\env.ps1`，否则使用系统 Rust。Cargo 依赖在受限沙箱内可能因 Schannel 报 `SEC_E_NO_CREDENTIALS`；这属于环境限制，依赖已缓存时用 `--offline`，不要为它改项目代码。

### 4.2 变更流程

1. **先复现/先写测试**：bug 或行为变更应先有失败测试（单元或 loopback UDP 集成测试），再进行实现。
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

当前基线是 **147 tests passed**（core 单元 63、RRQ UDP 集成 29、daemon 单元 10、daemon CLI 集成 5、GUI 单元 39（控制器 5 + 最近目录 7 + 网卡枚举 4 + 目录选择 2 + 用户状态 4 + 地址模型/首次启动/自动重启/产品图标 17）、doctest 1）。新增测试后更新 `docs/testing.md` 和本文件的数字。

其他有用命令：

```powershell
cargo run -p tftpd-r-daemon -- check-config --config config.toml
cargo run -p tftpd-r-daemon -- serve --config config.toml
cargo run -p tftpd-r-gui-slint -- --smoke-test
```

`.bootstrap/e2e_daemon.py`（若存在，git-ignored）会用原始 UDP 客户端以 `/image.bin` 形式做 daemon 端到端验证；不是交付物。

### 4.4 提交规则

- **绝不提交计划文档**：它未跟踪且被 `.git/info/exclude` 忽略；不要 `git add -f`，不要把它复制进 `docs/` 或其他跟踪文件。
- 不提交 `target/`、`.dsh-toolchain/`、`.bootstrap/`、日志和临时文件。
- 提交信息说明行为变化、验证命令与结果；一个提交聚焦一个主题。
- 历史重写/强推只在用户明确要求时进行；远程为 `origin`（GitHub），但仍应避免无谓改写。

## 5. 测试要求

- **协议 codec**：每个 opcode、边界长度、缺字段、NUL/UTF-8、未知 opcode、option 重复/非法值/大小写。
- **路径沙箱**：`../`、`/../`、双分隔符、盘符、ADS、设备名、符号链接/reparse、大小写/前缀混淆、root 前缀正向与反向、大小限制。
- **session/server**：重传、重复 ACK、错误 TID、重复 RRQ、零长度终止块、块号回绕、并发、取消/停止、超时耗尽、事件。
- **daemon**：CLI 解析、配置错误退出码（0/1/2）、默认配置与示例一致、二进制冒烟。
- **GUI**：控制器契约（start/stop/重复 start/无效配置/异步 restart/restart 失败上报/`stop()` 后不再报告 `Running`）、`recent.rs`（顺序、去重、上限、持久化、canonicalize、失效目录清理、清空后删除状态文件）、`user_state.rs`（首次启动标记的持久化与只读 profile 降级）、`netif.rs`（IP 解析/过滤/排序）、`folder_picker.rs`（空选择与非目录拒绝）、地址选择模型（两个特殊项恒定存在、选中但未发现的地址保留、摘要截断、`0.0.0.0` 独占/不可取消/最后一个地址回退）以及首次启动默认与自动重启判定（运行中且配置变化且无传输才重启；有传输只记录、不排队；`Stopping` 不复活）；事件洪泛、真实键盘/IME、真实网卡枚举、系统目录对话框、布局与字体只能靠目标机/人工验收，需在 `docs/testing.md` 如实标注。
- 时间相关测试目前使用真实计时器（1 s 级别），避免引入随机 sleep；新增测试尽量确定性。

## 6. 后续路线与优先级

1. **0.2.0 需求（R1/R2 及 GUI 交互补充已在 0.2.0 发布；详见 [docs/roadmap.md](docs/roadmap.md)）**
   - **R1 默认端口 69 ✅**：默认端口 69；权限不足时提示可改用非特权端口（如 6969），不强制、不回退、不提权、不改写配置（ADR 0007）。
   - **R2 多选绑定地址 ✅**：`port` + `bind_addresses` 多监听；GUI 多选选择器自动导入网卡 IP，并始终提供「全部网络 (0.0.0.0)」与「本地回环 (127.0.0.1)」（ADR 0008）。绑定全有或全无，不做部分失败服务。
   - **GUI 交互补充 ✅**：无有效配置的首次启动自动打开设置（每 profile 一次）并预选「全部网络」，「全部网络」在 picker 中独占且不可清空，最近根目录可清空；保存设置或切换根目录时，服务运行中且无传输则自动重启，否则只保存并提示手动重启（ADR 0009）。核心/daemon 的回环默认不变。
   - **R3 已取消**：INI 配置与固定位置 `~/.tftpdr.ini` 的需求经确认撤销，配置格式保持 TOML；GUI 继续读取工作目录的 `config.toml`。不要再实现或重新引入 INI 支持。
2. **Phase 6：本机 IPC + GUI daemon 控制**
   - 新建 `tftpd-r-control-protocol`（版本化 DTO、长度前缀 framing、请求 ID、最大消息、错误码）。
   - Windows named pipe / Unix domain socket；本机 ACL/权限校验，拒绝远程；命令分权（只读状态 vs start/stop vs 配置变更）。
   - 连接数/速率/订阅上限；断线重连；契约与未授权客户端测试；GUI IPC controller。
   - 不要把“本地”当可信；默认不允许 IPC 改 root/上传权限。
3. **Phase 7：WRQ 上传**
   - 必须显式配置开启；默认禁止覆盖；临时文件 + 原子替换；取消/磁盘满清理；上传大小/空间/时长限制；路径检查与 TOCTOU 复核。
   - 补齐 RFC 互操作与安全测试，更新 ADR 0005 后才可默认以外的行为。
4. **Phase 8：目标机验证**
   - MSVC Release、真实/老式 TFTP 客户端（含 `get /file`）、LAN/防火墙/TID 端口、69 端口权限、RDP、GUI 中文与交互、关闭窗口资源释放。
   - 依赖/许可证审计、支持矩阵、打包与升级/回滚说明；如实记录产物大小与依赖，不作未验证承诺。
5. **CI**：`.github/workflows/ci.yml` 已在公开仓库运行（Windows x64 + Linux x64）；发布由 `.github/workflows/release.yml` 在 `v*` tag 上构建并发布 Windows/Linux × x64/arm64 四个原生目标的归档。
6. **Windows Service**：独立后续工作，不要混入前台 daemon 生命周期；不得自动注册服务或提权。

## 7. 常见错误（禁止）

- 用 `canonicalize + starts_with` 或字符串前缀比较替代组件级沙箱。
- 为兼容老客户端而放宽 `..`、盘符/`:`、设备名或 reparse point 检查；根目录前缀只允许“剥离前导分隔符”。
- 静默把 `netascii` 当 `octet`，或对 WRQ 假成功。
- 在客户端 ERROR/日志中输出本地绝对路径或 OS 原始错误。
- 重复 RRQ 时新建会话；或对错误 TID 无限回 ERROR 5。
- UI 线程同步等待 stop/grace；或让后台任务直接改 Slint 属性。
- 自定义无边框标题栏时让 `WindowMoveArea` 覆盖窗口按钮（Windows 上按下按钮会同时开始拖动并把最大化窗口还原）；最小化/最大化/关闭必须走 `slint::Window` API（`set_minimized`/`set_maximized`/`quit_event_loop`），不要只依赖 Slint 的 `minimized`/`maximized` window property。
- 无上限的事件/日志/会话/请求缓冲。
- 在仍有进行中的传输时自动重启服务，或在传输结束后排队自动重启；配置变更应保存并提示手动重启（ADR 0009）。
- 把 `Stopping`（用户明确停止）因配置变更改回启动，或在 `stop()` 取走 handle 后仍从 watch 通道报告陈旧的 `Running`。
- 把 GUI 无配置时的「全部网络」表单默认写进 `ServerConfig::default()` 或 daemon 默认值，破坏核心/daemon 的回环默认。
- 把 `slint_build` 的 `embed_resources` 改成 `EmbedResourcesKind::EmbedForSoftwareRenderer`：该模式会在构建期预烘焙字形，使全部中文文案与运行时日志文本渲染成空白（2026-11-20 实际踩到）。窗口图标走 `include_bytes!` + `Window.icon`（见 `src/main.rs` 的 `app_icon()`），不要为此开启 `image-default-formats` 或改用该嵌入模式。
- 未经 ADR 与安全审查新增 `unsafe`、上传、IPC、服务注册或公网默认监听。
- 提交本地计划文档、便携工具链、构建产物；或在 CI/README 中宣称未验证的平台/客户端已通过。

## 8. 交付格式

每个阶段/改动完成后，最终回复至少包含：

1. 变更文件与实现范围；
2. 关键设计选择及与计划的偏差（ADR 链接）；
3. 实际运行的命令与结果（不得虚构）；
4. 已知限制/未覆盖风险；
5. 下一阶段建议。

文档与代码同等重要：行为变化必须反映到 README/security/architecture/ADR/testing；计划变化只更新本地计划文档，且**不提交**。
