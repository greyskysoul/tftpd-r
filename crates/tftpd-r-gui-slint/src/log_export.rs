//! Native save-file dialog for exporting the activity log.
//!
//! Like [`crate::folder_picker`], the workspace does not add a GUI-toolkit
//! dependency for native dialogs and hand-written FFI is forbidden. The
//! platform dialog is therefore shown by a helper process (Windows PowerShell
//! WinForms `SaveFileDialog`, or `zenity`/`kdialog` on Unix) on a background
//! thread. The helper only returns a path; the Rust side writes the text, so a
//! path can never be interpreted as script text.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use crate::i18n::Text;

/// Result of the save-file dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveChoice {
    /// The operator chose a destination path (the file may not exist yet).
    Path(PathBuf),
    /// The dialog was cancelled or closed.
    Cancelled,
    /// No platform dialog could be started (missing helper process).
    Unavailable,
}

/// Show the save dialog on a background thread.
///
/// `default_name` is a file name, not a path: it is passed through an
/// environment variable (Windows) or a fixed argument (Unix), never through a
/// shell. The dialog title and file-type filter are translated by the caller;
/// they travel the same way, so no message text ever becomes script text.
#[must_use]
pub fn save_log_async(default_name: String, text: Text) -> Receiver<SaveChoice> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(platform_save(&default_name, text));
    });
    receiver
}

/// Turn helper stdout into a destination path.
///
/// An empty/whitespace-only line means the dialog was cancelled; any non-empty
/// line is a path chosen by the operator and does not have to exist yet.
fn accept_save_path(stdout: &str) -> Option<PathBuf> {
    let text = stdout.trim();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

#[cfg(windows)]
fn platform_save(default_name: &str, text: Text) -> SaveChoice {
    use std::os::windows::process::CommandExt;

    let script = concat!(
        "Add-Type -AssemblyName System.Windows.Forms; ",
        "Add-Type -AssemblyName System.Drawing; ",
        "$dialog = New-Object System.Windows.Forms.SaveFileDialog; ",
        "$dialog.Title = $env:TFTPD_R_SAVE_TITLE; ",
        "$dialog.Filter = $env:TFTPD_R_SAVE_FILTER; ",
        "$dialog.FileName = $env:TFTPD_R_LOG_NAME; ",
        "$documents = [Environment]::GetFolderPath('MyDocuments'); ",
        "if ($documents) { $dialog.InitialDirectory = $documents }; ",
        "$owner = New-Object System.Windows.Forms.Form; ",
        "$owner.ShowInTaskbar = $false; ",
        "$owner.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::None; ",
        "$owner.StartPosition = [System.Windows.Forms.FormStartPosition]::CenterScreen; ",
        "$owner.Size = New-Object System.Drawing.Size(1, 1); ",
        "$owner.Opacity = 0; ",
        "$owner.TopMost = $true; ",
        "$owner.Show(); ",
        "$result = $dialog.ShowDialog($owner); ",
        "$owner.Close(); ",
        "if ($result -eq [System.Windows.Forms.DialogResult]::OK) ",
        "{ [Console]::Out.WriteLine($dialog.FileName) }"
    );
    let output = std::process::Command::new(crate::folder_picker::powershell_program())
        .args(["-NoLogo", "-NoProfile", "-STA", "-Command", script])
        .env("TFTPD_R_LOG_NAME", default_name)
        .env("TFTPD_R_SAVE_TITLE", text.save_dialog_title())
        .env("TFTPD_R_SAVE_FILTER", text.save_dialog_filter())
        .creation_flags(crate::folder_picker::CREATE_NO_WINDOW)
        .output();
    match output {
        Ok(output) => match accept_save_path(&String::from_utf8_lossy(&output.stdout)) {
            Some(path) => SaveChoice::Path(path),
            None => SaveChoice::Cancelled,
        },
        Err(_) => SaveChoice::Unavailable,
    }
}

#[cfg(not(windows))]
fn platform_save(default_name: &str, text: Text) -> SaveChoice {
    use std::path::Path;

    let zenity = ["/usr/bin/zenity", "/usr/local/bin/zenity"]
        .into_iter()
        .find(|program| Path::new(program).is_file());
    if let Some(program) = zenity {
        let output = std::process::Command::new(program)
            .args([
                "--file-selection",
                "--save",
                "--confirm-overwrite",
                &format!("--title={}", text.save_dialog_title()),
            ])
            .arg(format!("--filename={default_name}"))
            .output();
        return match output {
            Ok(output) if output.status.success() => {
                match accept_save_path(&String::from_utf8_lossy(&output.stdout)) {
                    Some(path) => SaveChoice::Path(path),
                    None => SaveChoice::Cancelled,
                }
            }
            Ok(_) => SaveChoice::Cancelled,
            Err(_) => SaveChoice::Cancelled,
        };
    }
    let kdialog = ["/usr/bin/kdialog", "/usr/local/bin/kdialog"]
        .into_iter()
        .find(|program| Path::new(program).is_file());
    if let Some(program) = kdialog {
        let output = std::process::Command::new(program)
            .arg("--title")
            .arg(text.save_dialog_title())
            .arg("--getsavefilename")
            .arg(default_name)
            .arg(text.save_dialog_kdialog_filter())
            .output();
        return match output {
            Ok(output) if output.status.success() => {
                match accept_save_path(&String::from_utf8_lossy(&output.stdout)) {
                    Some(path) => SaveChoice::Path(path),
                    None => SaveChoice::Cancelled,
                }
            }
            Ok(_) => SaveChoice::Cancelled,
            Err(_) => SaveChoice::Cancelled,
        };
    }
    SaveChoice::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_selection_is_a_cancelled_dialog() {
        assert_eq!(accept_save_path(""), None);
        assert_eq!(accept_save_path("   \r\n"), None);
    }

    #[test]
    fn a_chosen_path_is_kept_even_when_the_file_does_not_exist_yet() {
        assert_eq!(
            accept_save_path("C:\\Users\\me\\Documents\\tftpd-r-log.txt\r\n"),
            Some(PathBuf::from("C:\\Users\\me\\Documents\\tftpd-r-log.txt"))
        );
        assert_eq!(
            accept_save_path("/tmp/tftpd-r log.txt\n"),
            Some(PathBuf::from("/tmp/tftpd-r log.txt"))
        );
    }
}
