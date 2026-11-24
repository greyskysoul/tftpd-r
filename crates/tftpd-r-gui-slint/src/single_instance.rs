//! Startup check for an already running panel.
//!
//! A second launch must not silently create a second panel: the operator is
//! asked whether to raise the window that is already open, to start another
//! instance on purpose, or to cancel. This module implements the small
//! inter-process handshake that decision needs, without a new dependency and
//! without `unsafe`:
//!
//! - Every running panel owns one lock file `instance-<token>.lock` in the
//!   per-user state directory and refreshes its modification time regularly. A
//!   lock file whose timestamp is older than [`STALE_AFTER`] belongs to a crashed
//!   or frozen process and is ignored (and cleaned up).
//! - A launch that finds such a lock file asks the owner for help by writing
//!   `activate-request-<token>.req` with a short deadline. The owner polls for
//!   requests from its 200 ms UI timer, removes the request, raises its window
//!   and writes `activate-ack-<token>.ack`; the requester sees the
//!   acknowledgement and exits.
//! - "Start a new instance" registers a fresh lock and continues normally, so the
//!   two panels coexist and a third launch offers the choice again.
//!
//! The files are advisory UI state, never server configuration. Every failure is
//! reported as a value (no panics, no blocking waits on the UI thread), and the
//! worst case is the previous behaviour: two panels run side by side. This is a
//! local convenience feature, not an IPC channel: nothing listens on the network,
//! and the files stay inside the state directory described in `user_state.rs`.
//!
//! The token in a file name is generated from the process ID, the wall clock and
//! a per-process counter. It is not a secret: it only has to keep the files of
//! two processes apart.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use chrono::Utc;

/// Prefix of one running panel's lock file.
pub const LOCK_PREFIX: &str = "instance-";
/// Prefix of an activation request written by a second launch.
pub const REQUEST_PREFIX: &str = "activate-request-";
/// Prefix of the answer written by the panel that owns the lock.
pub const ACK_PREFIX: &str = "activate-ack-";
/// Suffix of lock files.
pub const LOCK_SUFFIX: &str = ".lock";
/// Suffix of activation requests.
pub const REQUEST_SUFFIX: &str = ".req";
/// Suffix of activation acknowledgements.
pub const ACK_SUFFIX: &str = ".ack";

/// Length of the hex token in a file name. Tokens of any other length are not
/// ours and are never deleted.
const TOKEN_LEN: usize = 32;

/// How often the owning panel refreshes its lock file. The UI timer runs every
/// 200 ms, so the refresh is only performed when this interval has elapsed.
pub const HEARTBEAT: Duration = Duration::from_secs(3);

/// A lock file older than this is considered abandoned. It has to be
/// comfortably longer than [`HEARTBEAT`], so a busy machine does not turn a live
/// panel into a "stale" one.
pub const STALE_AFTER: Duration = Duration::from_secs(15);

/// How long a launch waits for the owning panel to acknowledge an activation
/// request before reporting that the existing window did not respond.
pub const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(2);

/// Current wall-clock time in milliseconds since the Unix epoch.
#[must_use]
pub fn now_millis() -> u64 {
    u64::try_from(Utc::now().timestamp_millis()).unwrap_or(0)
}

/// A panel that is already running, as reported to the second launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Background {
    /// Process ID of the running panel, from its lock file (display only).
    pub pid: u32,
    /// Name token of the running panel's lock file.
    pub token: String,
}

/// Lock file of this process, once it owns one.
#[derive(Debug)]
pub struct Owner {
    dir: PathBuf,
    token: String,
    heartbeat: Instant,
}

impl Owner {
    /// Look for another panel in `dir` without registering anything.
    ///
    /// A lock file that is newer than [`STALE_AFTER`] wins. Stale lock files
    /// (crashed or frozen panels) are removed on the way. A lock whose content
    /// cannot be parsed still means somebody is alive, so it is reported with a
    /// process ID of 0 instead of being deleted.
    #[must_use]
    pub fn detect(dir: &Path, now: u64) -> Option<Background> {
        for (token, path, fresh) in lock_files(dir, now) {
            if fresh {
                return Some(read_lock(&path, &token).unwrap_or(Background { pid: 0, token }));
            }
            remove_quietly(&path);
        }
        None
    }

    /// Register this process as the owner of a new lock file.
    ///
    /// Called after the operator decided to start this instance, or at the start
    /// of a launch that found no other panel.
    #[must_use]
    pub fn register(dir: &Path) -> Self {
        let owner = Self {
            dir: dir.to_path_buf(),
            token: new_token(),
            heartbeat: Instant::now(),
        };
        owner.write_lock(now_millis());
        owner
    }

