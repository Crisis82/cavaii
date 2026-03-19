mod activity;
mod process;

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use activity::{ActivityState, ActivityTracker};
use cavaii_common::config::{self, DaemonConfig};
use cavaii_common::notify::notify_error_with_cooldown;
use process::OverlayProcess;
use tracing::{error, info, warn};

const CONFIG_RELOAD_DEBOUNCE: Duration = Duration::from_millis(260);

#[derive(Debug)]
pub enum DaemonError {
    Config(config::ConfigLoadError),
    Runtime(std::io::Error),
}

impl Display for DaemonError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(err) => write!(f, "failed to load config: {err}"),
            Self::Runtime(err) => write!(f, "runtime error: {err}"),
        }
    }
}

impl Error for DaemonError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(err) => Some(err),
            Self::Runtime(err) => Some(err),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct RuntimeConfig {
    daemon: DaemonConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct ConfigStamp {
    path: PathBuf,
    exists: bool,
    modified_millis: u128,
    len: u64,
}

impl ConfigStamp {
    fn read(path: &Path) -> Self {
        let Ok(metadata) = std::fs::metadata(path) else {
            return Self {
                path: path.to_path_buf(),
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
            path: path.to_path_buf(),
            exists: true,
            modified_millis,
            len: metadata.len(),
        }
    }
}

#[derive(Clone, Debug)]
struct PendingConfigReload {
    stamp: ConfigStamp,
    ready_at: Instant,
}

pub fn run(config_path: PathBuf) -> Result<(), DaemonError> {
    let mut active_config_path = resolved_config_path_or_input(&config_path);
    let mut runtime = load_runtime_config(&active_config_path).map_err(DaemonError::Config)?;

    info!("cavaii-daemon starting");
    if config_path.exists() {
        info!("config path: {} (found)", config_path.display());
    } else {
        info!(
            "config path: {} (not found, using built-in defaults)",
            config_path.display()
        );
    }

    let mut config_stamp = ConfigStamp::read(&active_config_path);
    let mut pending_config_reload: Option<PendingConfigReload> = None;
    let mut inactivity_grace_until: Option<Instant> = None;

    let mut activity = ActivityTracker::new();
    let mut overlay = OverlayProcess::new();

    loop {
        thread::sleep(Duration::from_millis(
            runtime.daemon.poll_interval_ms.max(16),
        ));
        let now = Instant::now();

        if let Some(exit_status) = overlay.poll_exit().map_err(DaemonError::Runtime)? {
            warn!("cavaii-daemon: overlay exited with status {exit_status}");
            notify_error_with_cooldown(
                "daemon.overlay_exited",
                "Cavaii Overlay Exited",
                &format!("Overlay process exited: {exit_status}"),
                runtime.daemon.notify_on_error,
                notify_cooldown(&runtime.daemon),
            );
        }

        let next_config_path = resolved_config_path_or_input(&config_path);
        let next_config_stamp = ConfigStamp::read(&next_config_path);
        if next_config_stamp == config_stamp {
            pending_config_reload = None;
        } else {
            match pending_config_reload.as_mut() {
                Some(pending) if pending.stamp != next_config_stamp => {
                    pending.stamp = next_config_stamp.clone();
                    pending.ready_at = now + CONFIG_RELOAD_DEBOUNCE;
                }
                Some(_) => {}
                None => {
                    pending_config_reload = Some(PendingConfigReload {
                        stamp: next_config_stamp.clone(),
                        ready_at: now + CONFIG_RELOAD_DEBOUNCE,
                    });
                }
            }
        }

        if pending_config_reload
            .as_ref()
            .is_some_and(|pending| now >= pending.ready_at)
        {
            let Some(pending) = pending_config_reload.take() else {
                continue;
            };
            config_stamp = pending.stamp;
            active_config_path = config_stamp.path.clone();
            match load_runtime_config(&active_config_path) {
                Ok(next_runtime) => {
                    if runtime != next_runtime {
                        info!("cavaii-daemon: config changed, reloading daemon settings");
                        inactivity_grace_until = extend_inactivity_grace(
                            inactivity_grace_until,
                            activity.state(),
                            now,
                            config_switch_grace_duration(&runtime.daemon, &next_runtime.daemon),
                        );
                        runtime = next_runtime;
                    }
                }
                Err(err) => {
                    warn!("cavaii-daemon: config reload failed (keeping current settings): {err}");
                    notify_error_with_cooldown(
                        "daemon.config_reload_failed",
                        "Cavaii Config Error",
                        &format!("Config reload failed: {err}"),
                        runtime.daemon.notify_on_error,
                        notify_cooldown(&runtime.daemon),
                    );
                }
            }
        }

        let process_allowed =
            process::any_allowed_process_running(&runtime.daemon.allowed_processes);
        if !process_allowed {
            inactivity_grace_until = None;
            overlay.stop().map_err(DaemonError::Runtime)?;
        }

        let playback_active = process::any_audio_playback_running();
        let mut instantaneous_active = process_allowed && playback_active;
        if !instantaneous_active
            && activity.state() == ActivityState::Active
            && inactivity_grace_until.is_some_and(|until| now < until)
        {
            instantaneous_active = true;
        }
        if inactivity_grace_until.is_some_and(|until| now >= until) {
            inactivity_grace_until = None;
        }
        let state_changed = activity.update(
            now,
            instantaneous_active,
            Duration::from_millis(runtime.daemon.activate_delay_ms),
            Duration::from_millis(runtime.daemon.deactivate_delay_ms),
        );

        if state_changed {
            match activity.state() {
                ActivityState::Active => info!("cavaii-daemon: audio active"),
                ActivityState::Inactive => info!("cavaii-daemon: audio inactive"),
            }
        }

        match activity.state() {
            ActivityState::Active => {
                if let Err(err) = overlay.ensure_running(&runtime.daemon, now) {
                    error!("cavaii-daemon: could not launch overlay: {err}");
                    notify_error_with_cooldown(
                        "daemon.overlay_launch_failed",
                        "Cavaii Overlay Start Failed",
                        &format!("Could not launch overlay: {err}"),
                        runtime.daemon.notify_on_error,
                        notify_cooldown(&runtime.daemon),
                    );
                }
            }
            ActivityState::Inactive => {
                if runtime.daemon.stop_on_silence {
                    overlay.stop().map_err(DaemonError::Runtime)?;
                }
            }
        }
    }
}

fn load_runtime_config(config_path: &Path) -> Result<RuntimeConfig, config::ConfigLoadError> {
    let app_config = config::load_or_default(config_path)?;
    Ok(RuntimeConfig {
        daemon: app_config.daemon,
    })
}

fn notify_cooldown(config: &DaemonConfig) -> Duration {
    Duration::from_secs(config.notify_cooldown_seconds)
}

fn config_switch_grace_duration(current: &DaemonConfig, next: &DaemonConfig) -> Duration {
    let millis = current
        .deactivate_delay_ms
        .max(next.deactivate_delay_ms)
        .max(2500);
    Duration::from_millis(millis)
}

fn extend_inactivity_grace(
    current_until: Option<Instant>,
    activity_state: ActivityState,
    now: Instant,
    duration: Duration,
) -> Option<Instant> {
    if activity_state != ActivityState::Active {
        return current_until;
    }

    let next_until = now + duration;
    match current_until {
        Some(existing) if existing > next_until => Some(existing),
        _ => Some(next_until),
    }
}

fn resolve_runtime_config_path(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

fn resolved_config_path_or_input(path: &Path) -> PathBuf {
    resolve_runtime_config_path(path).unwrap_or_else(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use cavaii_common::config::DaemonConfig;

    use super::{
        ActivityState, CONFIG_RELOAD_DEBOUNCE, ConfigStamp, PendingConfigReload,
        config_switch_grace_duration, extend_inactivity_grace,
    };

    #[test]
    fn config_switch_grace_has_minimum_duration() {
        let current = DaemonConfig {
            deactivate_delay_ms: 1200,
            ..DaemonConfig::default()
        };
        let next = DaemonConfig {
            deactivate_delay_ms: 1800,
            ..DaemonConfig::default()
        };

        assert_eq!(
            config_switch_grace_duration(&current, &next),
            Duration::from_millis(2500)
        );
    }

    #[test]
    fn extend_inactivity_grace_only_when_active() {
        let now = Instant::now();
        let duration = Duration::from_secs(3);

        assert_eq!(
            extend_inactivity_grace(None, ActivityState::Inactive, now, duration),
            None
        );

        let active_until = extend_inactivity_grace(None, ActivityState::Active, now, duration);
        assert!(active_until.is_some_and(|until| until >= now + duration));
    }

    #[test]
    fn debounce_keeps_latest_ready_at_when_stamp_changes() {
        let first = ConfigStamp {
            path: PathBuf::from("/tmp/first.toml"),
            exists: true,
            modified_millis: 1,
            len: 10,
        };
        let second = ConfigStamp {
            path: PathBuf::from("/tmp/second.toml"),
            exists: true,
            modified_millis: 2,
            len: 10,
        };
        let start = Instant::now();
        let mut pending = PendingConfigReload {
            stamp: first,
            ready_at: start + Duration::from_millis(100),
        };

        if pending.stamp != second {
            pending.stamp = second.clone();
            pending.ready_at = start + CONFIG_RELOAD_DEBOUNCE;
        }

        assert_eq!(pending.stamp, second);
        assert!(pending.ready_at >= start + CONFIG_RELOAD_DEBOUNCE);
    }
}
