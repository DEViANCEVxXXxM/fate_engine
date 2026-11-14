//! Consumer-owned diagnostics sink and source for the capability showcase.
//!
//! The engine deliberately has no profiler, overlay, or exporter. This module
//! shows how a consumer can collect the engine's structured values, combine
//! them with project state, and publish them through `log` without adding
//! presentation policy to the engine.

use std::fmt::Write as _;

use fate_engine::diagnostics::{DiagnosticSink, DiagnosticSource, DiagnosticValue};

/// Number of sink flushes between published log lines.
///
/// The app flushes twice per frame (frame values, then source values), so this
/// is roughly one published line per second at the 60 FPS presentation rate.
pub const LOG_INTERVAL: u64 = 120;

/// Sink that batches values into one `key=value` line and logs it periodically.
///
/// The app calls [`ShowcaseSink::begin_frame`] before recording frame values,
/// so a publication contains the most recent complete frame snapshot rather
/// than a history of every frame since the last publication. The buffered line
/// is only published when verbose logging is enabled.
#[derive(Debug, Default)]
pub struct ShowcaseSink {
    line: String,
    latest: String,
    verbose: bool,
    flushes: u64,
    replace_latest: bool,
}

impl ShowcaseSink {
    pub fn new(verbose: bool) -> Self {
        Self {
            line: String::new(),
            latest: String::new(),
            verbose,
            flushes: 0,
            replace_latest: false,
        }
    }

    /// Mark the next flush as the start of a new frame snapshot.
    pub fn begin_frame(&mut self) {
        self.replace_latest = true;
    }

    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
        if !verbose {
            self.latest.clear();
        }
    }

    pub fn is_verbose(&self) -> bool {
        self.verbose
    }

    /// Total number of flushes since startup, including discarded ones.
    #[cfg(test)]
    pub fn flushes(&self) -> u64 {
        self.flushes
    }

    /// Values buffered since the last flush.
    #[cfg(test)]
    pub fn line(&self) -> &str {
        &self.line
    }

    /// Most recent complete snapshot, waiting for the next publication.
    #[cfg(test)]
    pub fn latest(&self) -> &str {
        &self.latest
    }
}

impl DiagnosticSink for ShowcaseSink {
    fn record(&mut self, group: &'static str, name: &'static str, value: DiagnosticValue) {
        if !self.line.is_empty() {
            self.line.push(' ');
        }
        let _ = write!(self.line, "{group}.{name}={value}");
    }

    fn flush(&mut self) {
        if !self.line.is_empty() {
            if self.replace_latest {
                self.latest.clear();
                self.replace_latest = false;
            }
            if !self.latest.is_empty() {
                self.latest.push(' ');
            }
            self.latest.push_str(&self.line);
            self.line.clear();
        }

        self.flushes = self.flushes.wrapping_add(1);
        if self.flushes.is_multiple_of(LOG_INTERVAL) {
            if self.verbose && !self.latest.is_empty() {
                log::info!(target: "fate.showcase", "{}", self.latest);
            }
            self.latest.clear();
        }
    }
}

/// Project state contributed to the diagnostics stream each frame.
#[derive(Clone, Copy, Debug)]
pub struct ShowcaseSource {
    pub tick: u64,
    pub alpha: f32,
    pub paused: bool,
    pub state_hash: u64,
    pub verify_ms: f32,
    pub replay_enabled: bool,
    pub replay: &'static str,
    pub rollback: &'static str,
    pub snapshot: &'static str,
    pub live_entities: usize,
    pub capacity: usize,
    pub free_slots: usize,
    pub positions: usize,
    pub visible: usize,
    pub drawn: usize,
}

impl Default for ShowcaseSource {
    fn default() -> Self {
        Self {
            tick: 0,
            alpha: 0.0,
            paused: false,
            state_hash: 0,
            verify_ms: 0.0,
            replay_enabled: true,
            replay: "pending",
            rollback: "pending",
            snapshot: "pending",
            live_entities: 0,
            capacity: 0,
            free_slots: 0,
            positions: 0,
            visible: 0,
            drawn: 0,
        }
    }
}

