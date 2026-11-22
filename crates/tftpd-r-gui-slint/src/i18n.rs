//! Startup language selection and the panel's operator-facing text.
//!
//! The panel supports English and Simplified Chinese. The language is decided
//! **once at startup** from the system locale: Simplified Chinese locales show
//! Chinese, everything else (including every unsupported language and a
//! detection failure) shows English. There is no language picker, so a text is
//! always rendered in the language the process started with; this also means
//! the in-memory activity log does not have to store message IDs.
//!
//! Static labels live in the `.slint` files behind `@tr(...)` with English
//! source strings; [`Text`] covers the strings the Rust side builds at runtime
//! (log lines, validation errors, native dialog titles) and must stay in the
//! same language as the Slint translation chosen for the window
//! ([`Language::slint_locale`]).

use sys_locale::get_locale;

/// Languages the panel ships translations for.
///
/// The enum is deliberately small: adding a language means adding a variant, a
/// locale mapping in [`Language::from_locale`], a `.po` catalogue under
/// `translations/`, and the matching strings in [`Text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    /// English, also the fallback for every unsupported locale.
    #[default]
    En,
    /// Simplified Chinese.
    ZhCn,
}

impl Language {
    /// Decide the panel language from the current system locale.
    ///
    /// A locale that cannot be read counts as unsupported and yields English.
    #[must_use]
    pub fn detect() -> Self {
        Self::from_locale(get_locale().as_deref())
    }

    /// Map a BCP 47 style locale (`zh-CN`, `zh_CN.UTF-8`, `en-GB`, …) to a
    /// supported language.
    ///
    /// Only simplified Chinese is supported: Traditional Chinese locales
    /// (`zh-TW`, `zh-HK`, `zh-MO`, `zh-Hant*`) and every other language fall
    /// back to English instead of showing Simplified Chinese text.
    #[must_use]
    pub fn from_locale(locale: Option<&str>) -> Self {
        let Some(locale) = locale else {
            return Self::En;
        };
        // POSIX locales may carry an encoding or a modifier: `zh_CN.UTF-8@euro`.
        let locale = locale.trim();
        let locale = locale.split(['.', '@']).next().unwrap_or(locale);
        let mut parts = locale.split(['-', '_']);
        let Some(language) = parts.next() else {
            return Self::En;
        };
        if !language.eq_ignore_ascii_case("zh") {
            return Self::En;
        }
        let script_or_region = parts.next().unwrap_or_default().to_ascii_lowercase();
        match script_or_region.as_str() {
            // `zh-TW`, `zh-HK`, `zh-MO`, `zh-Hant`, `zh-Hant-TW`.
            "tw" | "hk" | "mo" | "hant" => Self::En,
            _ => Self::ZhCn,
        }
    }

    /// Locale name Slint uses to select the bundled translation.
    ///
    /// `"en"` selects the default language, i.e. the English source strings.
    #[must_use]
    pub fn slint_locale(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCn => "zh_CN",
        }
    }

    /// Human-readable name, shown in the About dialog.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::ZhCn => "简体中文",
        }
    }
}

/// Operator-facing strings for the language chosen at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text {
    language: Language,
}

/// Pick the English or the Chinese variant of a message.
macro_rules! text {
    ($self:ident, $en:expr, $zh:expr) => {
        match $self.language {
            Language::En => $en,
            Language::ZhCn => $zh,
        }
    };
}

impl Text {
    /// Texts for `language`.
    #[must_use]
    pub fn new(language: Language) -> Self {
        Self { language }
    }

