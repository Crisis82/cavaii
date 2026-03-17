use std::collections::HashSet;
use std::io::{self, BufRead, BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use cavaii_common::config::DaemonConfig;
use tracing::info;

const SPAWN_RETRY_INTERVAL: Duration = Duration::from_millis(1800);

pub struct OverlayProcess {
    child: Option<Child>,
    last_spawn_attempt: Option<Instant>,
}

impl OverlayProcess {
    pub fn new() -> Self {
        Self {
            child: None,
            last_spawn_attempt: None,
        }
    }

    pub fn ensure_running(&mut self, daemon: &DaemonConfig, now: Instant) -> io::Result<()> {
        if self.child.is_some() {
            return Ok(());
        }
        if self
            .last_spawn_attempt
            .is_some_and(|last| now.duration_since(last) < SPAWN_RETRY_INTERVAL)
        {
            return Ok(());
        }

        self.last_spawn_attempt = Some(now);
        let mut command = build_command(daemon);
        let mut child = command.spawn()?;
        if let Some(stderr) = child.stderr.take() {
            spawn_overlay_stderr_forwarder(stderr);
        }
        self.child = Some(child);
        info!("cavaii-daemon: started overlay process (cavaii)");
        Ok(())
    }

    pub fn poll_exit(&mut self) -> io::Result<Option<ExitStatus>> {
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };

        let maybe_exit = child.try_wait()?;
        if maybe_exit.is_some() {
            self.child = None;
        }

        Ok(maybe_exit)
    }

    pub fn stop(&mut self) -> io::Result<()> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };

        if let Err(err) = child.kill() {
            // Process might have just exited between poll and stop.
            if err.kind() != ErrorKind::InvalidInput {
                return Err(err);
            }
        }
        let _ = child.wait();
        info!("cavaii-daemon: stopped overlay process");
        Ok(())
    }
}

fn build_command(_daemon: &DaemonConfig) -> Command {
    let mut command = Command::new("cavaii");
    command.env("CAVAII_DISABLE_NOTIFICATIONS", "1");
    command.stdin(Stdio::null());
    command.stderr(Stdio::piped());
    command
}

fn spawn_overlay_stderr_forwarder(stderr: impl Read + Send + 'static) {
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line_result in reader.lines() {
            let Ok(line) = line_result else {
                break;
            };
            if should_suppress_gtk_warning(&line) {
                continue;
            }
            eprintln!("{line}");
        }
    });
}

fn should_suppress_gtk_warning(line: &str) -> bool {
    line.contains("Unknown key gtk-menu-images in ")
        || line.contains("Unknown key gtk-button-images in ")
}

pub fn any_allowed_process_running(allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }

    let allowed: HashSet<String> = allowed
        .iter()
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    if allowed.is_empty() {
        return true;
    }

    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return true,
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Ok(pid) = file_name.to_string_lossy().parse::<u32>() else {
            continue;
        };
        let proc_path = PathBuf::from("/proc").join(pid.to_string());
        if process_name_matches(&proc_path, &allowed) {
            return true;
        }
    }

    false
}

fn process_name_matches(proc_path: &Path, allowed: &HashSet<String>) -> bool {
    if let Ok(comm) = std::fs::read_to_string(proc_path.join("comm")) {
        let comm = comm.trim().to_ascii_lowercase();
        if allowed.contains(&comm) {
            return true;
        }
    }

    let Ok(cmdline) = std::fs::read(proc_path.join("cmdline")) else {
        return false;
    };
    let Some(first) = cmdline.split(|byte| *byte == 0).next() else {
        return false;
    };
    if first.is_empty() {
        return false;
    }
    let command = String::from_utf8_lossy(first);
    let name = Path::new(command.as_ref())
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if name.is_empty() {
        return false;
    }

    allowed.contains(&name)
}
