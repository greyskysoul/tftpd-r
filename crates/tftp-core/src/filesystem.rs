//! Sandboxed file system access.
//!
//! The sandbox validates a client-supplied TFTP name, walks the path one
//! component at a time rejecting reparse points (symlinks/junctions), and only
//! then opens a regular file for reading. The final canonical path is compared
//! component-wise with the canonical root, which avoids the classic
//! `starts_with("C:\\root")` match on `C:\\root2`.
//!
//! # Residual races
//!
//! Windows has no stable, safe-std way to open a file *relative to an already
//! opened directory handle* (openat/`FILE_FLAG_OPEN_REPARSE_POINT` with a
//! relative open). A privileged local attacker who can replace a path component
//! with a junction in the window between the metadata check and the open could
//! therefore still redirect the open. This is documented in
//! `docs/security.md`; the sandbox is not claimed to be a complete defence
//! against an administrator or a same-user attacker with local write access to
//! the exported tree.

use std::fs::{self, File as StdFile, Metadata, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use tokio::fs::File;

use crate::error::ErrorCode;

/// Maximum length of a client-supplied TFTP name in bytes.
pub const MAX_NAME_BYTES: usize = 1024;
/// Maximum length of a single path component.
pub const MAX_COMPONENT_BYTES: usize = 255;

/// A readable file plus the metadata the transfer needs.
#[derive(Debug)]
pub struct OpenedFile {
    /// Async handle positioned at offset 0.
    pub file: File,
    /// Size reported by the opened handle.
    pub size: u64,
    /// Sanitised name for events/logs.
    pub display_name: String,
}

/// A validated root directory.
#[derive(Debug, Clone)]
pub struct Sandbox {
    root: PathBuf,
    max_file_bytes: u64,
}

impl Sandbox {
    /// Canonicalise and validate the root directory.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError`] when the path is missing, not a directory or
    /// cannot be canonicalised.
    pub fn new(root: &Path, max_file_bytes: u64) -> Result<Self, SandboxError> {
        let canonical = fs::canonicalize(root).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => SandboxError::Missing(root.to_path_buf()),
            _ => SandboxError::Canonicalise(root.to_path_buf(), error),
        })?;
        let metadata = fs::metadata(&canonical)
            .map_err(|error| SandboxError::Canonicalise(canonical.clone(), error))?;
        if !metadata.is_dir() {
            return Err(SandboxError::NotDirectory(canonical));
        }
        Ok(Self {
            root: canonical,
            max_file_bytes,
        })
    }

    /// Canonical root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Validate a client-supplied name and join it to the root.
    ///
    /// This performs syntactic validation only; [`Self::open_for_read`] performs
    /// the filesystem checks.
    ///
    /// # Errors
    ///
    /// Returns [`FileError::InvalidName`] or [`FileError::Escape`] for unsafe
    /// names.
    pub fn resolve(&self, name: &str) -> Result<PathBuf, FileError> {
        validate_name(name)?;
        let mut path = self.root.clone();
        for component in split_components(name) {
            path.push(component);
        }
        Ok(path)
    }

    /// Open a file for reading through the sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`FileError`] for invalid names, missing files, permission
    /// problems, reparse points and size limits.
    pub async fn open_for_read(&self, name: &str) -> Result<OpenedFile, FileError> {
        let path = self.resolve(name)?;
        let root = self.root.clone();
        let display_name = crate::events::sanitize_display_name(name);
        let max = self.max_file_bytes;
        let (file, size) = tokio::task::spawn_blocking(move || open_checked(&root, &path, max))
            .await
            .map_err(|error| FileError::Io(io::Error::other(error)))??;
        Ok(OpenedFile {
            file: File::from_std(file),
            size,
            display_name,
        })
    }
}

fn open_checked(
    root: &Path,
    path: &Path,
    max_file_bytes: u64,
) -> Result<(StdFile, u64), FileError> {
    let relative = path.strip_prefix(root).map_err(|_| FileError::Escape)?;
    let mut current = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    if components.is_empty() {
        return Err(FileError::NotRegularFile);
    }
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(map_io_error)?;
        if is_reparse_point(&metadata) {
            return Err(FileError::ReparsePoint);
        }
        let is_last = index + 1 == components.len();
        if is_last {
            if !metadata.is_file() {
                return Err(FileError::NotRegularFile);
            }
            if metadata.len() > max_file_bytes {
                return Err(FileError::TooLarge {
                    size: metadata.len(),
                    limit: max_file_bytes,
                });
            }
        } else if !metadata.is_dir() {
            return Err(FileError::NotRegularFile);
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .open(&current)
        .map_err(map_io_error)?;
    let opened = file.metadata().map_err(map_io_error)?;
    if !opened.is_file() {
        return Err(FileError::NotRegularFile);
    }
    if opened.len() > max_file_bytes {
        return Err(FileError::TooLarge {
            size: opened.len(),
            limit: max_file_bytes,
        });
    }
    // Resolve the path a second time after opening. This catches a final
    // component that was swapped for a link between the metadata check and the
    // open; the remaining race is documented above.
    let canonical = fs::canonicalize(&current).map_err(map_io_error)?;
    if !path_is_within(root, &canonical) {
        return Err(FileError::Escape);
    }
    Ok((file, opened.len()))
}

fn map_io_error(error: io::Error) -> FileError {
    match error.kind() {
        io::ErrorKind::NotFound => FileError::NotFound,
        io::ErrorKind::PermissionDenied => FileError::PermissionDenied,
        _ => FileError::Io(error),
    }
}

#[cfg(windows)]
fn is_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn path_is_within(root: &Path, candidate: &Path) -> bool {
    let mut root_components = root.components();
    let mut candidate_components = candidate.components();
    loop {
        match (root_components.next(), candidate_components.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(expected), Some(actual)) => {
                if !components_equal(expected, actual) {
                    return false;
                }
            }
        }
    }
}

