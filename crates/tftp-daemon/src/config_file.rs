//! Loading and validating the TOML configuration file.
//!
//! The example file in the repository root is embedded at compile time and is
//! what `print-default-config` prints, so documentation and defaults cannot
//! drift apart.

use std::fs;
use std::path::{Path, PathBuf};

use tftp_core::{ConfigError, ServerConfig};

use crate::cli::ServeArgs;

/// Annotated, safe default configuration (identical to `config.example.toml`).
pub const DEFAULT_CONFIG_TOML: &str = include_str!("../../../config.example.toml");

/// Errors while reading or validating a configuration file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigLoadError {
    /// The file could not be read.
    #[error("could not read configuration file {path}: {source}")]
    Read {
        /// Path that was attempted.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The file exists but is not a valid configuration.
    #[error("configuration file {path} is invalid: {source}")]
    Invalid {
        /// Path of the offending file.
        path: PathBuf,
        /// Validation error.
        source: ConfigError,
    },
}

/// Read, parse and validate a configuration file.
///
/// # Errors
///
/// Returns [`ConfigLoadError`] for I/O, TOML or validation failures.
pub fn load(path: &Path) -> Result<ServerConfig, ConfigLoadError> {
    let text = fs::read_to_string(path).map_err(|source| ConfigLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    ServerConfig::from_toml_str(&text).map_err(|source| ConfigLoadError::Invalid {
        path: path.to_path_buf(),
        source,
    })
}

/// Apply `--bind`/`--root` command line overrides and re-validate.
///
/// # Errors
///
/// Returns [`ConfigLoadError::Invalid`] when an override makes the configuration
/// invalid (for example a root directory that does not exist).
pub fn apply_overrides(config: &mut ServerConfig, args: &ServeArgs) -> Result<(), ConfigLoadError> {
    if let Some(bind) = args.bind {
        config.bind_address = bind;
    }
    if let Some(root) = &args.root {
        config.root_dir = root.clone();
    }
    config
        .validate()
        .map_err(|source| ConfigLoadError::Invalid {
            path: args.config.clone(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn serve_args(config: PathBuf) -> ServeArgs {
        ServeArgs {
            config,
            bind: None,
            root: None,
            log: None,
        }
    }

    #[test]
    fn embedded_default_matches_example_file() {
        let on_disk = include_str!("../../../config.example.toml");
        assert_eq!(DEFAULT_CONFIG_TOML, on_disk);
        assert!(DEFAULT_CONFIG_TOML.contains("allow_upload = false"));
        assert!(DEFAULT_CONFIG_TOML.contains("bind_address = \"127.0.0.1:6969\""));
    }

    #[test]
    fn default_template_parses_with_existing_root() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::create_dir(dir.path().join("tftp-root")).expect("root");
        let text = DEFAULT_CONFIG_TOML.replace(
            "root_dir = \"tftp-root\"",
            &format!(
                "root_dir = {:?}",
                dir.path().join("tftp-root").to_string_lossy()
            ),
        );
        let parsed =
            ServerConfig::from_toml_str(&text).expect("default configuration must be valid");
        assert!(!parsed.allow_upload);
        assert_eq!(parsed.bind_address.port(), 6969);
    }

    #[test]
    fn load_reports_missing_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let error = load(&dir.path().join("absent.toml")).expect_err("must fail");
        assert!(matches!(error, ConfigLoadError::Read { .. }));
    }

    #[test]
    fn load_reports_unknown_fields_with_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        fs::write(&path, "root_dir = \"x\"\nsurprise = true\n").expect("write");
        let error = load(&path).expect_err("must fail");
        let text = error.to_string();
        assert!(
            text.contains("config.toml"),
            "message should name the file: {text}"
        );
        assert!(matches!(error, ConfigLoadError::Invalid { .. }));
    }

    #[test]
    fn overrides_are_validated() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("root");
        fs::create_dir(&root).expect("root");
        let mut config = ServerConfig {
            root_dir: root.clone(),
            ..ServerConfig::default()
        };
        let mut args = serve_args(dir.path().join("config.toml"));
        args.bind = Some("127.0.0.1:0".parse().expect("addr"));
        apply_overrides(&mut config, &args).expect("valid overrides");
        assert_eq!(config.bind_address, "127.0.0.1:0".parse().expect("addr"));

        args.root = Some(dir.path().join("missing"));
        assert!(apply_overrides(&mut config, &args).is_err());
    }
}