impl DiagnosticSource for ShowcaseSource {
    fn record_diagnostics(&self, sink: &mut dyn DiagnosticSink) {
        sink.record("sim", "tick", DiagnosticValue::Count(self.tick));
        sink.record("sim", "alpha", DiagnosticValue::Gauge(self.alpha));
        sink.record(
            "sim",
            "paused",
            DiagnosticValue::Text(if self.paused { "on" } else { "off" }),
        );
        sink.record("sim", "state_hash", DiagnosticValue::Count(self.state_hash));

        sink.record(
            "world",
            "live_entities",
            DiagnosticValue::Count(self.live_entities as u64),
        );
        sink.record(
            "world",
            "capacity",
            DiagnosticValue::Count(self.capacity as u64),
        );
        sink.record(
            "world",
            "free_slots",
            DiagnosticValue::Count(self.free_slots as u64),
        );
        sink.record(
            "world",
            "positions",
            DiagnosticValue::Count(self.positions as u64),
        );

        sink.record(
            "render",
            "visible",
            DiagnosticValue::Count(self.visible as u64),
        );
        sink.record("render", "drawn", DiagnosticValue::Count(self.drawn as u64));

        sink.record("verify", "ms", DiagnosticValue::Millis(self.verify_ms));
        sink.record(
            "verify",
            "replay_enabled",
            DiagnosticValue::Text(if self.replay_enabled { "on" } else { "off" }),
        );
        sink.record("verify", "replay", DiagnosticValue::Text(self.replay));
        sink.record("verify", "rollback", DiagnosticValue::Text(self.rollback));
        sink.record("verify", "snapshot", DiagnosticValue::Text(self.snapshot));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct CollectingSink {
        entries: Vec<(&'static str, &'static str, DiagnosticValue)>,
    }

    impl DiagnosticSink for CollectingSink {
        fn record(&mut self, group: &'static str, name: &'static str, value: DiagnosticValue) {
            self.entries.push((group, name, value));
        }
    }

    #[test]
    fn sink_batches_values_into_one_line() {
        let mut sink = ShowcaseSink::new(true);
        sink.begin_frame();
        sink.record("frame", "sim_ms", DiagnosticValue::Millis(1.5));
        sink.record("frame", "drawn", DiagnosticValue::Count(7));

        assert_eq!(sink.line(), "frame.sim_ms=1.500ms frame.drawn=7");

        sink.flush();

        assert!(sink.line().is_empty());
        assert_eq!(sink.latest(), "frame.sim_ms=1.500ms frame.drawn=7");
        assert_eq!(sink.flushes(), 1);
    }

    #[test]
    fn begin_frame_replaces_the_previous_snapshot() {
        let mut sink = ShowcaseSink::new(true);

        sink.begin_frame();
        sink.record("frame", "sim_ms", DiagnosticValue::Millis(1.0));
        sink.flush();
        sink.record("verify", "ms", DiagnosticValue::Millis(0.5));
        sink.flush();
        assert!(sink.latest().contains("frame.sim_ms=1.000ms"));
        assert!(sink.latest().contains("verify.ms=0.500ms"));

        sink.begin_frame();
        sink.record("frame", "sim_ms", DiagnosticValue::Millis(2.0));
        sink.flush();
        sink.record("verify", "ms", DiagnosticValue::Millis(0.1));
        sink.flush();

        assert!(sink.latest().contains("frame.sim_ms=2.000ms"));
        assert!(sink.latest().contains("verify.ms=0.100ms"));
        assert!(!sink.latest().contains("1.000ms"));
    }

    #[test]
    fn verbose_toggle_controls_publication_and_clears_latest() {
        let mut sink = ShowcaseSink::new(true);
        assert!(sink.is_verbose());

        sink.begin_frame();
        sink.record("frame", "total_ms", DiagnosticValue::Millis(2.0));
        sink.flush();
        assert!(!sink.latest().is_empty());

        sink.set_verbose(false);
        assert!(!sink.is_verbose());
        assert!(sink.latest().is_empty());
    }

    #[test]
    fn publication_happens_on_the_interval_and_discards_old_values() {
        let mut sink = ShowcaseSink::new(true);

        for _ in 0..LOG_INTERVAL {
            sink.begin_frame();
            sink.record("frame", "drawn", DiagnosticValue::Count(1));
            sink.flush();
        }

        assert_eq!(sink.flushes(), LOG_INTERVAL);
        assert!(sink.latest().is_empty());
    }

    #[test]
    fn source_reports_simulation_world_render_and_verification_groups() {
        let source = ShowcaseSource {
            tick: 42,
            alpha: 0.25,
            paused: true,
            state_hash: 0xabcd,
            verify_ms: 0.5,
            replay_enabled: false,
            replay: "ok",
            rollback: "failed",
            snapshot: "pending",
            live_entities: 161,
            capacity: 161,
            free_slots: 0,
            positions: 161,
            visible: 120,
            drawn: 120,
        };
        let mut sink = CollectingSink::default();

        source.record_diagnostics(&mut sink);

        assert!(
            sink.entries
                .contains(&("sim", "tick", DiagnosticValue::Count(42)))
        );
        assert!(
            sink.entries
                .contains(&("sim", "paused", DiagnosticValue::Text("on")))
        );
        assert!(
            sink.entries
                .contains(&("world", "live_entities", DiagnosticValue::Count(161)))
        );
        assert!(
            sink.entries
                .contains(&("render", "visible", DiagnosticValue::Count(120)))
        );
        assert!(
            sink.entries
                .contains(&("verify", "replay_enabled", DiagnosticValue::Text("off")))
        );
        assert!(
            sink.entries
                .contains(&("verify", "rollback", DiagnosticValue::Text("failed")))
        );
    }
}
