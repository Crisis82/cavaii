use std::process::{Command, Stdio};
use std::{env, path::PathBuf};

fn main() {
    let benchmark_exe = resolve_benchmark_exe();

    let status = Command::new(&benchmark_exe)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .unwrap_or_else(|err| panic!("failed to run {}: {err}", benchmark_exe.display()));

    assert!(
        status.success(),
        "perf benchmark exited with status {status}"
    );
}

fn resolve_benchmark_exe() -> PathBuf {
    if let Ok(path) = env::var("CARGO_BIN_EXE_perf-benchmark") {
        return PathBuf::from(path);
    }

    let current = env::current_exe()
        .unwrap_or_else(|err| panic!("cannot resolve current test executable path: {err}"));
    let release_dir = current
        .parent()
        .and_then(|value| value.parent())
        .unwrap_or_else(|| panic!("unexpected test executable path: {}", current.display()));

    let mut benchmark_exe = release_dir.join("perf-benchmark");
    if cfg!(windows) {
        benchmark_exe.set_extension("exe");
    }

    assert!(
        benchmark_exe.exists(),
        "benchmark binary not found at {}",
        benchmark_exe.display()
    );
    benchmark_exe
}
