//! In-memory activity log for the panel: levels, display filter and export.
//!
//! Entries carry a local wall-clock timestamp and are kept newest-first, up to
//! [`MAX_LOG_ROWS`]. The filter only affects what the panel displays; the
//! retained entries (and therefore the text export) are complete. Nothing is
//! written to disk automatically: an operator exports the log explicitly from
//! the panel, because the lines contain local paths and peer addresses.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::Path;

use chrono::{DateTime, FixedOffset, Local};

/// Maximum number of log entries retained in memory.
pub const MAX_LOG_ROWS: usize = 500;

/// Severity of one activity-log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Normal operation.
    Info,
    /// A request or transfer problem that did not stop the server.
    Warn,
    /// A failed action the operator has to know about.
    Error,
}

impl LogLevel {
    /// Stable label shown in the panel and in the text export.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// One retained log line with its local timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Local wall-clock time of the entry, including the UTC offset.
    pub at: DateTime<FixedOffset>,
    /// Severity used by the filter.
    pub level: LogLevel,
    /// Operator-facing message. [`LogEntry::now`] collapses it to one line;
    /// construct the struct directly only in tests that control the text.
    pub message: String,
}

impl LogEntry {
    /// Create an entry stamped with the current local time.
    ///
    /// Messages are collapsed to a single line: configuration and OS errors can
    /// contain embedded newlines, which would break the fixed-height log row and
    /// the one-line-per-entry text export.
    #[must_use]
    pub fn now(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            at: Local::now().fixed_offset(),
            level,
            message: single_line(message.into()),
        }
    }

    /// Compact `HH:MM:SS` label for the log table.
    #[must_use]
    pub fn display_time(&self) -> String {
        self.at.format("%H:%M:%S").to_string()
    }
}

/// Collapse every whitespace run (including embedded newlines) to one space.
fn single_line(message: String) -> String {
    let mut text = String::with_capacity(message.len());
    let mut pending_space = false;
    for character in message.chars() {
        if character.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !text.is_empty() {
                text.push(' ');
            }
            pending_space = false;
            text.push(character);
        }
    }
    text
}

/// Which retained levels the log table shows.
///
/// This is an exact-level filter: selecting `WARN` hides INFO and ERROR rows,
/// not only INFO. `All` is the default. The filter never drops entries, so
/// clearing it restores the complete retained log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFilter {
    /// Show every retained level.
    #[default]
    All,
    /// Show INFO rows only.
    Info,
    /// Show WARN rows only.
    Warn,
    /// Show ERROR rows only.
    Error,
}

impl LogFilter {
    /// Stable key used by the Slint property and the filter buttons.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    /// Parse a Slint filter key; unknown values fall back to [`Self::All`].
    #[must_use]
    pub fn from_key(key: &str) -> Self {
        match key {
            "INFO" => Self::Info,
            "WARN" => Self::Warn,
            "ERROR" => Self::Error,
            _ => Self::All,
        }
    }

    /// Whether an entry of `level` is visible under this filter.
    #[must_use]
    pub fn allows(self, level: LogLevel) -> bool {
        match self {
            Self::All => true,
            Self::Info => level == LogLevel::Info,
            Self::Warn => level == LogLevel::Warn,
            Self::Error => level == LogLevel::Error,
        }
    }
}

/// File name offered by the export dialog.
#[must_use]
pub fn export_file_name(at: DateTime<FixedOffset>) -> String {
    format!("tftpd-r-log-{}.txt", at.format("%Y%m%d-%H%M%S"))
}

/// Render the complete retained log as a plain-text export.
///
/// Entries are kept newest-first in memory but written oldest-first so the file
/// reads chronologically. The header records the product version, the export
/// time and the retained row count.
#[must_use]
pub fn format_export(
    entries: &VecDeque<LogEntry>,
    app_version: &str,
    exported_at: DateTime<FixedOffset>,
) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "TftpdR 运行日志");
    let _ = writeln!(text, "版本：{app_version}");
    let _ = writeln!(
        text,
        "导出时间：{}",
        exported_at.format("%Y-%m-%d %H:%M:%S %:z")
    );
    let _ = writeln!(
        text,
        "日志条数：{}（最多保留 {MAX_LOG_ROWS} 条）",
        entries.len()
    );
    let _ = writeln!(text, "说明：导出包含全部保留日志，不受界面筛选影响。");
    let _ = writeln!(text, "----------------------------------------");
    for entry in entries.iter().rev() {
        let _ = writeln!(
            text,
            "{} {} {}",
            entry.at.format("%Y-%m-%d %H:%M:%S"),
            entry.level.as_str(),
            entry.message
        );
    }
    text
}

