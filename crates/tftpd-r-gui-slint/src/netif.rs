//! Best-effort discovery of the host's unicast interface addresses.
//!
//! The workspace has no native interface-enumeration dependency and hand-written
//! FFI is forbidden (`unsafe_code = "deny"`), so the platform's own runtime is
//! asked for the list and only IP literals are parsed from its output. On
//! Windows that is the .NET `NetworkInterface` API (no WMI, no administrator
//! rights, unlike `Get-NetIPAddress`); elsewhere `hostname -I`. The command is a
//! fixed absolute path, no shell is involved and no user input is interpolated
//! into it. Discovery failure is not fatal: the settings dialog still offers the
//! wildcard and loopback entries.
//!
//! Link-local addresses (`fe80::/10`, `169.254.0.0/16`) and loopback addresses
//! are filtered out: link-local addresses are auto-configured and either not
//! useful for a service or, for IPv6, cannot be expressed without a scope id in
//! a [`std::net::SocketAddr`]. The settings dialog adds the loopback entry
//! explicitly.

use std::net::{IpAddr, Ipv6Addr};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Longest a platform helper may take before it is killed.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Windows `CREATE_NO_WINDOW`: the helper must not flash a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Discover and normalise local unicast addresses (IPv4 first, duplicates
/// removed). Returns an empty vector when the platform helper is unavailable.
#[must_use]
pub fn local_addresses() -> Vec<IpAddr> {
    normalize(platform_addresses())
}

/// Keep only addresses the server can bind meaningfully and make the order
/// deterministic.
fn normalize(mut addresses: Vec<IpAddr>) -> Vec<IpAddr> {
    addresses.retain(is_usable);
    addresses.sort_by_key(|address| (address.is_ipv6(), *address));
    addresses.dedup();
    addresses
}

fn is_usable(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            !ip.is_unspecified()
                && !ip.is_broadcast()
                && !ip.is_multicast()
                && !ip.is_loopback()
                && !ip.is_link_local()
        }
        IpAddr::V6(ip) => {
            !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_loopback()
                && !is_ipv6_link_local(ip)
        }
    }
}

fn is_ipv6_link_local(address: &Ipv6Addr) -> bool {
    (address.segments()[0] & 0xffc0) == 0xfe80
}

/// Parse whitespace/comma separated IP literals, ignoring anything else.
fn parse_addresses(text: &str) -> Vec<IpAddr> {
    text.split(|character: char| character.is_whitespace() || character == ',' || character == ';')
        .filter_map(|token| token.parse::<IpAddr>().ok())
        .collect()
}

#[cfg(windows)]
fn platform_addresses() -> Vec<IpAddr> {
    // .NET's NetworkInterface API wraps GetAdaptersAddresses: it needs no WMI or
    // CIM access and works under restricted tokens where Get-NetIPAddress
    // fails with "access denied".
    let script = "[System.Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces() \
                  | ForEach-Object { $_.GetIPProperties().UnicastAddresses } \
                  | ForEach-Object { $_.Address.IPAddressToString }";
    let Some(mut command) = powershell_command() else {
        return Vec::new();
    };
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        script,
    ]);
    with_no_console(&mut command);
    match run_with_timeout(command, DISCOVERY_TIMEOUT) {
        Some(output) => parse_addresses(&String::from_utf8_lossy(&output.stdout)),
        None => Vec::new(),
    }
}

#[cfg(not(windows))]
fn platform_addresses() -> Vec<IpAddr> {
    for program in ["/usr/bin/hostname", "/bin/hostname"] {
        if !PathBuf::from(program).is_file() {
            continue;
        }
        let Ok(output) = Command::new(program).arg("-I").output() else {
            continue;
        };
        if output.status.success() {
            let addresses = parse_addresses(&String::from_utf8_lossy(&output.stdout));
            if !addresses.is_empty() {
                return addresses;
            }
        }
    }
    Vec::new()
}

/// Absolute path to Windows PowerShell, falling back to `PATH` lookup.
#[cfg(windows)]
fn powershell_command() -> Option<Command> {
    let absolute = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| {
            root.join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .filter(|path| path.is_file());
    match absolute {
        Some(path) => Some(Command::new(path)),
        None => Some(Command::new("powershell.exe")),
    }
}

#[cfg(windows)]
fn with_no_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Spawn a helper, give it a bounded amount of time and collect its stdout.
fn run_with_timeout(mut command: Command, timeout: Duration) -> Option<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_ip_literals_from_helper_output() {
        let addrs = parse_addresses(
            "192.168.1.5\n10.0.0.7\n\nfe80::1%12\n::1\nnot-an-address\n224.0.0.1\n",
        );
        // `fe80::1%12` keeps its zone id in the helper output and cannot be
        // parsed as a plain IpAddr; link-local addresses are not offered anyway.
        assert_eq!(addrs.len(), 4);
        assert!(addrs.contains(&"192.168.1.5".parse().expect("ip")));
        assert!(addrs.contains(&"::1".parse().expect("ip")));
        assert!(addrs.contains(&"224.0.0.1".parse().expect("ip")));
    }

    #[test]
    fn normalise_drops_unusable_addresses_and_sorts_ipv4_first() {
        let addrs = normalize(vec![
            "::1".parse().expect("ip"),
            "192.168.1.5".parse().expect("ip"),
            "0.0.0.0".parse().expect("ip"),
            "127.0.0.1".parse().expect("ip"),
            "fe80::1".parse().expect("ip"),
            "224.0.0.1".parse().expect("ip"),
            "169.254.1.1".parse().expect("ip"),
            "10.0.0.7".parse().expect("ip"),
            "10.0.0.7".parse().expect("ip"),
            "2001:db8::1".parse().expect("ip"),
        ]);
        assert_eq!(
            addrs,
            vec![
                "10.0.0.7".parse::<IpAddr>().expect("ip"),
                "192.168.1.5".parse::<IpAddr>().expect("ip"),
                "2001:db8::1".parse::<IpAddr>().expect("ip"),
            ]
        );
    }

    #[test]
    fn link_local_ipv6_is_not_offered() {
        assert!(is_ipv6_link_local(&"fe80::1".parse().expect("ip")));
        assert!(is_ipv6_link_local(&"febf::1".parse().expect("ip")));
        assert!(!is_ipv6_link_local(&"2001:db8::1".parse().expect("ip")));
        assert!(!is_ipv6_link_local(&"fd00::1".parse().expect("ip")));
        assert!(!is_ipv6_link_local(&Ipv6Addr::LOCALHOST));
    }

    #[test]
    fn discovery_never_panics_and_only_returns_usable_addresses() {
        for address in local_addresses() {
            assert!(is_usable(&address), "unusable address {address}");
        }
    }
}
