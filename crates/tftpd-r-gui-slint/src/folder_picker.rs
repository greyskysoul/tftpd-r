//! Native folder picker without a GUI-toolkit dependency.
//!
//! The workspace does not depend on a file-dialog crate and hand-written FFI is
//! forbidden, so the platform's own dialog is shown through a helper process:
//! Windows PowerShell (WinForms `FolderBrowserDialog`) or, on Unix, `zenity` /
//! `kdialog` when installed.
//!
//! Starting PowerShell and loading WinForms costs 1–2 s on a typical Windows
//! machine, so the panel keeps one **pre-warmed** helper alive for the session
//! instead of paying that cost on every click:
//!
//! 1. The first time the settings dialog or the recent-root menu is opened, the
//!    worker starts the helper in the background and waits (off the UI thread)
//!    until it reports `READY`. Nothing is shown yet, so the panel still starts
//!    without a dialog and without blocking.
//! 2. A click sends one request line (the initial directory) over stdin. Only
//!    the dialog itself remains in the click path: measured ~0.06–0.37 s
//!    instead of ~1.6 s on the reference machine.
//! 3. If the helper cannot be started or dies, requests fall back to the
//!    original one-shot helper so the button keeps working, just slower. The
//!    helper is killed when the panel shuts down.
//!
//! Commands are fixed, no shell is involved, and neither the initial directory
//! nor the selected path is ever interpolated into script text: the directory
//! travels over stdin and the result over stdout, both as UTF-8. UTF-8 matters:
//! `[Console]::Out` uses the OEM code page (for example GBK), which used to
//! garble non-ASCII directory names.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

#[cfg(windows)]
use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use std::process::{Child, ChildStdin, Command, Stdio};
#[cfg(windows)]
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::Duration;

/// Windows `CREATE_NO_WINDOW`: the helper must not flash a console window.
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Longest the pre-warmed helper may take to report `READY` before the worker
/// gives up on it and uses the one-shot helper instead.
#[cfg(windows)]
const HELPER_START_TIMEOUT: Duration = Duration::from_secs(10);

/// Handle to the background folder-picker service.
///
/// The worker thread is cheap and always running; the Windows helper process is
/// only started by [`FolderPicker::prewarm`] or the first
/// [`FolderPicker::request`], and is reused for every later dialog.
pub struct FolderPicker {
    messages: Sender<Message>,
    /// The helper process, shared so [`FolderPicker::shutdown`] can stop a
    /// dialog that is still open. The worker owns the stdin/stdout handles.
    #[cfg(windows)]
    child: Arc<Mutex<Option<Child>>>,
}

/// Work item of the folder-picker worker.
enum Message {
    /// Start the helper without showing a dialog, so the next click is fast.
    Warm,
    /// Show the dialog and report the selection (or `None` when cancelled).
    Pick {
        initial: Option<PathBuf>,
        reply: Sender<Option<PathBuf>>,
    },
    /// Stop the worker and kill the helper; sent when the panel closes.
    Shutdown,
}

impl FolderPicker {
    /// Start the worker thread. No helper process is spawned yet.
    #[must_use]
    pub fn new() -> Self {
        let (messages, receiver) = mpsc::channel();
        #[cfg(windows)]
        {
            let child = Arc::new(Mutex::new(None));
            let worker_child = Arc::clone(&child);
            let _ = thread::spawn(move || run_worker(receiver, worker_child));
            Self { messages, child }
        }
        #[cfg(not(windows))]
        {
            let _ = thread::spawn(move || run_worker(receiver));
            Self { messages }
        }
    }

    /// Ask the worker to start the helper in the background.
    ///
    /// Idempotent: a warm helper and a failed helper are both left alone, so the
    /// UI timer can call this without spawning PowerShell repeatedly.
    pub fn prewarm(&self) {
        let _ = self.messages.send(Message::Warm);
    }

    /// Show the folder dialog and return a receiver for its result.
    ///
    /// The dialog runs on the background worker, so the Slint event loop stays
    /// responsive; the UI timer polls the receiver. A disconnected receiver is
    /// treated as a cancelled dialog by the caller.
    #[must_use]
    pub fn request(&self, initial: Option<PathBuf>) -> Receiver<Option<PathBuf>> {
        let (reply, receiver) = mpsc::channel();
        let _ = self.messages.send(Message::Pick { initial, reply });
        receiver
    }

