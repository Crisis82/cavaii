use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use cavaii_common::spectrum::SpectrumFrame;

pub(super) fn spawn_dummy_thread(
    latest: Arc<RwLock<SpectrumFrame>>,
    bar_count: usize,
    framerate: u32,
) {
    let fps = f64::from(framerate.max(1));
    let frame_delay = Duration::from_secs_f64((1.0 / fps).max(0.001));

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

#[derive(Debug)]
struct DummySineSource {
    bar_count: usize,
    phase: f32,
    frame_index: u64,
}

impl DummySineSource {
    fn new(bar_count: usize) -> Self {
        Self {
            bar_count,
            phase: 0.0,
            frame_index: 0,
        }
    }

    fn next_frame(&mut self) -> SpectrumFrame {
        let mut bars = Vec::with_capacity(self.bar_count);
        let spread = 0.35_f32;

        for index in 0..self.bar_count {
            let position = index as f32 * spread + self.phase;
            let value = (position.sin() * 0.5) + 0.5;
            bars.push(value);
        }

        self.phase += 0.2;
        self.frame_index += 1;

        SpectrumFrame::from_clamped(&bars, self.frame_index * 16)
    }
}

#[cfg(test)]
mod tests {
    use super::DummySineSource;

    #[test]
    fn produces_expected_bar_count() {
        let mut source = DummySineSource::new(12);
        let frame = source.next_frame();
        assert_eq!(frame.bar_count(), 12);
    }

    #[test]
    #[ignore = "manual perf smoke test"]
    fn perf_dummy_source_generation() {
        let mut source = DummySineSource::new(120);
        let iterations = 300_000_u32;
        use std::hint::black_box;

        let start = std::time::Instant::now();
        for _ in 0..iterations {
            let frame = source.next_frame();
            black_box(frame.peak);
        }
        let elapsed = start.elapsed();
        let us_per_iter = elapsed.as_secs_f64() * 1_000_000.0 / f64::from(iterations);
        black_box(source.phase);
        eprintln!(
            "perf_dummy_source_generation: iterations={iterations}, elapsed={elapsed:?}, us/iter={us_per_iter:.3}"
        );
    }
}
