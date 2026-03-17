mod cava;
mod dummy;

use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use cavaii_common::config::{VisualizerBackend, VisualizerConfig};
use cavaii_common::spectrum::SpectrumFrame;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Cava,
    Dummy,
}

pub struct LiveFrameStream {
    latest: Arc<RwLock<SpectrumFrame>>,
    source_kind: SourceKind,
}

impl LiveFrameStream {
    pub fn spawn(config: VisualizerConfig) -> Self {
        let bar_count = config.points.max(1);
        let latest = Arc::new(RwLock::new(SpectrumFrame::from_clamped(
            &vec![0.0; bar_count],
            now_millis(),
        )));
        let framerate = config.framerate.max(1);

        let source_kind = match config.backend {
            VisualizerBackend::Dummy => {
                dummy::spawn_dummy_thread(Arc::clone(&latest), bar_count, framerate);
                SourceKind::Dummy
            }
            VisualizerBackend::Cava => {
                if cava::spawn_cava_thread(Arc::clone(&latest), bar_count, framerate).is_ok() {
                    SourceKind::Cava
                } else {
                    warn!("cavaii: falling back to dummy frame source");
                    dummy::spawn_dummy_thread(Arc::clone(&latest), bar_count, framerate);
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

pub(super) fn now_millis() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis().min(u64::MAX as u128) as u64,
        Err(_) => 0,
    }
}
