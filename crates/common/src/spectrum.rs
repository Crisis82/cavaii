use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct SpectrumFrame {
    pub bars: Arc<[f32]>,
    pub peak: f32,
    pub timestamp_millis: u64,
}

impl SpectrumFrame {
    pub fn new(bars: &[f32], timestamp_millis: u64) -> Self {
        let mut peak = 0.0_f32;
        let mut clamped_bars = Vec::with_capacity(bars.len());
        for &value in bars {
            let clamped = value.clamp(0.0, 1.0);
            peak = peak.max(clamped);
            clamped_bars.push(clamped);
        }

        Self {
            bars: Arc::from(clamped_bars.into_boxed_slice()),
            peak,
            timestamp_millis,
        }
    }

    pub fn from_clamped(bars: &[f32], timestamp_millis: u64) -> Self {
        let mut peak = 0.0_f32;
        let mut owned = Vec::with_capacity(bars.len());
        for &value in bars {
            peak = peak.max(value);
            owned.push(value);
        }

        Self {
            bars: Arc::from(owned.into_boxed_slice()),
            peak,
            timestamp_millis,
        }
    }

    pub fn bar_count(&self) -> usize {
        self.bars.len()
    }
}

#[cfg(test)]
mod tests {
    use super::SpectrumFrame;
    use std::sync::Arc;

    #[test]
    fn clamps_values_to_unit_range() {
        let frame = SpectrumFrame::new(&[-1.0, 0.4, 2.0], 0);
        let expected: Arc<[f32]> = Arc::from(vec![0.0, 0.4, 1.0].into_boxed_slice());
        assert_eq!(frame.bars, expected);
        assert_eq!(frame.peak, 1.0);
    }
}
