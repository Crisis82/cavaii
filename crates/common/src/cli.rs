use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CliOptions {
    pub config_path: Option<PathBuf>,
    pub show_help: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliArgError {
    message: String,
}

impl CliArgError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

pub fn parse_standard_cli() -> Result<CliOptions, CliArgError> {
    parse_from_iter(std::env::args_os().skip(1))
}

pub fn usage(binary_name: &str) -> String {
    format!(
        "Usage: {binary_name} [--config <path>]\n       {binary_name} [--help]\n\nOptions:\n  -c, --config <path>  Load config.toml from a specific path\n  -h, --help           Show this help message"
    )
}

fn parse_from_iter(args: impl IntoIterator<Item = OsString>) -> Result<CliOptions, CliArgError> {
    let mut options = CliOptions::default();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "-h" | "--help" => {
                options.show_help = true;
            }
            "-c" | "--config" => {
                let Some(value) = args.next() else {
                    return Err(CliArgError::new("missing value for --config"));
                };
                options.config_path = Some(PathBuf::from(value));
            }
            value if value.starts_with("--config=") => {
                let path = &value["--config=".len()..];
                if path.is_empty() {
                    return Err(CliArgError::new("missing value for --config"));
                }
                options.config_path = Some(PathBuf::from(path));
            }
            other => {
                return Err(CliArgError::new(format!("unknown argument: {other}")));
            }
        }
    }

    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::{CliOptions, parse_from_iter};
    use std::path::PathBuf;

    #[test]
    fn parses_long_config_flag() {
        let parsed = parse_from_iter(["--config".into(), "/tmp/custom.toml".into()]);
        assert_eq!(
            parsed,
            Ok(CliOptions {
                config_path: Some(PathBuf::from("/tmp/custom.toml")),
                show_help: false,
            })
        );
    }

    #[test]
    fn parses_equals_config_flag() {
        let parsed = parse_from_iter(["--config=/tmp/custom.toml".into()]);
        assert_eq!(
            parsed,
            Ok(CliOptions {
                config_path: Some(PathBuf::from("/tmp/custom.toml")),
                show_help: false,
            })
        );
    }

    #[test]
    fn parses_short_help_flag() {
        let parsed = parse_from_iter(["-h".into()]);
        assert_eq!(
            parsed,
            Ok(CliOptions {
                config_path: None,
                show_help: true,
            })
        );
    }

    #[test]
    fn rejects_unknown_argument() {
        let parsed = parse_from_iter(["--wat".into()]);
        assert!(parsed.is_err());
    }
}