    /// Own token of this process; also the name part of its lock file.
    #[cfg(test)]
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Refresh the lock file timestamp.
    ///
    /// Called from the UI timer; it is a no-op between heartbeats. Returns
    /// whether a write was attempted, so the caller can log a failure once.
    pub fn heartbeat(&mut self, now: u64) -> bool {
        if self.heartbeat.elapsed() < HEARTBEAT {
            return false;
        }
        self.heartbeat = Instant::now();
        self.write_lock(now);
        true
    }

    /// Remove one activation request left by a second launch and answer it.
    ///
    /// Returns the request that was claimed, if any. The request file is removed
    /// before the answer is written, so two panels can never both raise their
    /// window for the same request. Expired and malformed requests are removed
    /// silently: the launch that wrote them has already given up.
    pub fn claim_activation(&mut self, now: u64) -> Option<Background> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return None;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(client_token) = file_token(&path, REQUEST_PREFIX, REQUEST_SUFFIX) else {
                continue;
            };
            if client_token == self.token {
                continue;
            }
            let Some(pid) = read_request(&path, now) else {
                remove_quietly(&path);
                continue;
            };
            // Remove before answering: the successful removal is what makes the
            // claim exclusive between two panels polling the same directory.
            if std::fs::remove_file(&path).is_err() {
                continue;
            }
            self.write_ack(&client_token);
            return Some(Background {
                pid,
                token: client_token,
            });
        }
        None
    }

    fn lock_path(&self) -> PathBuf {
        lock_path(&self.dir, &self.token)
    }

    fn ack_path(&self, client_token: &str) -> PathBuf {
        ack_path(&self.dir, client_token)
    }

    fn write_lock(&self, now: u64) {
        let content = format!(
            "pid={}\ntoken={}\nheartbeat={now}\nstale-after={}\n",
            std::process::id(),
            self.token,
            millis(STALE_AFTER)
        );
        if let Err(error) = write_atomic(&self.lock_path(), content.as_bytes()) {
            // Losing the lock file only means the next launch cannot see this
            // panel; the running server is unaffected.
            eprintln!("warning: cannot refresh the single-instance lock: {error}");
        }
    }

    fn write_ack(&self, client_token: &str) {
        // The content carries the *client* token (the one in the file name), so
        // the requester can verify that the answer belongs to its request.
        let content = format!(
            "pid={}\ntoken={client_token}\nnow={}\n",
            std::process::id(),
            now_millis()
        );
        if let Err(error) = write_atomic(&self.ack_path(client_token), content.as_bytes()) {
            eprintln!("warning: cannot acknowledge the activation request: {error}");
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        remove_quietly(&self.lock_path());
    }
}

/// A second launch's request that the running panel show its window.
#[derive(Debug)]
pub struct ActivationRequest {
    path: PathBuf,
    token: String,
    /// Deadline embedded in the request file. The owner ignores a request after
    /// it passed, so a second launch that already gave up cannot raise the panel
    /// later.
    deadline: u64,
}

impl ActivationRequest {
    /// Ask the panel described by `background` to show its window.
    ///
    /// The request carries a deadline so the owner ignores it if the requester
    /// has already given up (for example because the operator chose "cancel"
    /// while the request was in flight).
    #[must_use]
    pub fn send(dir: &Path, background: &Background, now: u64) -> Self {
        let token = new_token();
        let deadline = now.saturating_add(millis(ACTIVATION_TIMEOUT));
        let path = dir.join(format!("{REQUEST_PREFIX}{token}{REQUEST_SUFFIX}"));
        let content = format!(
            "pid={}\ntoken={}\ndeadline={deadline}\nowner={}\n",
            std::process::id(),
            token,
            background.token
        );
        if let Err(error) = write_atomic(&path, content.as_bytes()) {
            eprintln!("warning: cannot write the activation request: {error}");
        }
        Self {
            path,
            token,
            deadline,
        }
    }

    /// The request token, which names the acknowledgement file.
    #[cfg(test)]
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Whether the owner has acknowledged and raised its window.
    ///
    /// The acknowledgement is removed when it is read, so a stale answer cannot
    /// satisfy a later request.
    #[must_use]
    pub fn poll_ack(&self, dir: &Path) -> bool {
        let path = ack_path(dir, &self.token);
        let Ok(content) = std::fs::read_to_string(&path) else {
            return false;
        };
        let valid = content.lines().any(|line| {
            line.strip_prefix("token=")
                .is_some_and(|value| value.trim() == self.token)
        });
        if valid {
            remove_quietly(&path);
        }
        valid
    }

