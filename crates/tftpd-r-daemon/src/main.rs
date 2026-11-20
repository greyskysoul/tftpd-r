//! Headless TFTP daemon entry point.
//!
//! Run `tftpd-r --help` for usage. The daemon owns its Tokio runtime; the
//! core server does not create one.

mod cli;
mod config_file;
mod logging;
mod shutdown;

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use tftpd_r_core::events::EventSink;
use tftpd_r_core::server::ServerStatus;
use tftpd_r_core::TftpServer;
use tokio::sync::watch;
use tracing::{info, warn};

use cli::{CheckConfigArgs, Command, ServeArgs};

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    match cli.command {
        Command::PrintDefaultConfig => {
            print!("{}", config_file::DEFAULT_CONFIG_TOML);
            ExitCode::SUCCESS
        }
        Command::CheckConfig(args) => check_config(&args),
        Command::Serve(args) => serve(args),
    }
}

fn check_config(args: &CheckConfigArgs) -> ExitCode {
    match config_file::load(&args.config) {
        Ok(config) => {
            println!("configuration OK: {}", args.config.display());
            println!("  bind_address:            {}", config.bind_address);
            println!("  root_dir:                {}", config.display_root());
            println!("  mode:                    read-only (uploads disabled)");
            println!("  block_size (max):        {}", config.block_size);
            println!(
                "  timeout / retries:       {}s / {}",
                config.timeout_secs, config.max_retries
            );
            println!("  max sessions:            {}", config.max_sessions);
            println!("  max file bytes:          {}", config.max_file_bytes);
            println!(
                "  max transfer duration:   {}s",
                config.max_transfer_duration_secs
            );
            println!("  shutdown grace:          {}s", config.shutdown_grace_secs);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            if let Some(source) = std::error::Error::source(&error) {
                eprintln!("caused by: {source}");
            }
            ExitCode::from(2)
        }
    }
}

fn serve(args: ServeArgs) -> ExitCode {
    logging::init(args.log.as_deref());
    match run_serve(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_serve(args: ServeArgs) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not create the Tokio runtime")?;
    runtime.block_on(serve_async(args))
}

async fn serve_async(args: ServeArgs) -> Result<()> {
    let mut config = config_file::load(&args.config).context("could not load configuration")?;
    config_file::apply_overrides(&mut config, &args).context("invalid command line override")?;
    let bind = config.bind_address;
    let root = config.display_root();

    let events: Arc<dyn EventSink> = Arc::new(logging::TracingSink);
    info!(config = %args.config.display(), %bind, %root, "starting TFTP server");
    let server = TftpServer::start(config, events)
        .await
        .context("could not start the TFTP server")?;
    info!(local = %server.local_addr(), "server running; press Ctrl-C to stop");

    let mut status = server.status_receiver();
    let outcome = tokio::select! {
        signal = shutdown::wait_for_shutdown_signal() => Ok(signal),
        failure = wait_for_failure(&mut status) => Err(failure),
    };

    match &outcome {
        Ok(signal) => info!(signal, "shutdown requested"),
        Err(error) => warn!(%error, "server stopped unexpectedly"),
    }
    server.stop().await;
    info!("TFTP server stopped");
    outcome.map(|_| ())
}

async fn wait_for_failure(status: &mut watch::Receiver<ServerStatus>) -> anyhow::Error {
    loop {
        if status.changed().await.is_err() {
            return anyhow!("server status channel closed unexpectedly");
        }
        match status.borrow().clone() {
            ServerStatus::Failed { message } => return anyhow!("{message}"),
            ServerStatus::Stopped => return anyhow!("server stopped unexpectedly"),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_for_failure_reports_channel_closure() {
        let (tx, mut rx) = watch::channel(ServerStatus::Running {
            local_addr: "127.0.0.1:1".parse().expect("addr"),
        });
        drop(tx);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let error = runtime.block_on(wait_for_failure(&mut rx));
        assert!(error.to_string().contains("closed"));
    }
}
