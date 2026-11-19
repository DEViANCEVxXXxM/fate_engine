//! Opt-in, presentation-neutral render telemetry transport.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use super::{GpuFrameKind, GpuFrameTiming};

pub const RENDER_DIAGNOSTIC_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct RenderFrameDiagnostics {
    pub frame_id: u64,
    pub kind: GpuFrameKind,
    /// Time between calls to this renderer; absent for the first call.
    pub interval: Option<Duration>,
    /// Entire render call, including acquisition and presentation.
    pub render_time: Duration,
    /// Command recording/upload preparation, a subset of render_time.
    pub record_time: Duration,
    /// Input count before validation or budgeting. For tilemaps this is the
    /// sum of grid capacity across submitted layers (including empty and
    /// offscreen cells) plus sprites.
    pub requested_instances: usize,
    pub submitted_instances: usize,
    pub draw_calls: usize,
    pub submitted: bool,
    pub surface_size: (u32, u32),
}

#[derive(Clone, Copy, Debug)]
pub enum RenderDiagnosticEvent {
    Frame(RenderFrameDiagnostics),
    Gpu(GpuFrameTiming),
}

/// A bounded nonblocking subscription. Dropping it disconnects the observer.
pub struct RenderDiagnosticReceiver {
    receiver: Receiver<RenderDiagnosticEvent>,
    dropped: Arc<AtomicU64>,
}

impl RenderDiagnosticReceiver {
    pub fn drain(&self) -> impl Iterator<Item = RenderDiagnosticEvent> + '_ {
        self.receiver.try_iter().take(RENDER_DIAGNOSTIC_CAPACITY)
    }

    /// Number of events lost because the consumer did not drain promptly.
    pub fn take_dropped(&self) -> u64 {
        self.dropped.swap(0, Ordering::Relaxed)
    }
}

pub(super) struct RenderDiagnosticSender {
    sender: SyncSender<RenderDiagnosticEvent>,
    dropped: Arc<AtomicU64>,
    pub last_start: Option<Instant>,
    pub next_id: u64,
}

impl RenderDiagnosticSender {
    /// False on disconnection; a full queue only drops telemetry, never a frame.
    pub fn send(&self, event: RenderDiagnosticEvent) -> bool {
        match self.sender.try_send(event) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

pub(super) fn channel() -> (RenderDiagnosticSender, RenderDiagnosticReceiver) {
    let (sender, receiver) = sync_channel(RENDER_DIAGNOSTIC_CAPACITY);
    let dropped = Arc::new(AtomicU64::new(0));
    (
        RenderDiagnosticSender {
            sender,
            dropped: dropped.clone(),
            last_start: None,
            next_id: 0,
        },
        RenderDiagnosticReceiver { receiver, dropped },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_and_disconnected_consumers_never_block_rendering() {
        let (sender, receiver) = channel();
        let event = RenderDiagnosticEvent::Gpu(GpuFrameTiming {
            sample_id: 0,
            kind: GpuFrameKind::Meshes3d,
            scene: Duration::ZERO,
            overlay: None,
        });
        for _ in 0..RENDER_DIAGNOSTIC_CAPACITY + 7 {
            assert!(sender.send(event));
        }
        assert_eq!(receiver.take_dropped(), 7);
        assert_eq!(receiver.drain().count(), RENDER_DIAGNOSTIC_CAPACITY);
        assert_eq!(receiver.take_dropped(), 0);
        assert!(sender.send(event));
        drop(receiver);
        assert!(!sender.send(event));
    }
}
