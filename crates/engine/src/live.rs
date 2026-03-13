use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cavaii_common::config::{VisualizerBackend, VisualizerConfig};
use cavaii_common::spectrum::SpectrumFrame;
use tracing::{error, warn};

use crate::pipeline::{DummySineSource, FrameSource};

const CAVA_ATTACK: f32 = 0.8;
const CAVA_DECAY: f32 = 0.84;
const PIPEWIRE_RATE: u32 = 44_100;

#[derive(Debug, Clone, Copy)]
struct PipewireTuning {
    attack: f32,
    decay: f32,
    gain: f32,
    curve: f32,
    neighbor_mix: f32,
}

#[derive(Debug)]
struct PipewireBarsScratch {
    bar_count: usize,
    bin_energy: Vec<f32>,
    bin_count: Vec<u32>,
    bars: Vec<f32>,
}

impl PipewireBarsScratch {
    fn new(bar_count: usize) -> Self {
        Self {
            bar_count,
            bin_energy: vec![0.0_f32; bar_count],
            bin_count: vec![0_u32; bar_count],
            bars: vec![0.0_f32; bar_count],
        }
    }

    fn compute(&mut self, bytes: &[u8], channels: usize, tuning: PipewireTuning) -> &[f32] {
        if self.bar_count == 0 {
            return &self.bars;
        }

        let channels = channels.max(1);
        let bytes_per_frame = channels * std::mem::size_of::<f32>();
        if bytes.len() < bytes_per_frame {
            self.bars.fill(0.0);
            return &self.bars;
        }

        let frame_count = bytes.len() / bytes_per_frame;
        if frame_count == 0 {
            self.bars.fill(0.0);
            return &self.bars;
        }

        self.bin_energy.fill(0.0);
        self.bin_count.fill(0);

        for frame_idx in 0..frame_count {
            let frame_base = frame_idx * bytes_per_frame;
            let mut sample_sq_sum = 0.0_f32;
            let mut channel_count = 0_u32;

            for channel in 0..channels {
                let sample_offset = frame_base + (channel * std::mem::size_of::<f32>());
                let sample = f32::from_le_bytes([
                    bytes[sample_offset],
                    bytes[sample_offset + 1],
                    bytes[sample_offset + 2],
                    bytes[sample_offset + 3],
                ]);
                if sample.is_finite() {
                    sample_sq_sum += sample * sample;
                    channel_count += 1;
                }
            }

            if channel_count == 0 {
                continue;
            }

            let amplitude_rms = (sample_sq_sum / channel_count as f32).sqrt();
            let bin = frame_idx * self.bar_count / frame_count;
            self.bin_energy[bin] += amplitude_rms * amplitude_rms;
            self.bin_count[bin] += 1;
        }

        self.bars.fill(0.0);
        for (idx, value) in self.bars.iter_mut().enumerate() {
            let count = self.bin_count[idx];
            if count > 0 {
                *value = (self.bin_energy[idx] / count as f32).sqrt();
            }
        }

        // Neighbor blend smoothes sharp isolated spikes that feel too aggressive.
        if self.bar_count > 1 && tuning.neighbor_mix > 0.0 {
            let center_weight = (1.0 - (2.0 * tuning.neighbor_mix)).max(0.05);

            // Use in-place blending: process right-to-left to avoid overwriting needed values
            let last_idx = self.bar_count - 1;
            let mut prev_original = self.bars[last_idx];

            for idx in (1..last_idx).rev() {
                let current = self.bars[idx];
                let left = self.bars[idx - 1];
                let right = prev_original;

                prev_original = current;
                self.bars[idx] = (current * center_weight
                    + left * tuning.neighbor_mix
                    + right * tuning.neighbor_mix)
                    / (center_weight + 2.0 * tuning.neighbor_mix);
            }

            // Handle first element (no left neighbor)
            let current = self.bars[0];
            let right = prev_original;
            self.bars[0] =
                (current * center_weight + right * tuning.neighbor_mix)
                    / (center_weight + tuning.neighbor_mix);

            // Handle last element (no right neighbor)
            let current = self.bars[last_idx];
            let left = self.bars[last_idx - 1];
            self.bars[last_idx] =
                (current * center_weight + left * tuning.neighbor_mix)
                    / (center_weight + tuning.neighbor_mix);
        }

        for value in &mut self.bars {
            let boosted = *value * tuning.gain;
            *value = boosted.powf(tuning.curve).clamp(0.0, 1.0);
        }

        &self.bars
    }
}

