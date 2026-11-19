use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use crate::{DebugModel, EventLevel};

/// Consumer-drained, bounded log channel; logging never waits on the UI.
pub struct LogInbox {
    receiver: Receiver<(EventLevel, String)>,
    dropped: Arc<AtomicU64>,
}

struct CaptureLogger {
    downstream: Box<dyn log::Log>,
    sender: SyncSender<(EventLevel, String)>,
    dropped: Arc<AtomicU64>,
}

/// Wrap an existing logger, retaining its filtering and console output.
pub fn capture_logs(downstream: Box<dyn log::Log>) -> (impl log::Log, LogInbox) {
    let (sender, receiver) = sync_channel(500);
    let dropped = Arc::new(AtomicU64::new(0));
    (
        CaptureLogger {
            downstream,
            sender,
            dropped: dropped.clone(),
        },
        LogInbox { receiver, dropped },
    )
}

impl log::Log for CaptureLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.downstream.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        self.downstream.log(record);
        let level = match record.level() {
            log::Level::Error => EventLevel::Error,
            log::Level::Warn => EventLevel::Warning,
            _ => EventLevel::Info,
        };
        let message = format!("{}: {}", record.target(), record.args());
        if matches!(
            self.sender.try_send((level, message)),
            Err(TrySendError::Full(_))
        ) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn flush(&self) {
        self.downstream.flush();
    }
}

impl LogInbox {
    pub fn drain_into(&self, model: &mut DebugModel) {
        for (level, message) in self.receiver.try_iter().take(500) {
            model.event(level, message);
        }
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            model.event(
                EventLevel::Warning,
                format!("Log inbox full: {dropped} messages dropped"),
            );
        }
    }
}