fn components_equal(left: std::path::Component<'_>, right: std::path::Component<'_>) -> bool {
    let left = left.as_os_str();
    let right = right.as_os_str();
    if left == right {
        return true;
    }
    #[cfg(windows)]
    {
        // Windows path comparisons are case-insensitive. `eq_ignore_ascii_case`
        // covers the common ASCII case; the Unicode lower-case comparison is a
        // conservative fallback for non-ASCII directory names.
        match (left.to_str(), right.to_str()) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase(),
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn validate_name(name: &str) -> Result<(), FileError> {
    if name.is_empty() {
        return Err(FileError::InvalidName("empty file name"));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(FileError::InvalidName("file name too long"));
    }
    if name.contains('\0') {
        return Err(FileError::InvalidName("file name contains NUL"));
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(FileError::Escape);
    }
    if name.contains(':') {
        return Err(FileError::Escape);
    }
    let mut total = 0usize;
    for component in split_components(name) {
        if component.is_empty() {
            return Err(FileError::Escape);
        }
        if component == "." || component == ".." {
            return Err(FileError::Escape);
        }
        if component.len() > MAX_COMPONENT_BYTES {
            return Err(FileError::InvalidName("path component too long"));
        }
        if component.chars().any(|ch| ch.is_control()) {
            return Err(FileError::InvalidName(
                "file name contains control characters",
            ));
        }
        if component.ends_with('.') || component.ends_with(' ') {
            return Err(FileError::InvalidName("file name ends with a dot or space"));
        }
        if component.chars().any(is_windows_forbidden_char) {
            return Err(FileError::InvalidName(
                "file name contains a reserved character",
            ));
        }
        if is_windows_reserved_name(component) {
            return Err(FileError::InvalidName(
                "file name is a reserved device name",
            ));
        }
        total += component.len();
    }
    if total == 0 {
        return Err(FileError::InvalidName("empty file name"));
    }
    Ok(())
}

fn split_components(name: &str) -> impl Iterator<Item = &str> {
    name.split(['/', '\\'])
}

fn is_windows_forbidden_char(ch: char) -> bool {
    matches!(ch, '<' | '>' | '"' | '|' | '?' | '*')
}

fn is_windows_reserved_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0')
}

/// File system errors, with a safe client mapping.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// The name is syntactically unacceptable.
    #[error("invalid file name: {0}")]
    InvalidName(&'static str),
    /// The name would escape the root.
    #[error("path escapes the server root")]
    Escape,
    /// A component is a symlink, junction or other reparse point.
    #[error("path contains a reparse point")]
    ReparsePoint,
    /// The file does not exist.
    #[error("file not found")]
    NotFound,
    /// Access was denied by the OS.
    #[error("permission denied")]
    PermissionDenied,
    /// The path is not a regular file.
    #[error("not a regular file")]
    NotRegularFile,
    /// The file exceeds the configured limit.
    #[error("file is {size} bytes, limit is {limit} bytes")]
    TooLarge {
        /// Actual size.
        size: u64,
        /// Configured limit.
        limit: u64,
    },
    /// Any other I/O error.
    #[error("I/O error: {0}")]
    Io(io::Error),
}

impl FileError {
    /// TFTP error code to send to the client.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::NotFound => ErrorCode::FileNotFound,
            Self::PermissionDenied | Self::NotRegularFile => ErrorCode::AccessViolation,
            Self::TooLarge { .. } => ErrorCode::DiskFull,
            Self::InvalidName(_) | Self::Escape | Self::ReparsePoint | Self::Io(_) => {
                ErrorCode::AccessViolation
            }
        }
    }

    /// Client-safe message. Never contains a local path or OS error text.
    #[must_use]
    pub fn client_message(&self) -> &'static str {
        match self {
            Self::NotFound => "File not found",
            Self::PermissionDenied => "Access violation",
            Self::NotRegularFile => "Access violation",
            Self::TooLarge { .. } => "File exceeds the server size limit",
            Self::InvalidName(_) | Self::Escape | Self::ReparsePoint | Self::Io(_) => {
                "Access violation"
            }
        }
    }
}

