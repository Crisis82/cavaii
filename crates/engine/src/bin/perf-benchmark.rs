use std::fs;
use std::hint::black_box;
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_PARSE_ITERATIONS: u64 = 3_000_000;
const DEFAULT_DUMMY_ITERATIONS: u64 = 900_000;
const DEFAULT_BAR_COUNT: usize = 120;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
const CAVA_RAW_U16_MAX: f32 = u16::MAX as f32;

#[derive(Debug, Clone, Copy)]
struct BenchmarkConfig {
    parse_iterations: u64,
    dummy_iterations: u64,
    bar_count: usize,
}

impl BenchmarkConfig {
    fn from_env() -> Self {
        Self {
            parse_iterations: read_env_u64("CAVAII_BENCH_PARSE_ITERS", DEFAULT_PARSE_ITERATIONS),
            dummy_iterations: read_env_u64("CAVAII_BENCH_DUMMY_ITERS", DEFAULT_DUMMY_ITERATIONS),
            bar_count: read_env_usize("CAVAII_BENCH_BAR_COUNT", DEFAULT_BAR_COUNT).max(1),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ProcSnapshot {
    proc_jiffies: u64,
    total_jiffies: u64,
    rss_kb: u64,
    io_read_bytes: u64,
    io_write_bytes: u64,
    ctx_switch_voluntary: u64,
    ctx_switch_involuntary: u64,
}

impl ProcSnapshot {
    fn capture() -> Option<Self> {
        let proc_jiffies = read_proc_jiffies()?;
        let total_jiffies = read_total_cpu_jiffies()?;
        let status = fs::read_to_string("/proc/self/status").ok()?;
        let io = fs::read_to_string("/proc/self/io").ok()?;
        Some(Self {
            proc_jiffies,
            total_jiffies,
            rss_kb: parse_status_value(&status, "VmRSS:").unwrap_or(0),
            io_read_bytes: parse_io_value(&io, "read_bytes:").unwrap_or(0),
            io_write_bytes: parse_io_value(&io, "write_bytes:").unwrap_or(0),
            ctx_switch_voluntary: parse_status_value(&status, "voluntary_ctxt_switches:")
                .unwrap_or(0),
            ctx_switch_involuntary: parse_status_value(&status, "nonvoluntary_ctxt_switches:")
                .unwrap_or(0),
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct MetricStats {
    count: u64,
    min: f64,
    max: f64,
    sum: f64,
}

impl MetricStats {
    fn add(&mut self, value: f64) {
        if !value.is_finite() {
            return;
        }
        if self.count == 0 {
            self.min = value;
            self.max = value;
        } else {
            if value < self.min {
                self.min = value;
            }
            if value > self.max {
                self.max = value;
            }
        }
        self.count = self.count.saturating_add(1);
        self.sum += value;
    }

    fn avg(self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some(self.sum / self.count as f64)
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct SampleMetrics {
    sample_count: u64,
    cpu_percent: MetricStats,
    rss_kb: MetricStats,
    gpu_util_percent: MetricStats,
    gpu_mem_mib: MetricStats,
    gpu_available: bool,
}

fn main() {
    let config = BenchmarkConfig::from_env();
    let cpu_cores = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
        .max(1);

    println!("=== Cavaii Rust Performance Benchmark ===");
    println!(
        "workload: parse_iters={}, dummy_iters={}, bar_count={}",
        config.parse_iterations, config.dummy_iterations, config.bar_count
    );
    println!("sampling: every {} ms", SAMPLE_INTERVAL.as_millis().max(1));

    let before = ProcSnapshot::capture();
    let metrics = Arc::new(Mutex::new(SampleMetrics::default()));
    let running = Arc::new(AtomicBool::new(true));
    let sampler = spawn_sampler(
        Arc::clone(&running),
        Arc::clone(&metrics),
        SAMPLE_INTERVAL,
        cpu_cores,
    );

    let benchmark_start = Instant::now();
    let parse_duration = run_parse_workload(config.parse_iterations, config.bar_count);
    let dummy_duration = run_dummy_workload(config.dummy_iterations, config.bar_count);
    let total_duration = benchmark_start.elapsed();

    running.store(false, Ordering::Relaxed);
    let _ = sampler.join();

    let after = ProcSnapshot::capture();
    let metrics = {
        let guard = match metrics.lock() {
            Ok(value) => value,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard
    };

    println!("\nWorkload timing:");
    print_timing_line("parse_u16_frame", config.parse_iterations, parse_duration);
    print_timing_line("dummy_wave_gen", config.dummy_iterations, dummy_duration);
    println!("  total: {:.3}s", total_duration.as_secs_f64());

    println!(
        "\nUsage samples ({} samples):",
        metrics.sample_count.max(metrics.cpu_percent.count)
    );
    print_metric("CPU usage (% of one core)", metrics.cpu_percent, "%");
    print_metric("RSS memory", metrics.rss_kb, " KB");
    if metrics.gpu_available {
        print_metric("GPU usage", metrics.gpu_util_percent, "%");
        print_metric("GPU memory", metrics.gpu_mem_mib, " MiB");
    } else {
        println!("  GPU usage: n/a (nvidia-smi unavailable or no readable data)");
    }

    println!("\nProcess deltas:");
    if let (Some(start), Some(end)) = (before, after) {
        println!(
            "  read_bytes: {}",
            end.io_read_bytes.saturating_sub(start.io_read_bytes)
        );
        println!(
            "  write_bytes: {}",
            end.io_write_bytes.saturating_sub(start.io_write_bytes)
        );
        println!(
            "  voluntary_ctx_switches: {}",
            end.ctx_switch_voluntary
                .saturating_sub(start.ctx_switch_voluntary)
        );
        println!(
            "  involuntary_ctx_switches: {}",
            end.ctx_switch_involuntary
                .saturating_sub(start.ctx_switch_involuntary)
        );
    } else {
        println!("  n/a (could not read /proc snapshots)");
    }
}

fn spawn_sampler(
    running: Arc<AtomicBool>,
    metrics: Arc<Mutex<SampleMetrics>>,
    interval: Duration,
    cpu_cores: usize,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut previous = ProcSnapshot::capture();
        let mut gpu_enabled = true;

        while running.load(Ordering::Relaxed) {
            thread::sleep(interval);
            let current = ProcSnapshot::capture();
            let Some(current) = current else {
                continue;
            };

            let mut guard = match metrics.lock() {
                Ok(value) => value,
                Err(poisoned) => poisoned.into_inner(),
            };

            guard.sample_count = guard.sample_count.saturating_add(1);
            guard.rss_kb.add(current.rss_kb as f64);

            if let Some(prev) = previous {
                let delta_proc = current.proc_jiffies.saturating_sub(prev.proc_jiffies);
                let delta_total = current.total_jiffies.saturating_sub(prev.total_jiffies);
                if delta_total > 0 {
                    let usage =
                        (delta_proc as f64 / delta_total as f64) * (cpu_cores as f64 * 100.0);
                    guard.cpu_percent.add(usage);
                }
            }

            if gpu_enabled {
                match sample_gpu_usage() {
                    Some((gpu_usage, gpu_mem)) => {
                        guard.gpu_available = true;
                        guard.gpu_util_percent.add(gpu_usage);
                        guard.gpu_mem_mib.add(gpu_mem);
                    }
                    None => {
                        gpu_enabled = false;
                    }
                }
            }

            previous = Some(current);
        }
    })
}

fn run_parse_workload(iterations: u64, bar_count: usize) -> Duration {
    let frame = vec![0xFF_u8; bar_count * 2];
    let mut parsed = vec![0.0_f32; bar_count];
    let start = Instant::now();
    for _ in 0..iterations {
        let ok = parse_cava_raw_frame_into(black_box(&frame), black_box(&mut parsed));
        black_box(ok);
        black_box(parsed[0]);
    }
    black_box(&parsed);
    start.elapsed()
}

fn run_dummy_workload(iterations: u64, bar_count: usize) -> Duration {
    let mut phase = 0.0_f32;
    let mut checksum = 0.0_f32;
    let spread = 0.35_f32;
    let start = Instant::now();
    for _ in 0..iterations {
        for index in 0..bar_count {
            let position = (index as f32 * spread) + phase;
            let value = (position.sin() * 0.5) + 0.5;
            checksum += value;
        }
        phase += 0.2;
    }
    black_box(checksum);
    black_box(phase);
    start.elapsed()
}

fn parse_cava_raw_frame_into(frame: &[u8], output: &mut [f32]) -> bool {
    if output.is_empty() || frame.len() < output.len() * 2 {
        return false;
    }

    for (index, chunk) in frame.chunks_exact(2).take(output.len()).enumerate() {
        let raw = u16::from_le_bytes([chunk[0], chunk[1]]);
        output[index] = (raw as f32 / CAVA_RAW_U16_MAX).clamp(0.0, 1.0);
    }
    true
}

fn sample_gpu_usage() -> Option<(f64, f64)> {
    let output = Command::new("nvidia-smi")
        .arg("--query-gpu=utilization.gpu,memory.used")
        .arg("--format=csv,noheader,nounits")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut count = 0_u64;
    let mut usage_sum = 0.0_f64;
    let mut mem_sum = 0.0_f64;
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let mut parts = line.split(',').map(str::trim);
        let usage = parts.next().and_then(|value| value.parse::<f64>().ok());
        let mem = parts.next().and_then(|value| value.parse::<f64>().ok());
        let (Some(usage), Some(mem)) = (usage, mem) else {
            continue;
        };
        usage_sum += usage;
        mem_sum += mem;
        count = count.saturating_add(1);
    }
    if count == 0 {
        None
    } else {
        Some((usage_sum / count as f64, mem_sum / count as f64))
    }
}

fn read_proc_jiffies() -> Option<u64> {
    let raw = fs::read_to_string("/proc/self/stat").ok()?;
    let right_paren = raw.rfind(')')?;
    let rest = raw.get(right_paren + 2..)?;
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    Some(utime.saturating_add(stime))
}

fn read_total_cpu_jiffies() -> Option<u64> {
    let raw = fs::read_to_string("/proc/stat").ok()?;
    let first_line = raw.lines().next()?;
    if !first_line.starts_with("cpu ") {
        return None;
    }
    let mut total = 0_u64;
    for value in first_line.split_whitespace().skip(1) {
        total = total.saturating_add(value.parse::<u64>().ok()?);
    }
    Some(total)
}

fn parse_status_value(content: &str, key: &str) -> Option<u64> {
    let line = content
        .lines()
        .find(|line| line.starts_with(key))
        .map(str::trim)?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok()
}

fn parse_io_value(content: &str, key: &str) -> Option<u64> {
    let line = content
        .lines()
        .find(|line| line.starts_with(key))
        .map(str::trim)?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok()
}

fn read_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn read_env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn print_timing_line(name: &str, iterations: u64, duration: Duration) {
    let seconds = duration.as_secs_f64().max(1e-9);
    let us_per_iter = seconds * 1_000_000.0 / iterations.max(1) as f64;
    let iter_per_sec = iterations as f64 / seconds;
    println!(
        "  {name}: {iterations} iterations in {:.3}s ({us_per_iter:.3} us/iter, {iter_per_sec:.0} iter/s)",
        seconds
    );
}

fn print_metric(label: &str, stats: MetricStats, unit: &str) {
    if let Some(avg) = stats.avg() {
        println!(
            "  {label}: min {:.2}{unit}, avg {:.2}{unit}, max {:.2}{unit}",
            stats.min, avg, stats.max
        );
    } else {
        println!("  {label}: n/a");
    }
}
