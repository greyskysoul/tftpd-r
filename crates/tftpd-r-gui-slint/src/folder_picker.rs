//! Native folder picker without a GUI-toolkit dependency.
//!
//! The workspace does not depend on a file-dialog crate and hand-written FFI is
//! forbidden, so the platform's own dialog is shown through a helper process:
//! Windows PowerShell (WinForms `FolderBrowserDialog`) or, on Unix, `zenity` /
//! `kdialog` when installed. The helper runs on a background thread, so the
//! Slint event loop stays responsive; the selected directory is validated before
//! it is used. Commands are fixed, no shell is involved, and the initial
//! directory is passed through an environment variable so a path can never be
//! interpreted as script text.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

/// Windows `CREATE_NO_WINDOW`: the helper must not flash a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Start the platform picker on a background thread.
///
/// The returned receiver yields `Some(path)` when the user picked a directory
/// and `None` when the dialog was cancelled or no picker is available.
#[must_use]
pub fn pick_folder_async(initial: Option<PathBuf>) -> Receiver<Option<PathBuf>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(pick_folder(initial.as_deref()));
    });
    receiver
}

fn pick_folder(initial: Option<&Path>) -> Option<PathBuf> {
    let output = platform_pick(initial)?;
    accept_selection(&String::from_utf8_lossy(&output.stdout))
}

/// Turn the helper's stdout into a directory path, rejecting anything that is
/// not an existing directory (and any stray diagnostic lines).
fn accept_selection(stdout: &str) -> Option<PathBuf> {
    let text = stdout.trim();
    if text.is_empty() {
        return None;
    }
    let path = PathBuf::from(text);
    path.is_dir().then_some(path)
}

#[cfg(windows)]
fn platform_pick(initial: Option<&Path>) -> Option<std::process::Output> {
    use std::os::windows::process::CommandExt;

    // `SelectedPath` is read from an environment variable: interpolating the
    // path into the script would be an injection risk.
    let script = concat!(
        "Add-Type -AssemblyName System.Windows.Forms; ",
        "$dialog = New-Object System.Windows.Forms.FolderBrowserDialog; ",
        "$dialog.Description = '选择 TFTP 根目录'; ",
        "$dialog.ShowNewFolderButton = $true; ",
        "$initial = $env:TFTPD_R_INITIAL_DIR; ",
        "if ($initial -and (Test-Path -LiteralPath $initial)) { $dialog.SelectedPath = $initial }; ",
        "if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) ",
        "{ [Console]::Out.WriteLine($dialog.SelectedPath) }"
    );
    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from);
    let program = system_root
        .map(|root| {
            root.join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("powershell.exe"));
    let mut command = std::process::Command::new(program);
    command
        .args(["-NoLogo", "-NoProfile", "-STA", "-Command", script])
        .env(
            "TFTPD_R_INITIAL_DIR",
            initial.unwrap_or_else(|| Path::new("")),
        )
        .creation_flags(CREATE_NO_WINDOW);
    command.output().ok()
}

#[cfg(not(windows))]
fn platform_pick(initial: Option<&Path>) -> Option<std::process::Output> {
    let initial = initial.unwrap_or_else(|| Path::new(""));
    let zenity = ["/usr/bin/zenity", "/usr/local/bin/zenity"]
        .into_iter()
        .find(|program| Path::new(program).is_file());
    if let Some(program) = zenity {
        let mut command = std::process::Command::new(program);
        command.args([
            "--file-selection",
            "--directory",
            "--title=选择 TFTP 根目录",
            "--filename",
        ]);
        command.arg(format!("{}/", initial.display()));
        if let Ok(output) = command.output() {
            if output.status.success() {
                return Some(output);
            }
        }
    }
    let kdialog = ["/usr/bin/kdialog", "/usr/local/bin/kdialog"]
        .into_iter()
        .find(|program| Path::new(program).is_file())?;
    let mut command = std::process::Command::new(kdialog);
    command.arg("--getexistingdirectory").arg(initial);
    command.output().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_selection_is_a_cancelled_dialog() {
        assert_eq!(accept_selection(""), None);
        assert_eq!(accept_selection("   \r\n"), None);
    }

    #[test]
    fn a_non_directory_is_rejected() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("file.txt");
        std::fs::write(&file, b"x").expect("write");
        assert_eq!(accept_selection(&file.to_string_lossy()), None);
        assert_eq!(
            accept_selection(&dir.path().to_string_lossy()),
            Some(dir.path().to_path_buf())
        );
    }
}