/// Write an exported log to the path the operator chose.
///
/// Kept separate from the dialog so the file content can be tested without a
/// graphical session. Existing files are replaced only after the save dialog
/// confirmed the overwrite.
///
/// # Errors
///
/// Returns the filesystem error when the path cannot be written.
pub fn write_export(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32, second: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 3600)
            .expect("offset")
            .with_ymd_and_hms(2026, 11, 21, hour, minute, second)
            .single()
            .expect("timestamp")
    }

    fn entry(at: DateTime<FixedOffset>, level: LogLevel, message: &str) -> LogEntry {
        LogEntry {
            at,
            level,
            message: message.to_string(),
        }
    }

    #[test]
    fn levels_have_stable_labels() {
        assert_eq!(LogLevel::Info.as_str(), "INFO");
        assert_eq!(LogLevel::Warn.as_str(), "WARN");
        assert_eq!(LogLevel::Error.as_str(), "ERROR");
    }

    #[test]
    fn filters_are_exact_level_and_all_shows_everything() {
        assert!(LogFilter::All.allows(LogLevel::Info));
        assert!(LogFilter::All.allows(LogLevel::Warn));
        assert!(LogFilter::All.allows(LogLevel::Error));

        assert!(LogFilter::Info.allows(LogLevel::Info));
        assert!(!LogFilter::Info.allows(LogLevel::Warn));
        assert!(!LogFilter::Info.allows(LogLevel::Error));

        assert!(!LogFilter::Warn.allows(LogLevel::Info));
        assert!(LogFilter::Warn.allows(LogLevel::Warn));
        assert!(!LogFilter::Warn.allows(LogLevel::Error));

        assert!(!LogFilter::Error.allows(LogLevel::Info));
        assert!(!LogFilter::Error.allows(LogLevel::Warn));
        assert!(LogFilter::Error.allows(LogLevel::Error));
    }

    #[test]
    fn filter_keys_roundtrip_and_unknown_values_fall_back() {
        for filter in [
            LogFilter::All,
            LogFilter::Info,
            LogFilter::Warn,
            LogFilter::Error,
        ] {
            assert_eq!(LogFilter::from_key(filter.key()), filter);
        }
        assert_eq!(LogFilter::from_key("all"), LogFilter::All);
        assert_eq!(LogFilter::from_key("DEBUG"), LogFilter::All);
        assert_eq!(LogFilter::default(), LogFilter::All);
    }

    #[test]
    fn export_is_oldest_first_with_dates_and_the_full_header() {
        let mut entries = VecDeque::new();
        // `push_log` prepends, so pushing in chronological order leaves the
        // newest entry at the front, like the panel model.
        entries.push_front(entry(at(9, 0, 1), LogLevel::Info, "first"));
        entries.push_front(entry(at(9, 0, 2), LogLevel::Warn, "second"));
        entries.push_front(entry(at(9, 0, 3), LogLevel::Error, "third"));

        let text = format_export(&entries, "0.2.1", at(9, 5, 0));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "TftpdR 运行日志");
        assert_eq!(lines[1], "版本：0.2.1");
        assert_eq!(lines[2], "导出时间：2026-11-21 09:05:00 +08:00");
        assert_eq!(lines[3], "日志条数：3（最多保留 500 条）");
        assert_eq!(lines[4], "说明：导出包含全部保留日志，不受界面筛选影响。");
        assert_eq!(
            &lines[6..],
            [
                "2026-11-21 09:00:01 INFO first",
                "2026-11-21 09:00:02 WARN second",
                "2026-11-21 09:00:03 ERROR third",
            ]
        );
    }

    #[test]
    fn export_file_name_is_timestamped_and_uses_txt() {
        assert_eq!(
            export_file_name(at(9, 5, 7)),
            "tftpd-r-log-20261121-090507.txt"
        );
    }

    #[test]
    fn entry_display_time_is_compact_wall_clock() {
        let entry = entry(at(23, 59, 58), LogLevel::Info, "late");
        assert_eq!(entry.display_time(), "23:59:58");
    }

    #[test]
    fn messages_with_embedded_newlines_stay_on_one_line() {
        let entry = LogEntry::now(
            LogLevel::Warn,
            "config.toml 不可用：invalid configuration TOML:\n  line 1, column 6\r\n\t|",
        );
        assert_eq!(
            entry.message,
            "config.toml 不可用：invalid configuration TOML: line 1, column 6 |"
        );
        assert!(!entry.message.contains('\n'));
        assert!(!entry.message.contains('\r'));
        assert!(!entry.message.contains('\t'));
    }

    #[test]
    fn export_writes_the_full_text_to_the_chosen_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("tftpd-r-log-20261121-090507.txt");
        let mut entries = VecDeque::new();
        entries.push_front(entry(at(9, 0, 1), LogLevel::Warn, "warning"));
        let text = format_export(&entries, "0.2.1", at(9, 5, 7));

        write_export(&path, &text).expect("write export");
        assert_eq!(std::fs::read_to_string(&path).expect("read export"), text);
    }

    #[test]
    fn export_write_failures_are_reported() {
        let dir = tempfile::tempdir().expect("temp dir");
        let error = write_export(dir.path(), "cannot write into a directory")
            .expect_err("writing to a directory must fail");
        assert!(error.kind() != std::io::ErrorKind::NotFound);
    }
}
