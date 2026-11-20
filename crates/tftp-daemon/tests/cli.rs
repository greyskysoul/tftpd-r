//! End-to-end CLI tests for the daemon binary.
//!
//! These run the compiled executable, so they cover argument parsing, config
//! loading, validation and exit codes together.

use std::fs;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_tftp-daemon")
}

fn example_config_with_root(root: &std::path::Path) -> String {
    include_str!("../../../config.example.toml").replace(
        "root_dir = \"tftp-root\"",
        &format!("root_dir = {:?}", root.to_string_lossy()),
    )
}

#[test]
fn print_default_config_is_the_example_and_validates() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = Command::new(binary())
        .arg("print-default-config")
        .output()
        .expect("run binary");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("utf-8 output");
    assert_eq!(text, include_str!("../../../config.example.toml"));

    let root = dir.path().join("tftp-root");
    fs::create_dir(&root).expect("create root");
    let config_path = dir.path().join("config.toml");
    fs::write(&config_path, example_config_with_root(&root)).expect("write config");

    let check = Command::new(binary())
        .arg("check-config")
        .arg("--config")
        .arg(&config_path)
        .output()
        .expect("run check-config");
    assert!(
        check.status.success(),
        "check-config failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert!(
        stdout.contains("configuration OK"),
        "unexpected output: {stdout}"
    );
    assert!(stdout.contains("read-only"), "unexpected output: {stdout}");
}

#[test]
fn check_config_rejects_uploads_with_exit_code_two() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("config.toml");
    fs::write(
        &config_path,
        "bind_address = \"127.0.0.1:0\"\nroot_dir = \"irrelevant\"\nallow_upload = true\n",
    )
    .expect("write config");
    let output = Command::new(binary())
        .arg("check-config")
        .arg("--config")
        .arg(&config_path)
        .output()
        .expect("run check-config");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("allow_upload"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn check_config_reports_missing_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = Command::new(binary())
        .arg("check-config")
        .arg("--config")
        .arg(dir.path().join("absent.toml"))
        .output()
        .expect("run check-config");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not read"));
}

#[test]
fn serve_refuses_invalid_configuration() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("config.toml");
    fs::write(
        &config_path,
        "root_dir = \"irrelevant\"\nallow_upload = true\n",
    )
    .expect("write");
    let output = Command::new(binary())
        .arg("serve")
        .arg("--config")
        .arg(&config_path)
        .output()
        .expect("run serve");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("allow_upload"));
}

#[test]
fn serve_starts_and_keeps_running() {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path().join("root");
    fs::create_dir(&root).expect("create root");
    fs::write(root.join("hello.txt"), b"hello").expect("write file");
    let config_path = dir.path().join("config.toml");
    let config = example_config_with_root(&root).replace(
        "bind_address = \"127.0.0.1:6969\"",
        "bind_address = \"127.0.0.1:0\"",
    );
    fs::write(&config_path, config).expect("write config");

    let mut child = Command::new(binary())
        .arg("serve")
        .arg("--config")
        .arg(&config_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            panic!("daemon exited unexpectedly: {status}");
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    child.kill().expect("kill daemon");
    let _ = child.wait();
}
