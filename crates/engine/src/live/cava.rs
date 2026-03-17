use std::env;
use std::fs;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};
use std::thread;

use cavaii_common::spectrum::SpectrumFrame;
use tracing::error;

use super::now_millis;

const CAVA_RAW_U16_MAX: f32 = u16::MAX as f32;

pub(super) fn spawn_cava_thread(
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
        let mut read_buf = [0_u8; 8192];
        let mut pending = Vec::<u8>::with_capacity(16384);
        let frame_bytes = bar_count * std::mem::size_of::<u16>();
        let mut parsed = vec![0.0_f32; bar_count];

        if frame_bytes == 0 {
            let _ = fs::remove_file(&config_path);
            let _ = child.kill();
            return;
        }

        loop {
            let read = match reader.read(&mut read_buf) {
                Ok(0) => break,
                Ok(value) => value,
                Err(err) => {
                    error!("cavaii: error reading cava output: {err}");
                    break;
                }
            };

            pending.extend_from_slice(&read_buf[..read]);
            let usable = pending.len() - (pending.len() % frame_bytes);
            if usable < frame_bytes {
                continue;
            }

            let mut offset = 0usize;
            while offset + frame_bytes <= usable {
                if parse_cava_raw_frame_into(&pending[offset..offset + frame_bytes], &mut parsed) {
                    let frame = SpectrumFrame::from_clamped(&parsed, now_millis());
                    if let Ok(mut target) = latest.write() {
                        *target = frame;
                    }
                }
                offset += frame_bytes;
            }

            if usable == pending.len() {
                pending.clear();
            } else {
                let tail_len = pending.len() - usable;
                pending.copy_within(usable.., 0);
                pending.truncate(tail_len);
            }
        }

        let _ = fs::remove_file(&config_path);
        let _ = child.kill();
    });

    Ok(())
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
data_format = binary
bit_format = 16bit
"
    );

    fs::write(&path, config)?;
    Ok(path)
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

#[cfg(test)]
mod tests {
    use super::parse_cava_raw_frame_into;

    #[test]
    fn parses_cava_raw_u16_frame() {
        let mut parsed = vec![0.0_f32; 3];
        let frame = [
            0_u16.to_le_bytes(),
            32_768_u16.to_le_bytes(),
            65_535_u16.to_le_bytes(),
        ]
        .concat();
        assert!(parse_cava_raw_frame_into(&frame, &mut parsed));
        assert_eq!(parsed[0], 0.0);
        assert!((parsed[1] - 0.500_007_6).abs() < 1e-5);
        assert_eq!(parsed[2], 1.0);
    }

    #[test]
    fn rejects_short_raw_frame() {
        let mut parsed = vec![0.0_f32; 3];
        let frame = [0_u8; 4];
        assert!(!parse_cava_raw_frame_into(&frame, &mut parsed));
        assert_eq!(parsed, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    #[ignore = "manual perf smoke test"]
    fn perf_parse_cava_raw_u16_frame() {
        let frame = vec![0xFF_u8; 120 * 2];
        let mut parsed = vec![0.0_f32; 120];
        let iterations = 1_000_000_u32;
        use std::hint::black_box;

        let start = std::time::Instant::now();
        for _ in 0..iterations {
            let ok = parse_cava_raw_frame_into(black_box(&frame), black_box(&mut parsed));
            black_box(ok);
            black_box(parsed[0]);
        }
        let elapsed = start.elapsed();
        let us_per_iter = elapsed.as_secs_f64() * 1_000_000.0 / f64::from(iterations);
        black_box(&parsed);
        eprintln!(
            "perf_parse_cava_raw_u16_frame: iterations={iterations}, elapsed={elapsed:?}, us/iter={us_per_iter:.3}"
        );
    }
}
