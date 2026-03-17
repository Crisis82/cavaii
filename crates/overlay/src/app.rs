use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, UNIX_EPOCH};

use cavaii_common::config::{self, AppConfig};
use cavaii_common::notify::notify_error_with_cooldown;
use gtk::glib;
use gtk::prelude::*;
use tracing::{info, warn};

const APP_ID: &str = "io.cavaii.overlay";
const CONFIG_POLL_INTERVAL: Duration = Duration::from_millis(180);
const CONFIG_RELOAD_DEBOUNCE: Duration = Duration::from_millis(260);

#[derive(Debug)]
pub enum AppError {
    Config(config::ConfigLoadError),
}

impl Display for AppError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(err) => write!(f, "could not load config: {err}"),
        }
    }
}

impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(err) => Some(err),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct ConfigStamp {
    exists: bool,
    modified_millis: u128,
    len: u64,
}

impl ConfigStamp {
    fn read(path: &Path) -> Self {
        let Ok(metadata) = std::fs::metadata(path) else {
            return Self {
                exists: false,
                modified_millis: 0,
                len: 0,
            };
        };

        let modified_millis = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_millis())
            .unwrap_or(0);

        Self {
            exists: true,
            modified_millis,
            len: metadata.len(),
        }
    }
}

#[derive(Clone)]
struct RunningOverlay {
    windows: Vec<gtk::ApplicationWindow>,
    runtime: RuntimeConfig,
    stream: Rc<cavaii_engine::live::LiveFrameStream>,
}

type OverlayState = Rc<std::cell::RefCell<Option<RunningOverlay>>>;