    /// Whether the acknowledgement deadline has passed.
    #[must_use]
    pub fn expired(&self, now: u64) -> bool {
        now >= self.deadline
    }

    /// Remove a request that was never answered.
    pub fn discard(&self) {
        remove_quietly(&self.path);
    }
}

impl Drop for ActivationRequest {
    fn drop(&mut self) {
        // A request that is still waiting when the process exits must not wake
        // the owner later; an answered one is already gone.
        self.discard();
    }
}

/// Name of one panel's lock file.
fn lock_path(dir: &Path, token: &str) -> PathBuf {
    dir.join(format!("{LOCK_PREFIX}{token}{LOCK_SUFFIX}"))
}

/// Name of the acknowledgement for `client_token`.
fn ack_path(dir: &Path, client_token: &str) -> PathBuf {
    dir.join(format!("{ACK_PREFIX}{client_token}{ACK_SUFFIX}"))
}

/// One lock file found in the directory, with a freshness check against `now`.
fn lock_files(dir: &Path, now: u64) -> Vec<(String, PathBuf, bool)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(token) = file_token(&path, LOCK_PREFIX, LOCK_SUFFIX) else {
            continue;
        };
        let modified = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok();
        files.push((token, path, is_fresh(modified, now)));
    }
    files
}

/// Whether a lock file modified at `modified` still describes a live panel.
///
/// A timestamp in the future (a clock adjustment) counts as live: refusing to
/// offer the activation choice is better than deleting the lock of a running
/// panel.
fn is_fresh(modified: Option<SystemTime>, now: u64) -> bool {
    let Some(modified) = modified else {
        return false;
    };
    let modified = unix_millis_of(modified);
    modified > now || now.saturating_sub(modified) < millis(STALE_AFTER)
}

/// Read a lock file's process ID.
fn read_lock(path: &Path, token: &str) -> Option<Background> {
    let content = std::fs::read_to_string(path).ok()?;
    let pid = content
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|value| value.trim().parse().ok())?;
    Some(Background {
        pid,
        token: token.to_string(),
    })
}

/// Read a request's process ID and check that it has not expired.
fn read_request(path: &Path, now: u64) -> Option<u32> {
    let content = std::fs::read_to_string(path).ok()?;
    let pid = content
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|value| value.trim().parse().ok())?;
    let deadline = content
        .lines()
        .find_map(|line| line.strip_prefix("deadline="))
        .and_then(|value| value.trim().parse::<u64>().ok())?;
    (now < deadline).then_some(pid)
}

/// Token part of `name` when it starts with `prefix`, ends with `suffix` and the
/// token has the expected shape. Anything else is not one of our files and is
/// never touched.
fn file_token(path: &Path, prefix: &str, suffix: &str) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let token = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if token.len() != TOKEN_LEN || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(token.to_ascii_lowercase())
}

/// Write `path` through a unique temporary file so another process never reads
/// a half-written lock, request or acknowledgement.
fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let result = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(content)?;
        file.sync_all()
    })();
    if let Err(error) = result {
        remove_quietly(&temporary);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary, path) {
        remove_quietly(&temporary);
        return Err(error);
    }
    Ok(())
}

/// Remove `path` and ignore "not found" and permission errors: every caller is
/// cleaning up advisory state and must not fail the panel.
fn remove_quietly(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Milliseconds of a duration.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Milliseconds of a filesystem timestamp, or 0 when it predates the epoch.
fn unix_millis_of(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, millis)
}

/// A new hex token for a lock, request or acknowledgement file name.
fn new_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let pid = u64::from(std::process::id());
    let nanos = now_nanos();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{:016x}{:016x}",
        fnv1a(pid ^ nanos.rotate_left(17) ^ count.rotate_left(33)),
        fnv1a(nanos ^ pid.rotate_left(29) ^ count)
    )
}

/// Nanoseconds of the wall clock, clamped to 0 before the epoch.
fn now_nanos() -> u64 {
    u64::try_from(Utc::now().timestamp_nanos_opt().unwrap_or(0)).unwrap_or(0)
}