    /// Separator between list items (`、` reads wrong in English).
    #[must_use]
    pub fn list_separator(self) -> &'static str {
        text!(self, ", ", "、")
    }

    /// Placeholder for an unknown value.
    #[must_use]
    pub fn dash(self) -> &'static str {
        "—"
    }

    // --- Service status -----------------------------------------------------

    /// Server is starting.
    #[must_use]
    pub fn status_starting(self) -> &'static str {
        text!(self, "Starting", "启动中")
    }

    /// Server is running.
    #[must_use]
    pub fn status_running(self) -> &'static str {
        text!(self, "Running", "运行中")
    }

    /// Server is stopping.
    #[must_use]
    pub fn status_stopping(self) -> &'static str {
        text!(self, "Stopping", "停止中")
    }

    /// The last start attempt failed.
    #[must_use]
    pub fn status_start_failed(self) -> &'static str {
        text!(self, "Start failed", "启动失败")
    }

    /// The server is stopped.
    #[must_use]
    pub fn status_stopped(self) -> &'static str {
        text!(self, "Stopped", "已停止")
    }

    /// The server task failed while running.
    #[must_use]
    pub fn status_error(self) -> &'static str {
        text!(self, "Server error", "服务异常")
    }

    /// No root directory has been selected.
    #[must_use]
    pub fn root_unset(self) -> &'static str {
        text!(self, "No root directory", "未指定根目录")
    }

    // --- Bind addresses -----------------------------------------------------

    /// Label of the IPv4 wildcard entry.
    #[must_use]
    pub fn all_networks(self) -> &'static str {
        text!(self, "All networks (0.0.0.0)", "全部网络 (0.0.0.0)")
    }

    /// Label of the IPv4 loopback entry.
    #[must_use]
    pub fn loopback(self) -> &'static str {
        text!(self, "Loopback (127.0.0.1)", "本地回环 (127.0.0.1)")
    }

    /// Label of the IPv6 wildcard entry.
    #[must_use]
    pub fn all_networks_ipv6(self) -> &'static str {
        text!(self, "All IPv6 networks (::)", "全部 IPv6 网络 (::)")
    }

    /// Label of the IPv6 loopback entry.
    #[must_use]
    pub fn loopback_ipv6(self) -> &'static str {
        text!(self, "Loopback IPv6 (::1)", "本地回环 IPv6 (::1)")
    }

    /// No bind address is selected.
    #[must_use]
    pub fn no_address_selected(self) -> &'static str {
        text!(self, "No address selected", "未选择地址")
    }

    /// `first、second 等 3 个地址` / `first, second and 3 addresses`.
    #[must_use]
    pub fn address_count_summary(self, first: &str, second: &str, count: usize) -> String {
        text!(
            self,
            format!("{first}, {second} and {count} addresses"),
            format!("{first}、{second} 等 {count} 个地址")
        )
    }

    /// Listener summary when more than three addresses are bound.
    #[must_use]
    pub fn listener_count_summary(self, first: &str, count: usize) -> String {
        text!(
            self,
            format!("{first} and {count} addresses"),
            format!("{first} 等 {count} 个地址")
        )
    }

    // --- Startup log --------------------------------------------------------

    /// First line of the activity log.
    #[must_use]
    pub fn panel_started(self, version: &str) -> String {
        text!(
            self,
            format!("TftpdR {version} panel started: read-only RRQ, uploads disabled."),
            format!("TftpdR {version} 面板已启动：只读 RRQ，上传已禁用。")
        )
    }

    /// The per-user state directory was unusable.
    #[must_use]
    pub fn portable_state_dir(self, dir: &str) -> String {
        text!(
            self,
            format!(
                "The user configuration directory is not writable; state files use the portable directory: {dir}."
            ),
            format!("用户配置目录不可写；状态文件改用便携目录：{dir}。")
        )
    }

    /// The root directory of the previous run was restored.
    #[must_use]
    pub fn root_restored(self, root: &str) -> String {
        text!(
            self,
            format!("Restored the root directory of the last run: {root}."),
            format!("已恢复上次使用的根目录：{root}。")
        )
    }

    /// The remembered root directory is gone.
    #[must_use]
    pub fn root_stale(self, path: &str) -> String {
        text!(
            self,
            format!(
                "The root directory of the last run is no longer usable and was ignored: {path}."
            ),
            format!("上次使用的根目录已不可用，已忽略：{path}。")
        )
    }

    /// `config.toml` was loaded.
    #[must_use]
    pub fn config_loaded(self) -> &'static str {
        text!(self, "Loaded config.toml.", "已加载 config.toml。")
    }

    /// `config.toml` supplied a root, so the remembered one is unused.
    #[must_use]
    pub fn config_overrides_memory(self) -> &'static str {
        text!(
            self,
            "config.toml provides the root directory, so the last-run record was not used.",
            "config.toml 已提供根目录，因此未使用上次使用记录。"
        )
    }

    /// `config.toml` exists but is unusable.
    #[must_use]
    pub fn config_unusable(self, error: &str) -> String {
        text!(
            self,
            format!("config.toml is unusable: {error}; the panel defaults are used."),
            format!("config.toml 不可用：{error}；已使用面板默认值。")
        )
    }

    /// First run without a configuration.
    #[must_use]
    pub fn first_start(self) -> &'static str {
        text!(
            self,
            "First start: choose a root directory and bind addresses, then save the settings.",
            "首次启动：请选择根目录和绑定地址，然后保存设置。"
        )
    }

    // --- Root directory -----------------------------------------------------

    /// The root directory was written to `last-root.txt`.
    #[must_use]
    pub fn root_remembered(self, path: &str) -> String {
        text!(
            self,
            format!("Remembered the root directory: {path}; it is restored on the next start."),
            format!("已记住根目录：{path}，下次启动将自动恢复。")
        )
    }

    /// `last-root.txt` could not be written.
    #[must_use]
    pub fn root_not_remembered(self, error: &str) -> String {
        text!(
            self,
            format!(
                "Cannot remember the root directory: {error}; the next start will not restore it."
            ),
            format!("无法记住根目录：{error}；下次启动不会自动恢复。")
        )
    }

    /// A directory was picked in the settings form (not applied yet).
    #[must_use]
    pub fn root_selected_in_form(self, path: &str) -> String {
        text!(
            self,
            format!("Selected root directory: {path}. Click “Save settings” to apply it."),
            format!("已选择根目录：{path}。点击“保存设置”后生效。")
        )
    }

    /// Inline reminder in the settings form after picking a directory.
    #[must_use]
    pub fn root_selected_not_saved(self) -> &'static str {
        text!(
            self,
            "Directory selected. Click “Save settings” to apply it.",
            "已选择目录，点击“保存设置”后生效。"
        )
    }

    /// The picked path is not a directory.
    #[must_use]
    pub fn path_is_not_a_directory(self, path: &str) -> String {
        text!(
            self,
            format!("The selected path is not a directory: {path}."),
            format!("所选路径不是目录：{path}。")
        )
    }

    /// A recent root cannot be applied because it disappeared.
    #[must_use]
    pub fn root_unavailable(self, path: &str) -> String {
        text!(
            self,
            format!(
                "Cannot switch the root directory: {path} does not exist or is not a directory."
            ),
            format!("无法切换根目录：{path} 不存在或不是目录。")
        )
    }

    /// Feedback while the native folder dialog is starting.
    #[must_use]
    pub fn opening_folder_picker(self) -> &'static str {
        text!(self, "Opening the folder picker…", "正在打开文件夹选择器…")
    }

    /// Root switched; the idle server restarts.
    #[must_use]
    pub fn root_switched_restarting(self, root: &str) -> String {
        text!(
            self,
            format!(
                "Root directory switched to {root}; no transfer is active, restarting the server."
            ),
            format!("根目录已切换为 {root}；当前无进行中的传输，正在自动重启服务。")
        )
    }

    /// Root switched but transfers are active.
    #[must_use]
    pub fn root_switched_deferred(self, root: &str, transfers: usize) -> String {
        text!(
            self,
            format!(
                "Root directory switched to {root}; {transfers} {} in progress, no automatic restart — restart manually later.",
                if transfers == 1 { "transfer is" } else { "transfers are" }
            ),
            format!("根目录已切换为 {root}；有 {transfers} 个进行中的传输，未自动重启，稍后请手动重启。")
        )
    }

    /// Root switched, automatic restart failed.
    #[must_use]
    pub fn root_switched_restart_failed(self, root: &str) -> String {
        text!(
            self,
            format!("Root directory switched to {root}; the automatic restart failed, restart manually."),
            format!("根目录已切换为 {root}；自动重启失败，请手动重启。")
        )
    }

    /// The requested root is already active.
    #[must_use]
    pub fn root_unchanged(self, root: &str) -> String {
        text!(
            self,
            format!("The root directory already is {root}; no restart needed."),
            format!("根目录已经是 {root}，无需重启。")
        )
    }

    /// Root stored for the next start.
    #[must_use]
    pub fn root_stored_for_next_start(self, root: &str) -> String {
        text!(
            self,
            format!("Root directory set to {root}; it takes effect on the next start."),
            format!("根目录已设置为 {root}；将在下次启动时生效。")
        )
    }

    // --- Service actions ----------------------------------------------------

    /// A start or restart was requested.
    #[must_use]
    pub fn service_starting(self, restart: bool, bind: &str, root: &str) -> String {
        text!(
            self,
            if restart {
                format!("Restarting the server: listening on {bind}, root: {root}.")
            } else {
                format!("Starting the server: listening on {bind}, root: {root}.")
            },
            format!(
                "正在{}服务：监听 {bind}，根目录：{root}。",
                if restart { "重启" } else { "启动" }
            )
        )
    }

    /// The server could not be started or restarted.
    #[must_use]
    pub fn service_start_failed(self, restart: bool, message: &str) -> String {
        text!(
            self,
            if restart {
                format!("Cannot restart the server: {message}")
            } else {
                format!("Cannot start the server: {message}")
            },
            format!(
                "无法{}服务：{message}",
                if restart { "重启" } else { "启动" }
            )
        )
    }

    /// A stop was requested.
    #[must_use]
    pub fn service_stopping(self) -> &'static str {
        text!(
            self,
            "Stopping the server; active transfers are cancelled after the grace period.",
            "正在停止服务；活动传输会按宽限期取消。"
        )
    }

    /// The server became ready.
    #[must_use]
    pub fn service_ready(self, bind: &str, root: &str) -> String {
        text!(
            self,
            format!("Server ready: listening on {bind}, root: {root}."),
            format!("服务已就绪：监听 {bind}，根目录：{root}。")
        )
    }

    /// The server stopped.
    #[must_use]
    pub fn service_stopped(self) -> &'static str {
        text!(self, "Server stopped.", "服务已停止。")
    }

    /// The server task reported a failure.
    #[must_use]
    pub fn service_error(self, message: &str) -> String {
        text!(
            self,
            format!("Server error: {message}."),
            format!("服务异常：{message}。")
        )
    }

    /// An automatic restart failed (log line, without the trailing period of
    /// [`Self::restart_failed`]).
    #[must_use]
    pub fn restart_failed(self, error: &str) -> String {
        text!(
            self,
            format!("Automatic restart failed: {error}"),
            format!("自动重启失败：{error}")
        )
    }

    /// The theme preference could not be written to the state file.
    #[must_use]
    pub fn theme_preference_not_saved(self, error: &str) -> String {
        text!(
            self,
            format!("Cannot remember the appearance setting: {error}; it applies only until the panel closes."),
            format!("无法记住外观设置：{error}；仅本次运行有效。")
        )
    }

    // --- Settings -----------------------------------------------------------

    /// Settings were rejected by the shared validation.
    #[must_use]
    pub fn settings_invalid(self, message: &str) -> String {
        text!(
            self,
            format!("Invalid settings: {message}"),
            format!("设置无效：{message}")
        )
    }

    /// Settings saved; the idle server restarts.
    #[must_use]
    pub fn settings_saved_restarting(self) -> &'static str {
        text!(
            self,
            "Settings saved: no transfer is active, restarting the server.",
            "设置已保存：当前无进行中的传输，正在自动重启服务。"
        )
    }

    /// Settings saved but transfers are active.
    #[must_use]
    pub fn settings_saved_deferred(self, transfers: usize) -> String {
        text!(
            self,
            format!(
                "Settings saved: {transfers} {} in progress, no automatic restart — restart manually later.",
                if transfers == 1 { "transfer is" } else { "transfers are" }
            ),
            format!("设置已保存：有 {transfers} 个进行中的传输，未自动重启，稍后请手动重启。")
        )
    }

    /// Settings saved, automatic restart failed.
    #[must_use]
    pub fn settings_saved_restart_failed(self) -> &'static str {
        text!(
            self,
            "Settings saved, but the automatic restart failed; restart manually.",
            "设置已保存，但自动重启失败；请手动重启。"
        )
    }

    /// Settings saved for the next start.
    #[must_use]
    pub fn settings_saved_next_start(self) -> &'static str {
        text!(
            self,
            "Settings saved; they take effect on the next start.",
            "设置已保存；将在下次启动时生效。"
        )
    }

    /// The saved settings match the running configuration.
    #[must_use]
    pub fn settings_unchanged(self) -> &'static str {
        text!(self, "Settings unchanged.", "设置未发生变化。")
    }

    /// No valid port value.
    #[must_use]
    pub fn port_invalid(self, text: &str) -> String {
        text!(
            self,
            format!("Invalid port “{text}”; expected 1–65535."),
            format!("端口“{text}”无效，应为 1–65535。")
        )
    }

    /// Port 0 is a test-only value.
    #[must_use]
    pub fn port_out_of_range(self) -> &'static str {
        text!(
            self,
            "The port must be between 1 and 65535 (0 is for tests only).",
            "端口必须在 1 到 65535 之间（0 仅用于测试）。"
        )
    }

    /// A bind address could not be parsed.
    #[must_use]
    pub fn bind_address_invalid(self, address: &str) -> String {
        text!(
            self,
            format!("Invalid bind address “{address}”."),
            format!("绑定地址“{address}”无效。")
        )
    }

    /// The bind list would be empty.
    #[must_use]
    pub fn select_bind_address(self) -> &'static str {
        text!(
            self,
            "Select at least one bind address.",
            "请至少选择一个绑定地址。"
        )
    }

    /// A root directory is required before starting.
    #[must_use]
    pub fn choose_root_first(self) -> &'static str {
        text!(self, "Choose a root directory first.", "请先选择根目录。")
    }

    /// `ServerConfig::validate` rejected the form.
    #[must_use]
    pub fn config_validation_failed(self, error: &str) -> String {
        text!(
            self,
            format!("Configuration validation failed: {error}"),
            format!("配置校验失败：{error}")
        )
    }

    /// A numeric settings field is not a number.
    #[must_use]
    pub fn number_invalid(self, label: &str, text: &str) -> String {
        text!(
            self,
            format!("{label} “{text}” is not a valid number."),
            format!("{label}“{text}”不是有效的数字。")
        )
    }

    /// Label of the block-size field, used in validation messages.
    #[must_use]
    pub fn label_block_size(self) -> &'static str {
        text!(self, "Block size", "最大块大小")
    }

    /// Label of the timeout field.
    #[must_use]
    pub fn label_timeout(self) -> &'static str {
        text!(self, "Timeout", "超时")
    }

    /// Label of the session-count field.
    #[must_use]
    pub fn label_max_sessions(self) -> &'static str {
        text!(self, "Maximum concurrent sessions", "最大并发会话")
    }

    // --- Activity log -------------------------------------------------------

    /// The recent-root history was cleared.
    #[must_use]
    pub fn recent_cleared(self) -> &'static str {
        text!(
            self,
            "Recent root directories cleared.",
            "最近使用的根目录已清空。"
        )
    }

    /// A transfer started.
    #[must_use]
    pub fn transfer_started(self, file: &str, size: &str, peer: &str) -> String {
        text!(
            self,
            format!("Transfer started: {file} ({size}), from {peer}."),
            format!("开始传输：{file}（{size}），来自 {peer}。")
        )
    }

    /// A transfer completed.
    #[must_use]
    pub fn transfer_completed(self, file: &str, size: &str, seconds: Option<f32>) -> String {
        let duration_en =
            seconds.map_or_else(String::new, |seconds| format!(", took {seconds:.1} s"));
        let duration_zh =
            seconds.map_or_else(String::new, |seconds| format!("，耗时 {seconds:.1} 秒"));
        text!(
            self,
            format!("Transfer completed: {file} ({size}{duration_en})."),
            format!("传输完成：{file}（{size}{duration_zh}）。")
        )
    }

    /// A transfer failed.
    #[must_use]
    pub fn transfer_failed(self, file: &str, reason: &str) -> String {
        text!(
            self,
            format!("Transfer failed: {file} ({reason})."),
            format!("传输失败：{file}（{reason}）。")
        )
    }

    /// A request was rejected before a transfer started.
    #[must_use]
    pub fn request_rejected(self, file: &str, reason: &str) -> String {
        text!(
            self,
            format!("Request rejected: {file} ({reason})."),
            format!("请求被拒绝：{file}（{reason}）。")
        )
    }

    /// Size of a file is not known yet.
    #[must_use]
    pub fn size_unknown(self) -> &'static str {
        text!(self, "unknown size", "大小未知")
    }

    /// Status cell of an active transfer.
    #[must_use]
    pub fn transfer_in_progress(self) -> &'static str {
        text!(self, "In progress", "传输中")
    }

    /// Count of active transfers.
    #[must_use]
    pub fn active_transfers(self, count: usize) -> String {
        text!(self, format!("{count} active"), format!("{count} 个进行中"))
    }

    /// The activity log was exported.
    #[must_use]
    pub fn log_exported(self, path: &str) -> String {
        text!(
            self,
            format!("Activity log exported: {path}."),
            format!("运行日志已导出：{path}。")
        )
    }

    // --- Log export header --------------------------------------------------

    /// Title line of the exported text file.
    #[must_use]
    pub fn export_title(self) -> &'static str {
        text!(self, "TftpdR activity log", "TftpdR 运行日志")
    }

    /// Product version line of the exported text file.
    #[must_use]
    pub fn export_version(self, version: &str) -> String {
        text!(
            self,
            format!("Version: {version}"),
            format!("版本：{version}")
        )
    }

    /// Export timestamp line of the exported text file.
    #[must_use]
    pub fn export_time(self, timestamp: &str) -> String {
        text!(
            self,
            format!("Exported: {timestamp}"),
            format!("导出时间：{timestamp}")
        )
    }

    /// Row-count line of the exported text file.
    #[must_use]
    pub fn export_rows(self, rows: usize, retained: usize) -> String {
        text!(
            self,
            format!("Rows: {rows} (up to {retained} are retained)"),
            format!("日志条数：{rows}（最多保留 {retained} 条）")
        )
    }

    /// Note that the display filter is not applied to the export.
    #[must_use]
    pub fn export_note(self) -> &'static str {
        text!(
            self,
            "Note: the export contains every retained entry and ignores the display filter.",
            "说明：导出包含全部保留日志，不受界面筛选影响。"
        )
    }

    /// Exporting the activity log failed.
    #[must_use]
    pub fn log_export_failed(self, error: &str) -> String {
        text!(
            self,
            format!("Exporting the activity log failed: {error}."),
            format!("导出运行日志失败：{error}。")
        )
    }

    /// No platform save dialog is available.
    #[must_use]
    pub fn log_export_unavailable(self) -> &'static str {
        text!(
            self,
            "Exporting the activity log failed: no save dialog is available (PowerShell, zenity or kdialog is required).",
            "导出运行日志失败：未找到可用的保存对话框（需要 PowerShell、zenity 或 kdialog）。"
        )
    }

    // --- Native dialogs -----------------------------------------------------

    /// Title of the native folder dialog.
    #[must_use]
    pub fn folder_dialog_title(self) -> &'static str {
        text!(self, "Select the TFTP root directory", "选择 TFTP 根目录")
    }

    /// Title of the native save dialog.
    #[must_use]
    pub fn save_dialog_title(self) -> &'static str {
        text!(self, "Export the activity log", "导出运行日志")
    }

    /// File-type filter of the save dialog.
    #[cfg_attr(not(windows), allow(dead_code))]
    #[must_use]
    pub fn save_dialog_filter(self) -> &'static str {
        text!(
            self,
            "Text files (*.txt)|*.txt|All files (*.*)|*.*",
            "文本文件 (*.txt)|*.txt|所有文件 (*.*)|*.*"
        )
    }

    /// File-type argument for `kdialog`.
    #[cfg_attr(windows, allow(dead_code))]
    #[must_use]
    pub fn save_dialog_kdialog_filter(self) -> &'static str {
        text!(self, "Text files (*.txt)", "文本文件 (*.txt)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplified_chinese_locales_select_chinese() {
        for locale in [
            "zh",
            "zh-CN",
            "zh_CN",
            "zh_CN.UTF-8",
            "zh-Hans",
            "zh-Hans-CN",
            "zh-SG",
            "ZH-cn",
        ] {
            assert_eq!(
                Language::from_locale(Some(locale)),
                Language::ZhCn,
                "locale {locale}"
            );
        }
    }

    #[test]
    fn traditional_chinese_and_other_languages_fall_back_to_english() {
        for locale in [
            "zh-TW",
            "zh_HK",
            "zh-MO",
            "zh-Hant",
            "zh-Hant-TW",
            "en",
            "en-GB",
            "en_US.UTF-8",
            "de-DE",
            "ja-JP",
            "fr",
            "C",
            "POSIX",
            "",
        ] {
            assert_eq!(
                Language::from_locale(Some(locale)),
                Language::En,
                "locale {locale}"
            );
        }
    }

    #[test]
    fn a_missing_locale_is_english() {
        assert_eq!(Language::from_locale(None), Language::En);
        assert_eq!(Language::default(), Language::En);
    }

    #[test]
    fn slint_locale_names_match_the_bundled_catalogues() {
        assert_eq!(Language::En.slint_locale(), "en");
        assert_eq!(Language::ZhCn.slint_locale(), "zh_CN");
    }

    #[test]
    fn status_texts_are_localised() {
        let en = Text::new(Language::En);
        let zh = Text::new(Language::ZhCn);
        assert_eq!(en.status_running(), "Running");
        assert_eq!(zh.status_running(), "运行中");
        assert_eq!(en.root_unset(), "No root directory");
        assert_eq!(zh.root_unset(), "未指定根目录");
    }

    #[test]
    fn list_separators_follow_the_language() {
        assert_eq!(Text::new(Language::En).list_separator(), ", ");
        assert_eq!(Text::new(Language::ZhCn).list_separator(), "、");
    }

    #[test]
    fn transfer_duration_is_optional_and_localised() {
        let en = Text::new(Language::En);
        let zh = Text::new(Language::ZhCn);
        assert_eq!(
            en.transfer_completed("a.bin", "1.0 KiB", Some(1.25)),
            "Transfer completed: a.bin (1.0 KiB, took 1.2 s)."
        );
        assert_eq!(
            zh.transfer_completed("a.bin", "1.0 KiB", None),
            "传输完成：a.bin（1.0 KiB）。"
        );
    }

    #[test]
    fn singular_and_plural_transfer_counts_read_correctly() {
        let en = Text::new(Language::En);
        assert!(en
            .root_switched_deferred("C:/share", 1)
            .contains("1 transfer is in progress"));
        assert!(en
            .root_switched_deferred("C:/share", 3)
            .contains("3 transfers are in progress"));
        assert_eq!(
            Text::new(Language::ZhCn).root_switched_deferred("C:/share", 3),
            "根目录已切换为 C:/share；有 3 个进行中的传输，未自动重启，稍后请手动重启。"
        );
    }

    #[test]
    fn every_message_differs_between_the_languages() {
        let en = Text::new(Language::En);
        let zh = Text::new(Language::ZhCn);
        let english = [
            en.status_starting(),
            en.status_running(),
            en.status_stopping(),
            en.status_start_failed(),
            en.status_stopped(),
            en.status_error(),
            en.root_unset(),
            en.all_networks(),
            en.loopback(),
            en.no_address_selected(),
            en.first_start(),
            en.root_selected_not_saved(),
            en.opening_folder_picker(),
            en.service_stopping(),
            en.service_stopped(),
            en.settings_saved_restarting(),
            en.settings_saved_restart_failed(),
            en.settings_saved_next_start(),
            en.settings_unchanged(),
            en.port_out_of_range(),
            en.select_bind_address(),
            en.choose_root_first(),
            en.recent_cleared(),
            en.size_unknown(),
            en.transfer_in_progress(),
            en.log_export_unavailable(),
            en.folder_dialog_title(),
            en.save_dialog_title(),
        ];
        let chinese = [
            zh.status_starting(),
            zh.status_running(),
            zh.status_stopping(),
            zh.status_start_failed(),
            zh.status_stopped(),
            zh.status_error(),
            zh.root_unset(),
            zh.all_networks(),
            zh.loopback(),
            zh.no_address_selected(),
            zh.first_start(),
            zh.root_selected_not_saved(),
            zh.opening_folder_picker(),
            zh.service_stopping(),
            zh.service_stopped(),
            zh.settings_saved_restarting(),
            zh.settings_saved_restart_failed(),
            zh.settings_saved_next_start(),
            zh.settings_unchanged(),
            zh.port_out_of_range(),
            zh.select_bind_address(),
            zh.choose_root_first(),
            zh.recent_cleared(),
            zh.size_unknown(),
            zh.transfer_in_progress(),
            zh.log_export_unavailable(),
            zh.folder_dialog_title(),
            zh.save_dialog_title(),
        ];
        for (english, chinese) in english.iter().zip(chinese.iter()) {
            assert_ne!(english, chinese, "message not translated: {english}");
        }
    }

    // --- Translation catalogue consistency ---------------------------------

    /// Root of the crate (`.../crates/tftpd-r-gui-slint`).
    fn manifest_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// The one catalogue the panel ships. English is the source language, so it
    /// has no `.po` file of its own.
    const CATALOGUE: &str = "translations/zh_CN/LC_MESSAGES/tftpd-r-gui-slint.po";

    /// Every `.slint` file of the UI in a stable order.
    fn slint_sources() -> Vec<std::path::PathBuf> {
        let dir = manifest_dir().join("ui");
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("read ui directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "slint")
            })
            .collect();
        files.sort();
        assert!(!files.is_empty(), "no .slint files found in {dir:?}");
        files
    }

    /// Extract the `@tr("...")` message ids of one Slint file.
    ///
    /// Only a plain double-quoted literal is supported, which is what the UI
    /// uses; a string with an escaped quote would need a real parser and is
    /// rejected by the assertion below instead of being silently skipped.
    fn translated_strings(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = source;
        while let Some(offset) = rest.find("@tr(\"") {
            let after = &rest[offset + 5..];
            let Some(end) = after.find('"') else {
                panic!("unterminated @tr literal in: {after}");
            };
            let literal = &after[..end];
            assert!(
                !literal.contains('\\'),
                "escaped quotes in @tr() are not supported by this check: {literal}"
            );
            found.push(literal.to_string());
            rest = &after[end + 1..];
        }
        found
    }

    /// Parse a `.po` catalogue into `msgid -> msgstr`.
    fn parse_catalogue(text: &str) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = Vec::new();
        let mut current: Option<(String, Option<String>)> = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(value) = line.strip_prefix("msgid ") {
                if let Some((id, translation)) = current.take() {
                    if !id.is_empty() {
                        entries.push((id, translation.unwrap_or_default()));
                    }
                }
                current = Some((unquote(value), None));
            } else if let Some(value) = line.strip_prefix("msgstr ") {
                let Some((id, _)) = current.take() else {
                    panic!("msgstr without msgid: {line}");
                };
                current = Some((id, Some(unquote(value))));
            } else if line.starts_with("msgctxt ") {
                panic!("the catalogue must not use msgctxt: {line}");
            } else if line.starts_with('"') {
                // Continuation line of a multi-line value.
                let extra = unquote(line);
                if let Some((id, translation)) = current.take() {
                    match translation {
                        Some(text) => current = Some((id, Some(text + &extra))),
                        None => current = Some((id + &extra, None)),
                    }
                }
            }
        }
        if let Some((id, translation)) = current.take() {
            if !id.is_empty() {
                entries.push((id, translation.unwrap_or_default()));
            }
        }
        entries
    }

    fn unquote(value: &str) -> String {
        let value = value.trim();
        assert!(
            value.starts_with('"') && value.ends_with('"') && value.len() >= 2,
            "not a quoted string: {value}"
        );
        value[1..value.len() - 1].replace("\\\"", "\"")
    }

    #[test]
    fn every_translatable_slint_string_has_a_catalogue_entry() {
        let catalogue = parse_catalogue(
            &std::fs::read_to_string(manifest_dir().join(CATALOGUE))
                .expect("read the zh_CN catalogue"),
        );
        let mut missing = Vec::new();
        let mut translated = 0;
        for file in slint_sources() {
            let source = std::fs::read_to_string(&file).expect("read .slint file");
            for message in translated_strings(&source) {
                translated += 1;
                match catalogue.iter().find(|(id, _)| *id == message) {
                    Some((_, translation)) if !translation.is_empty() => {}
                    Some(_) => missing.push(format!("{message} (empty translation)")),
                    None => missing.push(message),
                }
            }
        }
        assert!(
            translated > 0,
            "no @tr() strings found; the UI is not translatable"
        );
        assert!(missing.is_empty(), "missing translations: {missing:#?}");
    }

    #[test]
    fn the_catalogue_has_no_unused_entries() {
        let catalogue = parse_catalogue(
            &std::fs::read_to_string(manifest_dir().join(CATALOGUE))
                .expect("read the zh_CN catalogue"),
        );
        let mut used = Vec::new();
        for file in slint_sources() {
            let source = std::fs::read_to_string(&file).expect("read .slint file");
            used.extend(translated_strings(&source));
        }
        let unused: Vec<&String> = catalogue
            .iter()
            .map(|(id, _)| id)
            .filter(|id| !used.contains(id))
            .collect();
        assert!(
            unused.is_empty(),
            "catalogue entries no longer used by the UI: {unused:#?}"
        );
    }

    #[test]
    fn translations_keep_the_placeholders_of_their_source_string() {
        let catalogue = parse_catalogue(
            &std::fs::read_to_string(manifest_dir().join(CATALOGUE))
                .expect("read the zh_CN catalogue"),
        );
        for (id, translation) in &catalogue {
            let source_slots = id.matches("{}").count();
            let translated_slots = translation.matches("{}").count();
            assert_eq!(
                source_slots, translated_slots,
                "placeholder mismatch for {id:?}: {translation:?}"
            );
        }
    }

    #[test]
    fn the_slint_sources_stay_in_english() {
        // English is the source language of the catalogue: a Chinese literal
        // left in a `.slint` file would never be translated and would still be
        // Chinese in the English UI. Comments may mention Chinese examples.
        for file in slint_sources() {
            let source = std::fs::read_to_string(&file).expect("read .slint file");
            for (number, line) in source.lines().enumerate() {
                let code = line.split("//").next().unwrap_or_default();
                assert!(
                    !code.chars().any(is_cjk),
                    "{}:{}: use @tr() with an English source string: {line}",
                    file.display(),
                    number + 1
                );
            }
        }
    }

    /// Whether `character` is a CJK ideograph (the languages the panel could
    /// accidentally leave hard-coded).
    fn is_cjk(character: char) -> bool {
        matches!(character, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
    }
}