impl PipewireTuning {
    fn from_config(config: &VisualizerConfig) -> Self {
        Self {
            attack: config.pipewire_attack.clamp(0.01, 1.0),
            decay: config.pipewire_decay.clamp(0.5, 0.9995),
            gain: config.pipewire_gain.clamp(0.1, 6.0),
            curve: config.pipewire_curve.clamp(0.4, 2.5),
            neighbor_mix: config.pipewire_neighbor_mix.clamp(0.0, 0.45),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Pipewire,
    Cava,
    Dummy,
}

pub struct LiveFrameStream {
    latest: Arc<RwLock<SpectrumFrame>>,
    source_kind: SourceKind,
}

impl LiveFrameStream {
    pub fn spawn(config: VisualizerConfig) -> Self {
        let bar_count = config.bars.max(1);
        let latest = Arc::new(RwLock::new(SpectrumFrame::from_clamped(
            &vec![0.0; bar_count],
            now_millis(),
        )));
        let framerate = config.framerate.max(1);
        let pipewire_tuning = PipewireTuning::from_config(&config);

        let source_kind = match config.backend {
            VisualizerBackend::Dummy => {
                spawn_dummy_thread(Arc::clone(&latest), bar_count, framerate);
                SourceKind::Dummy
            }
            VisualizerBackend::Pipewire => {
                if spawn_pipewire_thread(Arc::clone(&latest), bar_count, pipewire_tuning).is_ok() {
                    SourceKind::Pipewire
                } else if spawn_cava_thread(Arc::clone(&latest), bar_count, framerate).is_ok() {
                    SourceKind::Cava
                } else {
                    warn!("cavaii: falling back to dummy frame source");
                    spawn_dummy_thread(Arc::clone(&latest), bar_count, framerate);
                    SourceKind::Dummy
                }
            }
            VisualizerBackend::Cava => {
                if spawn_cava_thread(Arc::clone(&latest), bar_count, framerate).is_ok() {
                    SourceKind::Cava
                } else if spawn_pipewire_thread(Arc::clone(&latest), bar_count, pipewire_tuning)
                    .is_ok()
                {
                    SourceKind::Pipewire
                } else {
                    warn!("cavaii: falling back to dummy frame source");
                    spawn_dummy_thread(Arc::clone(&latest), bar_count, framerate);
                    SourceKind::Dummy
                }
            }
            VisualizerBackend::Auto => {
                if spawn_cava_thread(Arc::clone(&latest), bar_count, framerate).is_ok() {
                    SourceKind::Cava
                } else if spawn_pipewire_thread(Arc::clone(&latest), bar_count, pipewire_tuning)
                    .is_ok()
                {
                    SourceKind::Pipewire
                } else {
                    warn!("cavaii: falling back to dummy frame source");
                    spawn_dummy_thread(Arc::clone(&latest), bar_count, framerate);
                    SourceKind::Dummy
                }
            }
        };

        Self {
            latest,
            source_kind,
        }
    }

    pub fn source_kind(&self) -> SourceKind {
        self.source_kind
    }

    pub fn latest_frame(&self) -> SpectrumFrame {
        match self.latest.read() {
            Ok(frame) => frame.clone(),
            Err(_) => SpectrumFrame::from_clamped(&[], now_millis()),
        }
    }
}

fn spawn_dummy_thread(latest: Arc<RwLock<SpectrumFrame>>, bar_count: usize, framerate: u32) {
    let frame_delay = Duration::from_millis((1000_u64 / u64::from(framerate)).max(1));

    thread::spawn(move || {
        let mut source = DummySineSource::new(bar_count);
        loop {
            let frame = source.next_frame();
            if let Ok(mut target) = latest.write() {
                *target = frame;
            }
            thread::sleep(frame_delay);
        }
    });
}

fn spawn_pipewire_thread(
    latest: Arc<RwLock<SpectrumFrame>>,
    bar_count: usize,
    tuning: PipewireTuning,
) -> std::io::Result<()> {
    let mut command = Command::new("pw-cat");
    command
        .arg("--record")
        .arg("--raw")
        .arg("--format")
        .arg("f32")
        .arg("--rate")
        .arg(PIPEWIRE_RATE.to_string())
        .arg("--channels")
        .arg("2")
        .arg("--latency")
        .arg("64")
        .arg("--media-category")
        .arg("Capture")
        .arg("--media-role")
        .arg("Music")
        .arg("-P")
        .arg("stream.capture.sink=true")
        .arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = command.spawn()?;

    // Detect immediate startup failures so auto mode can fall back quickly.
    thread::sleep(Duration::from_millis(120));
    if let Some(status) = child.try_wait()? {
        return Err(std::io::Error::other(format!(
            "pw-cat exited early with status {status}"
        )));
    }

    thread::spawn(move || {
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                error!("cavaii: pw-cat did not provide stdout");
                let _ = child.kill();
                return;
            }
        };

        let mut reader = BufReader::new(stdout);
        let mut read_buf = [0_u8; 8192];
        let mut pending = Vec::<u8>::with_capacity(16384);
        let mut smoothed = vec![0.0_f32; bar_count];
        let mut scratch = PipewireBarsScratch::new(bar_count);
        let frame_stride = 2 * std::mem::size_of::<f32>();

        loop {
            let read = match reader.read(&mut read_buf) {
                Ok(0) => break,
                Ok(value) => value,
                Err(err) => {
                    error!("cavaii: error reading pw-cat output: {err}");
                    break;
                }
            };

            pending.extend_from_slice(&read_buf[..read]);
            let usable = pending.len() - (pending.len() % frame_stride);
            if usable < frame_stride {
                continue;
            }

            let bars = scratch.compute(&pending[..usable], 2, tuning);
            apply_decay_smoothing(&mut smoothed, &bars, tuning.attack, tuning.decay);
            let frame = SpectrumFrame::from_clamped(&smoothed, now_millis());
            if let Ok(mut target) = latest.write() {
                *target = frame;
            }

            if usable == pending.len() {
                pending.clear();
            } else {
                let tail_len = pending.len() - usable;
                pending.copy_within(usable.., 0);
                pending.truncate(tail_len);
            }
        }

        let _ = child.kill();
    });