/// Errors constructing a [`Sandbox`].
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// Root directory does not exist.
    #[error("root directory {0:?} does not exist")]
    Missing(PathBuf),
    /// Root path exists but is not a directory.
    #[error("root path {0:?} is not a directory")]
    NotDirectory(PathBuf),
    /// Root path could not be canonicalised.
    #[error("root path {0:?} could not be resolved: {1}")]
    Canonicalise(PathBuf, io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn sandbox() -> (tempfile::TempDir, Sandbox) {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("hello.txt"), b"hello").expect("write");
        fs::create_dir(dir.path().join("sub")).expect("mkdir");
        fs::write(dir.path().join("sub").join("nested.bin"), b"nested").expect("write");
        let sandbox = Sandbox::new(dir.path(), 1024).expect("sandbox");
        (dir, sandbox)
    }

    #[test]
    fn resolves_plain_and_nested_names() {
        let (dir, sandbox) = sandbox();
        assert_eq!(
            sandbox.resolve("hello.txt").expect("resolve"),
            fs::canonicalize(dir.path())
                .expect("canon")
                .join("hello.txt")
        );
        assert!(sandbox.resolve("sub/nested.bin").is_ok());
        assert!(sandbox.resolve("sub\\nested.bin").is_ok());
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        let (_dir, sandbox) = sandbox();
        for name in [
            "../secret",
            "..",
            ".",
            "./hello.txt",
            "sub/../../x",
            "/etc/passwd",
            r"C:\Windows\win.ini",
            r"\\server\share\x",
            "a//b",
            "a/",
            "a\\",
            "x:stream",
        ] {
            assert!(
                sandbox.resolve(name).is_err(),
                "expected rejection for {name:?}"
            );
        }
    }

    #[test]
    fn rejects_windows_device_and_odd_names() {
        let (_dir, sandbox) = sandbox();
        for name in [
            "CON",
            "nul.txt",
            "COM1",
            "LPT9.log",
            "trailing.",
            "trailing ",
            "bad?.txt",
            "bad*name",
            "a\tb",
        ] {
            assert!(
                sandbox.resolve(name).is_err(),
                "expected rejection for {name:?}"
            );
        }
    }

    #[tokio::test]
    async fn opens_regular_file_and_reports_size() {
        let (_dir, sandbox) = sandbox();
        let opened = sandbox.open_for_read("hello.txt").await.expect("open");
        assert_eq!(opened.size, 5);
        assert_eq!(opened.display_name, "hello.txt");
    }

    #[tokio::test]
    async fn missing_file_maps_to_not_found() {
        let (_dir, sandbox) = sandbox();
        let error = sandbox
            .open_for_read("nope.txt")
            .await
            .expect_err("must fail");
        assert!(matches!(error, FileError::NotFound));
        assert_eq!(error.error_code(), ErrorCode::FileNotFound);
    }

    #[tokio::test]
    async fn directories_are_not_served() {
        let (_dir, sandbox) = sandbox();
        let error = sandbox.open_for_read("sub").await.expect_err("must fail");
        assert!(matches!(error, FileError::NotRegularFile));
    }

    #[tokio::test]
    async fn size_limit_is_enforced() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut file = fs::File::create(dir.path().join("big.bin")).expect("create");
        file.write_all(&[0u8; 32]).expect("write");
        let sandbox = Sandbox::new(dir.path(), 16).expect("sandbox");
        let error = sandbox
            .open_for_read("big.bin")
            .await
            .expect_err("must fail");
        assert!(matches!(error, FileError::TooLarge { .. }));
        assert_eq!(error.error_code(), ErrorCode::DiskFull);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn file_symlink_inside_root_is_rejected() {
        let dir = tempfile::tempdir().expect("temp dir");
        let outside = tempfile::tempdir().expect("outside");
        fs::write(outside.path().join("secret.txt"), b"secret").expect("write");
        let link = dir.path().join("link.txt");
        if std::os::windows::fs::symlink_file(outside.path().join("secret.txt"), &link).is_err() {
            // Creating symlinks requires Developer Mode or elevation; skip.
            eprintln!("skipping symlink test: OS refused to create a symlink");
            return;
        }
        let sandbox = Sandbox::new(dir.path(), 1024).expect("sandbox");
        let error = sandbox
            .open_for_read("link.txt")
            .await
            .expect_err("must fail");
        assert!(
            matches!(error, FileError::ReparsePoint),
            "unexpected: {error:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_symlink_inside_root_is_rejected() {
        let dir = tempfile::tempdir().expect("temp dir");
        let outside = tempfile::tempdir().expect("outside");
        fs::write(outside.path().join("secret.txt"), b"secret").expect("write");
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("link.txt"),
        )
        .expect("symlink");
        let sandbox = Sandbox::new(dir.path(), 1024).expect("sandbox");
        let error = sandbox
            .open_for_read("link.txt")
            .await
            .expect_err("must fail");
        assert!(
            matches!(error, FileError::ReparsePoint),
            "unexpected: {error:?}"
        );
    }

    #[test]
    fn prefix_confusion_is_rejected_by_component_comparison() {
        let root = Path::new(r"C:\root");
        assert!(path_is_within(root, Path::new(r"C:\root\a.bin")));
        assert!(!path_is_within(root, Path::new(r"C:\root2\a.bin")));
        assert!(!path_is_within(root, Path::new(r"C:\other\a.bin")));
    }
}
