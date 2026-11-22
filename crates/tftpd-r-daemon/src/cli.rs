//! Command line surface of the headless daemon.
//!
//! Parsing is separated from execution so argument handling can be unit tested
//! without binding sockets or touching the file system.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Headless TFTP server with configuration file and graceful shutdown.
#[derive(Debug, Parser)]
#[command(
    name = "tftpd-r",
    version,
    about = "TftpdR: read-only TFTP (RFC 1350) server daemon with a sandboxed root directory",
    long_about = None
)]
pub struct Cli {
    /// Subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the server in the foreground until Ctrl-C.
    Serve(ServeArgs),
    /// Validate a configuration file and print a summary; binds no socket.
    CheckConfig(CheckConfigArgs),
    /// Print an annotated default configuration to stdout.
    PrintDefaultConfig,
}

/// Arguments for `serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Path to the TOML configuration file.
    #[arg(short, long, value_name = "FILE", default_value = "config.toml")]
    pub config: PathBuf,
    /// Override the configured UDP listen address and port (replaces the
    /// configured address list with this single listener).
    #[arg(long, value_name = "ADDR")]
    pub bind: Option<SocketAddr>,
    /// Override the configured root directory.
    #[arg(long, value_name = "DIR")]
    pub root: Option<PathBuf>,
    /// Log filter: a level (off/error/warn/info/debug/trace) or comma-separated
    /// `target=level` directives; overrides the TFTPD_R_LOG environment variable.
    #[arg(long, value_name = "FILTER")]
    pub log: Option<String>,
}

/// Arguments for `check-config`.
#[derive(Debug, Args)]
pub struct CheckConfigArgs {
    /// Path to the TOML configuration file.
    #[arg(short, long, value_name = "FILE", default_value = "config.toml")]
    pub config: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_defaults_to_config_toml() {
        let cli = Cli::try_parse_from(["tftpd-r", "serve"]).expect("parse");
        let Command::Serve(args) = cli.command else {
            panic!("expected serve")
        };
        assert_eq!(args.config, PathBuf::from("config.toml"));
        assert!(args.bind.is_none());
    }

    #[test]
    fn serve_accepts_overrides() {
        let cli = Cli::try_parse_from([
            "tftpd-r",
            "serve",
            "--config",
            "server.toml",
            "--bind",
            "127.0.0.1:16969",
            "--root",
            "C:\\srv\\tftp",
        ])
        .expect("parse");
        let Command::Serve(args) = cli.command else {
            panic!("expected serve")
        };
        assert_eq!(args.config, PathBuf::from("server.toml"));
        assert_eq!(args.bind, Some("127.0.0.1:16969".parse().expect("addr")));
        assert_eq!(args.root, Some(PathBuf::from("C:\\srv\\tftp")));
    }

    #[test]
    fn check_config_and_print_default_config_parse() {
        let cli = Cli::try_parse_from(["tftpd-r", "check-config", "-c", "x.toml"]).expect("parse");
        assert!(
            matches!(cli.command, Command::CheckConfig(args) if args.config == std::path::Path::new("x.toml"))
        );

        let cli = Cli::try_parse_from(["tftpd-r", "print-default-config"]).expect("parse");
        assert!(matches!(cli.command, Command::PrintDefaultConfig));
    }

    #[test]
    fn invalid_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["tftpd-r", "run"]).is_err());
    }
}