    Ok(())
}

fn spawn_cava_thread(
    latest: Arc<RwLock<SpectrumFrame>>,
    bar_count: usize,
    framerate: u32,
) -> std::io::Result<()> {
    let config_path = write_cava_config(bar_count, framerate)?;

    thread::spawn(move || {
        let mut command = Command::new("cava");
        command
            .arg("-p")
            .arg(&config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                error!("cavaii: failed to start cava: {err}");
                let _ = fs::remove_file(&config_path);
                return;
            }
        };

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                error!("cavaii: cava did not provide stdout");
                let _ = fs::remove_file(&config_path);
                let _ = child.kill();
                return;
            }
        };

        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut smoothed = vec![0.0_f32; bar_count];
        let mut parsed = vec![0.0_f32; bar_count];
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if parse_cava_line_into(&line, &mut parsed) {
                        apply_decay_smoothing(&mut smoothed, &parsed, CAVA_ATTACK, CAVA_DECAY);
                        let frame = SpectrumFrame::from_clamped(&smoothed, now_millis());
                        if let Ok(mut target) = latest.write() {
                            *target = frame;
                        }
                    }
                }
                Err(err) => {
                    error!("cavaii: error reading cava output: {err}");
                    break;
                }
            }
        }

        let _ = fs::remove_file(&config_path);
        let _ = child.kill();
    });

    Ok(())
}

fn apply_decay_smoothing(smoothed: &mut [f32], input: &[f32], attack: f32, decay: f32) {
    for (current, next) in smoothed.iter_mut().zip(input.iter()) {
        let target = next.clamp(0.0, 1.0);
        if target > *current {
            *current = (*current * (1.0 - attack)) + (target * attack);
        } else {
            *current *= decay;
            if *current < target {
                *current = target;
            }
        }
    }
}

fn write_cava_config(bar_count: usize, framerate: u32) -> std::io::Result<PathBuf> {
    let timestamp = now_millis();
    let path = env::temp_dir().join(format!(
        "cavaii-cava-{}-{timestamp}.conf",
        std::process::id()
    ));

    let config = format!(
        "[general]
bars = {bar_count}
framerate = {framerate}

[input]
method = pulse
source = auto

[output]
method = raw
raw_target = /dev/stdout
data_format = ascii
ascii_max_range = 1000
bar_delimiter = 59
frame_delimiter = 10
"
    );

    fs::write(&path, config)?;
    Ok(path)
}

#[cfg(test)]
fn parse_cava_line(line: &str, expected_bars: usize) -> Option<Vec<f32>> {
    let mut bars = vec![0.0_f32; expected_bars];
    if parse_cava_line_into(line, &mut bars) {
        Some(bars)
    } else {
        None
    }
}

fn parse_cava_line_into(line: &str, output: &mut [f32]) -> bool {
    if output.is_empty() {
        return false;
    }

    let mut written = 0usize;
    for token in line.trim().split(';') {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        let raw = match trimmed.parse::<f32>() {
            Ok(value) => value,
            Err(_) => return false,
        };

        if written < output.len() {
            output[written] = (raw / 1000.0).clamp(0.0, 1.0);
            written += 1;
        } else {
            break;
        }
    }

    if written == 0 {
        return false;
    }

    for slot in &mut output[written..] {
        *slot = 0.0;
    }
    true
}

fn now_millis() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis().min(u64::MAX as u128) as u64,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{PipewireBarsScratch, PipewireTuning, parse_cava_line};

    #[test]
    fn parses_ascii_bar_line() {
        let parsed = parse_cava_line("50;125;1000\n", 3);
        assert_eq!(parsed, Some(vec![0.05, 0.125, 1.0]));
    }

    #[test]
    fn pads_short_line_to_expected_count() {
        let parsed = parse_cava_line("900;450\n", 4);
        assert_eq!(parsed, Some(vec![0.9, 0.45, 0.0, 0.0]));
    }

    #[test]
    fn builds_bars_from_interleaved_f32le() {
        let samples: [f32; 8] = [0.1, -0.1, 0.8, -0.8, 0.2, 0.2, 0.9, 0.9];
        let mut bytes = Vec::new();
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }

        let tuning = PipewireTuning {
            attack: 0.2,
            decay: 0.9,
            gain: 1.0,
            curve: 1.0,
            neighbor_mix: 0.2,
        };
        let mut scratch = PipewireBarsScratch::new(2);
        let bars = scratch.compute(&bytes, 2, tuning);
        assert_eq!(bars.len(), 2);
        assert!(bars[0] > 0.0);
        assert!(bars[1] > 0.0);
        assert!(bars[1] >= bars[0]);
    }
}