    /// Stop the helper, including a dialog that is still open.
    ///
    /// Called after the event loop returns; the worker exits on the following
    /// `Shutdown` message and never leaves an orphaned PowerShell process
    /// behind.
    pub fn shutdown(&self) {
        let _ = self.messages.send(Message::Shutdown);
        #[cfg(windows)]
        if let Ok(mut slot) = self.child.lock() {
            if let Some(mut child) = slot.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[cfg(windows)]
fn run_worker(messages: Receiver<Message>, child: Arc<Mutex<Option<Child>>>) {
    let mut helper: Option<Helper> = None;
    // A helper that could not be started (or died) is not retried: every later
    // request uses the one-shot helper, which keeps the button usable but loses
    // the warm-up speed instead of failing twice per click.
    let mut helper_failed = false;
    while let Ok(message) = messages.recv() {
        match message {
            Message::Warm => {
                if helper.is_none() && !helper_failed {
                    helper = Helper::start(Arc::clone(&child));
                    helper_failed = helper.is_none();
                }
            }
            Message::Pick { initial, reply } => {
                if helper.is_none() && !helper_failed {
                    helper = Helper::start(Arc::clone(&child));
                    helper_failed = helper.is_none();
                }
                if let Some(active) = helper.as_mut() {
                    match active.pick(initial.as_deref()) {
                        Some(picked) => {
                            let _ = reply.send(picked);
                            continue;
                        }
                        None => {
                            // The dialog never returns when the helper dies
                            // mid-request; report a cancelled dialog and use the
                            // one-shot path from now on.
                            helper = None;
                            helper_failed = true;
                            let _ = reply.send(None);
                            continue;
                        }
                    }
                }
                let _ = reply.send(pick_folder_once(initial.as_deref()));
            }
            Message::Shutdown => break,
        }
    }
}

#[cfg(not(windows))]
fn run_worker(messages: Receiver<Message>) {
    while let Ok(message) = messages.recv() {
        if let Message::Pick { initial, reply } = message {
            let _ = reply.send(pick_folder_once(initial.as_deref()));
        }
    }
}

/// One PowerShell process that loads WinForms once and shows one dialog per
/// request line, so the ~1.5 s startup no longer sits in the click path.
#[cfg(windows)]
struct Helper {
    stdin: ChildStdin,
    lines: Receiver<String>,
    child: Arc<Mutex<Option<Child>>>,
}

#[cfg(windows)]
impl Helper {
    fn start(slot: Arc<Mutex<Option<Child>>>) -> Option<Self> {
        let mut command = Command::new(powershell_program());
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-STA",
                "-Command",
                PERSISTENT_SCRIPT,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        let mut process = command.spawn().ok()?;
        let Some(stdin) = process.stdin.take() else {
            let _ = process.kill();
            let _ = process.wait();
            return None;
        };
        let Some(stdout) = process.stdout.take() else {
            let _ = process.kill();
            let _ = process.wait();
            return None;
        };
        match slot.lock() {
            Ok(mut slot) => *slot = Some(process),
            Err(_) => {
                let _ = process.kill();
                let _ = process.wait();
                return None;
            }
        }
        let (sender, lines) = mpsc::channel();
        let _ = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let helper = Self {
            stdin,
            lines,
            child: slot,
        };
        match helper.lines.recv_timeout(HELPER_START_TIMEOUT) {
            Ok(line) if line == "READY" => Some(helper),
            // Dropping the local helper kills the process it just started.
            _ => None,
        }
    }

    /// Send one initial directory and wait for `PICK:<path>` or `PICK:`.
    ///
    /// `None` means the helper died; `Some(None)` is a cancelled dialog.
    fn pick(&mut self, initial: Option<&Path>) -> Option<Option<PathBuf>> {
        let mut request =
            initial.map_or_else(String::new, |path| path.to_string_lossy().into_owned());
        // A Windows path cannot contain a line break, so a line is an
        // unambiguous request; an empty line simply means "no initial path".
        request.push('\n');
        self.stdin.write_all(request.as_bytes()).ok()?;
        self.stdin.flush().ok()?;
        let reply = self.lines.recv().ok()?;
        parse_reply(&reply)
    }
}

#[cfg(windows)]
impl Drop for Helper {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.child.lock() {
            if let Some(mut child) = slot.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Parse one helper reply line. `PICK:` alone is a cancelled dialog; anything
/// else is not part of the protocol (for example a stray banner line).
#[cfg(any(windows, test))]
fn parse_reply(line: &str) -> Option<Option<PathBuf>> {
    let path = line.strip_prefix("PICK:")?;
    Some((!path.is_empty()).then(|| PathBuf::from(path)))
}

fn pick_folder_once(initial: Option<&Path>) -> Option<PathBuf> {
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
const PERSISTENT_SCRIPT: &str = concat!(
    "$ErrorActionPreference = 'Stop'; ",
    "Add-Type -AssemblyName System.Windows.Forms; ",
    "Add-Type -AssemblyName System.Drawing; ",
    "$utf8 = New-Object System.Text.UTF8Encoding $false; ",
    "$out = New-Object System.IO.StreamWriter([Console]::OpenStandardOutput(), $utf8); ",
    "$out.AutoFlush = $true; ",
    "$in = New-Object System.IO.StreamReader([Console]::OpenStandardInput(), $utf8); ",
    "$out.WriteLine('READY'); ",
    "while ($true) { ",
    "$initial = $in.ReadLine(); ",
    "if ($null -eq $initial) { break }; ",
    "$dialog = New-Object System.Windows.Forms.FolderBrowserDialog; ",
    "$dialog.Description = '选择 TFTP 根目录'; ",
    "$dialog.ShowNewFolderButton = $true; ",
    "if ($initial -and (Test-Path -LiteralPath $initial)) { $dialog.SelectedPath = $initial }; ",
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
    "{ $out.WriteLine('PICK:' + $dialog.SelectedPath) } else { $out.WriteLine('PICK:') }; ",
    "$dialog.Dispose(); $owner.Dispose() ",
    "}"
);

#[cfg(windows)]
fn platform_pick(initial: Option<&Path>) -> Option<std::process::Output> {
    // `SelectedPath` is read from an environment variable: interpolating the
    // path into the script would be an injection risk.
    //
    // The dialog is shown with a tiny, invisible, top-most owner form. Without
    // an owner the helper process's dialog can be created behind the panel (or
    // behind whatever window currently has the foreground); the top-most owner
    // keeps it visible and activated.
    let script = concat!(
        "Add-Type -AssemblyName System.Windows.Forms; ",
        "Add-Type -AssemblyName System.Drawing; ",
        "$dialog = New-Object System.Windows.Forms.FolderBrowserDialog; ",
        "$dialog.Description = '选择 TFTP 根目录'; ",
        "$dialog.ShowNewFolderButton = $true; ",
        "$initial = $env:TFTPD_R_INITIAL_DIR; ",
        "if ($initial -and (Test-Path -LiteralPath $initial)) { $dialog.SelectedPath = $initial }; ",
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
        "{ $utf8 = New-Object System.Text.UTF8Encoding $false; ",
        "$out = New-Object System.IO.StreamWriter([Console]::OpenStandardOutput(), $utf8); ",
        "$out.AutoFlush = $true; $out.WriteLine($dialog.SelectedPath) }"
    );
    let mut command = std::process::Command::new(powershell_program());
    command
        .args(["-NoLogo", "-NoProfile", "-STA", "-Command", script])
        .env(
            "TFTPD_R_INITIAL_DIR",
            initial.unwrap_or_else(|| Path::new("")),
        )
        .creation_flags(CREATE_NO_WINDOW);
    command.output().ok()
}

/// Absolute path of the built-in Windows PowerShell, falling back to a PATH
/// lookup when `SystemRoot` is unavailable.
#[cfg(windows)]
pub(crate) fn powershell_program() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| {
            root.join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("powershell.exe"))
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

    #[test]
    fn a_non_ascii_directory_is_accepted() {
        // PowerShell used to print the path in the OEM code page, so a Chinese
        // directory name arrived as invalid UTF-8 and was silently discarded.
        let dir = tempfile::tempdir().expect("temp dir");
        let share = dir.path().join("固件 共享");
        std::fs::create_dir(&share).expect("share dir");
        assert_eq!(
            accept_selection(&share.to_string_lossy()),
            Some(share.clone())
        );
    }

    #[test]
    fn helper_replies_are_parsed_without_mangling_the_path() {
        assert_eq!(parse_reply("PICK:"), Some(None));
        assert_eq!(
            parse_reply("PICK:C:\\srv\\固件 共享"),
            Some(Some(PathBuf::from("C:\\srv\\固件 共享")))
        );
        assert_eq!(parse_reply("PICK:D:\\"), Some(Some(PathBuf::from("D:\\"))));
        // Readiness and stray helper output are not pick results.
        assert_eq!(parse_reply("READY"), None);
        assert_eq!(parse_reply(""), None);
    }

    /// End-to-end check of the warm helper: start it, open a dialog and verify
    /// that shutting the panel down closes the dialog and unblocks the worker.
    /// Ignored because it opens a real native window and needs a desktop
    /// session; run it manually on Windows with
    /// `cargo test -p tftpd-r-gui-slint -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore = "opens the native folder dialog; run manually on a desktop session"]
    fn the_warm_helper_closes_its_dialog_on_shutdown() {
        use std::sync::mpsc::TryRecvError;
        use std::time::{Duration, Instant};

        let picker = FolderPicker::new();
        picker.prewarm();
        // WinForms needs ~1.5 s to load; the worker starts it in the
        // background, so give it time before opening the dialog.
        std::thread::sleep(Duration::from_secs(3));
        let receiver = picker.request(None);
        // The dialog is open now. Shutting down must kill the helper, close
        // the window and report a cancelled pick instead of hanging.
        std::thread::sleep(Duration::from_secs(1));
        picker.shutdown();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match receiver.try_recv() {
                Ok(result) => {
                    assert_eq!(result, None, "a closed dialog must be a cancel");
                    break;
                }
                Err(TryRecvError::Empty) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                other => panic!("unexpected picker result: {other:?}"),
            }
        }
    }
}