#[derive(Clone, Debug)]
struct PendingReload {
    stamp: ConfigFilesStamp,
    ready_at: std::time::Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConfigFilesStamp {
    config_path: PathBuf,
    colors_path: PathBuf,
    config: ConfigStamp,
    colors: ConfigStamp,
}

impl ConfigFilesStamp {
    fn read(config_path: &Path) -> Self {
        let (_, active_config_path, colors_path) = resolve_runtime_file_paths(config_path);
        Self {
            config_path: active_config_path.clone(),
            colors_path: colors_path.clone(),
            config: ConfigStamp::read(&active_config_path),
            colors: ConfigStamp::read(&colors_path),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RuntimeConfig {
    app_config: AppConfig,
    config_exists: bool,
    resolved_config_path: Option<PathBuf>,
    colors_path: PathBuf,
    colors_exists: bool,
}

pub fn run(config_path: PathBuf) -> Result<(), AppError> {
    let runtime = load_runtime_config(&config_path).map_err(AppError::Config)?;
    info!("cavaii starting");
    if runtime.config_exists {
        info!("config path: {} (found)", config_path.display());
    } else {
        info!(
            "config path: {} (not found, using built-in defaults)",
            config_path.display()
        );
    }
    if let Some(resolved_config_path) = runtime.resolved_config_path.as_ref()
        && resolved_config_path != &config_path
    {
        info!("resolved config path: {}", resolved_config_path.display());
    }
    if runtime.colors_exists {
        info!("colors path: {} (found)", runtime.colors_path.display());
    } else {
        info!(
            "colors path: {} (not found, using config/default colors)",
            runtime.colors_path.display()
        );
    }
    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| {
        let state = Rc::new(std::cell::RefCell::new(None));
        apply_config(app, &state, runtime.clone());

        let app_weak = app.downgrade();
        let config_path_for_reload = config_path.clone();
        let state_for_reload = Rc::clone(&state);
        let mut last_processed_stamp = ConfigFilesStamp::read(&config_path_for_reload);
        let mut pending_reload: Option<PendingReload> = None;

        glib::timeout_add_local(CONFIG_POLL_INTERVAL, move || {
            let Some(app) = app_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };

            let next_stamp = ConfigFilesStamp::read(&config_path_for_reload);
            let now = std::time::Instant::now();

            if next_stamp == last_processed_stamp {
                pending_reload = None;
                return glib::ControlFlow::Continue;
            }

            match pending_reload.as_mut() {
                Some(pending) if pending.stamp != next_stamp => {
                    pending.stamp = next_stamp.clone();
                    pending.ready_at = now + CONFIG_RELOAD_DEBOUNCE;
                }
                Some(_) => {}
                None => {
                    pending_reload = Some(PendingReload {
                        stamp: next_stamp.clone(),
                        ready_at: now + CONFIG_RELOAD_DEBOUNCE,
                    });
                }
            }

            let Some(pending) = pending_reload.as_ref() else {
                return glib::ControlFlow::Continue;
            };
            if now < pending.ready_at {
                return glib::ControlFlow::Continue;
            }

            let Some(pending) = pending_reload.take() else {
                return glib::ControlFlow::Continue;
            };
            last_processed_stamp = pending.stamp;

            match load_runtime_config(&config_path_for_reload) {
                Ok(next_runtime) => {
                    let should_apply = state_for_reload
                        .borrow()
                        .as_ref()
                        .map(|running| running.runtime != next_runtime)
                        .unwrap_or(true);
                    if should_apply {
                        info!("cavaii: config/colors changed, reloading overlay");
                        apply_config(&app, &state_for_reload, next_runtime);
                    }
                }
                Err(err) => {
                    warn!("cavaii: config reload failed (keeping current settings): {err}");
                    let (notify_enabled, notify_cooldown) = state_for_reload
                        .borrow()
                        .as_ref()
                        .map(|running| {
                            (
                                running.runtime.app_config.daemon.notify_on_error,
                                Duration::from_secs(
                                    running.runtime.app_config.daemon.notify_cooldown_seconds,
                                ),
                            )
                        })
                        .unwrap_or((true, Duration::from_secs(45)));
                    notify_error_with_cooldown(
                        "overlay.config_reload_failed",
                        "Cavaii Config Error",
                        &format!("Config reload failed: {err}"),
                        notify_enabled,
                        notify_cooldown,
                    );
                }
            }

            glib::ControlFlow::Continue
        });
    });

    let args = ["cavaii"];
    let _exit = app.run_with_args(&args);

    Ok(())
}

fn apply_config(app: &gtk::Application, state: &OverlayState, next_runtime: RuntimeConfig) {
    let next_stream = state
        .borrow()
        .as_ref()
        .filter(|running| {
            !audio_stream_config_changed(&running.runtime.app_config, &next_runtime.app_config)
        })
        .map(|running| Rc::clone(&running.stream))
        .unwrap_or_else(|| crate::ui::spawn_frame_stream(&next_runtime.app_config));
    let next_windows = crate::ui::build_overlay_windows(
        app,
        next_runtime.app_config.clone(),
        Rc::clone(&next_stream),
    );
    let previous = state.borrow_mut().replace(RunningOverlay {
        windows: next_windows,
        runtime: next_runtime,
        stream: next_stream,
    });

    if let Some(running) = previous {
        for window in running.windows {
            window.close();
        }
    }
}

fn audio_stream_config_changed(current: &AppConfig, next: &AppConfig) -> bool {
    current.visualizer.backend != next.visualizer.backend
        || current.visualizer.points != next.visualizer.points
        || current.visualizer.framerate != next.visualizer.framerate
}

fn load_runtime_config(config_path: &Path) -> Result<RuntimeConfig, config::ConfigLoadError> {
    let (resolved_config_path, config_load_path, colors_path) =
        resolve_runtime_file_paths(config_path);
    let config_exists = config_load_path.exists();
    let colors_exists = colors_path.exists();
    let mut config = config::load_or_default(&config_load_path)?;
    match config::load_color_overrides(&colors_path) {
        Ok(overrides) => config::apply_color_overrides(&mut config, overrides),
        Err(err) => {
            warn!("cavaii: colors override load failed (using default colors): {err}");
            notify_error_with_cooldown(
                "overlay.colors_load_failed",
                "Cavaii Colors Error",
                &format!("Could not load colors override: {err}"),
                config.daemon.notify_on_error,
                Duration::from_secs(config.daemon.notify_cooldown_seconds),
            );
        }
    }

    Ok(RuntimeConfig {
        app_config: config,
        resolved_config_path,
        config_exists,
        colors_path,
        colors_exists,
    })
}

fn resolve_runtime_config_path(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

fn resolve_runtime_file_paths(config_path: &Path) -> (Option<PathBuf>, PathBuf, PathBuf) {
    let resolved_config_path = resolve_runtime_config_path(config_path);
    let active_config_path = resolved_config_path
        .clone()
        .unwrap_or_else(|| config_path.to_path_buf());
    let colors_path = config::default_colors_path(&active_config_path);
    (resolved_config_path, active_config_path, colors_path)
}
