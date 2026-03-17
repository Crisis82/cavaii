mod app;
mod ui;

use std::time::Duration;

use cavaii_common::config::{self, DaemonConfig};
use cavaii_common::notify::notify_error_with_cooldown;

fn main() {
    let config_path = match config::ensure_default_config_files() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("cavaii: failed to initialize config files: {err}");
            std::process::exit(1);
        }
    };

    let logging_enabled = config::load_or_default(&config_path)
        .map(|value| value.logging)
        .unwrap_or(true);

    if logging_enabled && let Err(err) = cavaii_common::logging::init_logging("cavaii") {
        eprintln!("cavaii logging init failed: {err}");
    }

    if let Err(err) = app::run(config_path) {
        tracing::error!("cavaii failed: {err}");
        let defaults = DaemonConfig::default();
        notify_error_with_cooldown(
            "overlay.fatal",
            "Cavaii Overlay Error",
            &format!("{err}"),
            defaults.notify_on_error,
            Duration::from_secs(defaults.notify_cooldown_seconds),
        );
        std::process::exit(1);
    }
}