/// FNV-1a over the 64-bit input, used only to spread similar inputs.
fn fnv1a(mut value: u64) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for _ in 0..8 {
        hash ^= value & 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        value >>= 8;
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::time::{Duration, SystemTime};

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    /// Age a file so the freshness check sees it as stale.
    fn make_stale(path: &Path) {
        let old = SystemTime::now() - Duration::from_secs(120);
        let file = File::options()
            .write(true)
            .open(path)
            .expect("open lock file");
        file.set_times(std::fs::FileTimes::new().set_modified(old))
            .expect("set file time");
    }

    #[test]
    fn an_empty_directory_has_no_running_panel() {
        let dir = dir();
        assert_eq!(Owner::detect(dir.path(), now_millis()), None);
    }

    #[test]
    fn a_fresh_lock_and_its_activation_handshake_round_trip() {
        let dir = dir();
        let now = now_millis();
        let mut owner = Owner::register(dir.path());
        assert!(dir
            .path()
            .join(format!("{LOCK_PREFIX}{}{LOCK_SUFFIX}", owner.token()))
            .is_file());

        // A second launch finds the owner and asks for its window.
        let detected = Owner::detect(dir.path(), now).expect("the fresh lock must be found");
        assert_eq!(detected.token, owner.token());
        assert_eq!(detected.pid, std::process::id());

        let request = ActivationRequest::send(dir.path(), &detected, now);
        assert!(
            !request.poll_ack(dir.path()),
            "no answer has been written yet"
        );
        let claimed = owner
            .claim_activation(now)
            .expect("the owner must claim the request");
        assert_eq!(claimed.pid, std::process::id());
        assert_eq!(claimed.token, request.token());
        assert!(
            request.poll_ack(dir.path()),
            "the owner must acknowledge the request"
        );
        assert!(
            owner.claim_activation(now).is_none(),
            "a claimed request must not be handled twice"
        );
    }

    #[test]
    fn a_stale_lock_file_is_ignored_and_cleaned_up() {
        let dir = dir();
        let path = lock_path(dir.path(), &new_token());
        std::fs::write(&path, "pid=1\ntoken=dead\n").expect("write lock");
        make_stale(&path);

        assert_eq!(
            Owner::detect(dir.path(), now_millis()),
            None,
            "an abandoned lock must not prompt"
        );
        assert!(!path.exists(), "the stale lock file is removed");
    }

    #[test]
    fn unrelated_files_and_wrong_token_lengths_are_never_touched() {
        let dir = dir();
        let unrelated = [
            "recent-dirs.txt",
            "instance-.lock",
            "instance-short.lock",
            "instance-not-hex-at-all-not-hex-at-all.lock",
        ];
        for name in unrelated {
            std::fs::write(dir.path().join(name), "x").expect("write file");
        }
        // Mark them stale: even then the name filter must protect them.
        for name in unrelated {
            make_stale(&dir.path().join(name));
        }
        assert_eq!(Owner::detect(dir.path(), now_millis()), None);
        for name in unrelated {
            assert!(
                dir.path().join(name).exists(),
                "{name} must survive the cleanup"
            );
        }
    }

    #[test]
    fn an_expired_request_is_dropped_without_an_answer() {
        let dir = dir();
        let now = now_millis();
        let mut owner = Owner::register(dir.path());
        let background = Background {
            pid: 42,
            token: "0".repeat(TOKEN_LEN),
        };
        let request = ActivationRequest::send(dir.path(), &background, now - 60_000);
        assert!(request.expired(now), "the deadline is in the past");
        assert!(owner.claim_activation(now).is_none());
        assert!(
            !ack_path(dir.path(), request.token()).exists(),
            "an expired request must not be answered"
        );
        assert!(!request.path.exists(), "the expired request is removed");
    }

    #[test]
    fn dropping_the_owner_removes_its_lock_file() {
        let dir = dir();
        let owner = Owner::register(dir.path());
        let path = lock_path(dir.path(), owner.token());
        assert!(path.is_file());
        drop(owner);
        assert!(!path.exists(), "the lock must not outlive the panel");
    }

    #[test]
    fn tokens_are_unique_well_formed_and_parsed_back() {
        let first = new_token();
        let second = new_token();
        assert_eq!(first.len(), TOKEN_LEN);
        assert_ne!(first, second);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            file_token(&lock_path(Path::new("."), &first), LOCK_PREFIX, LOCK_SUFFIX),
            Some(first)
        );
        assert_eq!(
            file_token(Path::new("instance-xyz.lock"), LOCK_PREFIX, LOCK_SUFFIX),
            None
        );
    }
}
