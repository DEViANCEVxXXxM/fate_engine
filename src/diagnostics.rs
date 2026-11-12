//! Reusable frame diagnostics and structured metric sinks.
//!
//! This module is a small foundation, not a profiler or an overlay. It
//! provides rolling frame statistics, a machine-readable [`FrameReport`], and
//! sink/source seams so future systems can expose counters and gauges without
//! binding the engine to a particular exporter or presentation layer.
//!
//! Consumers own presentation. A project can log the report, draw it in its own
//! HUD, or forward it to an external metrics system.

use std::fmt;
use std::fmt::Write as _;

use crate::time::Time;

/// Number of frame samples retained by [`FrameStats`].
pub const DEFAULT_FRAME_WINDOW: usize = 60;

/// Timing for one frame, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameSample {
    pub sim_ms: f32,
    pub render_ms: f32,
    pub drawn_instances: usize,
}

impl FrameSample {
    pub const fn new(sim_ms: f32, render_ms: f32, drawn_instances: usize) -> Self {
        Self {
            sim_ms,
            render_ms,
            drawn_instances,
        }
    }

    pub fn total_ms(self) -> f32 {
        self.sim_ms + self.render_ms
    }
}

/// Budget classification for a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetPolicy {
    Healthy,
    Warning,
    Critical,
}

impl BudgetPolicy {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }

    /// Classify a frame from its total time, target budget, and draw pressure.
    ///
    /// `draw_pressure` is expected in `0.0..=1.0`. The thresholds match the
    /// current demo policy: healthy frames stay within the target and below
    /// 75% draw pressure; warning frames stay within 125% of the target and
    /// below 90% pressure; everything else is critical.
    pub fn classify(total_ms: f32, target_ms: f32, draw_pressure: f32) -> Self {
        if total_ms <= target_ms && draw_pressure < 0.75 {
            Self::Healthy
        } else if total_ms <= target_ms * 1.25 && draw_pressure < 0.9 {
            Self::Warning
        } else {
            Self::Critical
        }
    }
}

/// Rolling frame timing and budget counters.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameStats {
    recent_frame_times: [f32; DEFAULT_FRAME_WINDOW],
    recent_count: usize,
    next_index: usize,
    healthy_frames: usize,
    warning_frames: usize,
    critical_frames: usize,
    peak_draw_pressure: f32,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            recent_frame_times: [0.0; DEFAULT_FRAME_WINDOW],
            recent_count: 0,
            next_index: 0,
            healthy_frames: 0,
            warning_frames: 0,
            critical_frames: 0,
            peak_draw_pressure: 0.0,
        }
    }
}

impl FrameStats {
    /// Record a frame time without budget classification.
    pub fn add(&mut self, frame_ms: f32) {
        let len = self.recent_frame_times.len();
        self.recent_frame_times[self.next_index] = frame_ms;
        self.next_index = (self.next_index + 1) % len;
        self.recent_count = (self.recent_count + 1).min(len);
    }

    /// Record a frame time and its budget classification.
    pub fn record(&mut self, frame_ms: f32, policy: BudgetPolicy, draw_pressure: f32) {
        self.add(frame_ms);
        self.peak_draw_pressure = self.peak_draw_pressure.max(draw_pressure);

        match policy {
            BudgetPolicy::Healthy => self.healthy_frames += 1,
            BudgetPolicy::Warning => self.warning_frames += 1,
            BudgetPolicy::Critical => self.critical_frames += 1,
        }
    }

    pub fn recent_count(&self) -> usize {
        self.recent_count
    }

    pub fn average_ms(&self) -> f32 {
        if self.recent_count == 0 {
            return 0.0;
        }

        let count = self.recent_count.min(self.recent_frame_times.len());
        let sum: f32 = self.recent_frame_times.iter().take(count).sum();
        sum / count as f32
    }

    pub fn worst_ms(&self) -> f32 {
        if self.recent_count == 0 {
            return 0.0;
        }

        let count = self.recent_count.min(self.recent_frame_times.len());
        self.recent_frame_times
            .iter()
            .take(count)
            .fold(0.0, |max, value| max.max(*value))
    }

    pub fn snapshot(&self) -> (f32, f32) {
        (self.average_ms(), self.worst_ms())
    }

    pub fn budget_breakdown(&self) -> (usize, usize, usize) {
        (
            self.healthy_frames,
            self.warning_frames,
            self.critical_frames,
        )
    }

    pub fn peak_draw_pressure(&self) -> f32 {
        self.peak_draw_pressure
    }

    pub fn peak_draw_pressure_pct(&self) -> f32 {
        self.peak_draw_pressure * 100.0
    }
}

/// Machine-readable summary of the latest frame and rolling statistics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameReport {
    pub sim_ms: f32,
    pub render_ms: f32,
    pub total_ms: f32,
    pub avg_ms: f32,
    pub worst_ms: f32,
    pub target_ms: f32,
    pub headroom_ms: f32,
    pub draw_count: usize,
    pub draw_limit: usize,
    pub draw_pressure_pct: f32,
    pub healthy_frames: usize,
    pub warning_frames: usize,
    pub critical_frames: usize,
    pub policy: BudgetPolicy,
}

impl FrameReport {
    pub fn summary(&self) -> String {
        format!(
            "sim={:.1}ms render={:.1}ms drawn={}/{} load={:.1}% total={:.1}ms avg={:.1}ms worst={:.1}ms target={:.1}ms headroom={:.1}ms budget=healthy:{}/warning:{}/critical:{} tier={}",
            self.sim_ms,
            self.render_ms,
            self.draw_count,
            self.draw_limit,
            self.draw_pressure_pct,
            self.total_ms,
            self.avg_ms,
            self.worst_ms,
            self.target_ms,
            self.headroom_ms,
            self.healthy_frames,
            self.warning_frames,
            self.critical_frames,
            self.policy.label(),
        )
    }
}

/// Frame timing collector with a fixed target and draw limit.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameDiagnostics {
    stats: FrameStats,
    target_ms: f32,
    draw_limit: usize,
    last: FrameSample,
    last_policy: BudgetPolicy,
}

impl FrameDiagnostics {
    pub fn new(target_ms: f32, draw_limit: usize) -> Self {
        Self {
            stats: FrameStats::default(),
            target_ms: target_ms.max(0.0),
            draw_limit,
            last: FrameSample::default(),
            last_policy: BudgetPolicy::Healthy,
        }
    }

    /// Use the engine's current frame budget as the target.
    pub fn with_default_target(draw_limit: usize) -> Self {
        Self::new(Time::frame_budget_ms() as f32, draw_limit)
    }

    pub fn set_target_ms(&mut self, target_ms: f32) {
        self.target_ms = target_ms.max(0.0);
    }

    pub fn set_draw_limit(&mut self, draw_limit: usize) {
        self.draw_limit = draw_limit;
    }

    pub fn target_ms(&self) -> f32 {
        self.target_ms
    }

    pub fn draw_limit(&self) -> usize {
        self.draw_limit
    }

    pub fn stats(&self) -> &FrameStats {
        &self.stats
    }

    /// Record one frame and return the updated report.
    pub fn record(&mut self, sample: FrameSample) -> FrameReport {
        let draw_pressure = self.draw_pressure_for(sample.drawn_instances);
        let policy = BudgetPolicy::classify(sample.total_ms(), self.target_ms, draw_pressure);
        self.stats.record(sample.total_ms(), policy, draw_pressure);
        self.last = sample;
        self.last_policy = policy;
        self.report()
    }

    /// Report the latest recorded frame. Returns a zeroed report before the
    /// first call to [`Self::record`].
    pub fn report(&self) -> FrameReport {
        let total_ms = self.last.total_ms();
        let (avg_ms, worst_ms) = self.stats.snapshot();
        let (healthy_frames, warning_frames, critical_frames) = self.stats.budget_breakdown();

        FrameReport {
            sim_ms: self.last.sim_ms,
            render_ms: self.last.render_ms,
            total_ms,
            avg_ms,
            worst_ms,
            target_ms: self.target_ms,
            headroom_ms: self.target_ms - total_ms,
            draw_count: self.last.drawn_instances,
            draw_limit: self.draw_limit,
            draw_pressure_pct: self.draw_pressure() * 100.0,
            healthy_frames,
            warning_frames,
            critical_frames,
            policy: self.last_policy,
        }
    }

    pub fn policy(&self) -> BudgetPolicy {
        self.last_policy
    }

    /// Draw pressure for the latest frame, in `0.0..=1.0`.
    pub fn draw_pressure(&self) -> f32 {
        self.draw_pressure_for(self.last.drawn_instances)
    }

    fn draw_pressure_for(&self, drawn_instances: usize) -> f32 {
        if self.draw_limit == 0 {
            0.0
        } else {
            (drawn_instances as f32 / self.draw_limit as f32).clamp(0.0, 1.0)
        }
    }
}

/// A single structured diagnostic value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiagnosticValue {
    Count(u64),
    Gauge(f32),
    Millis(f32),
    Text(&'static str),
}

impl fmt::Display for DiagnosticValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Count(value) => write!(formatter, "{value}"),
            Self::Gauge(value) => write!(formatter, "{value:.3}"),
            Self::Millis(value) => write!(formatter, "{value:.3}ms"),
            Self::Text(value) => formatter.write_str(value),
        }
    }
}

/// Destination for structured diagnostic values.
///
/// Implementations should be cheap and must not block simulation work. Call
/// [`DiagnosticSink::flush`] at a frame boundary to publish buffered values.
pub trait DiagnosticSink {
    fn record(&mut self, group: &'static str, name: &'static str, value: DiagnosticValue);

    fn flush(&mut self) {}
}

/// Source that can contribute diagnostics on demand.
///
/// Future renderer, simulation, or network systems can implement this trait
/// without making the engine depend on their concrete types.
pub trait DiagnosticSource {
    fn record_diagnostics(&self, sink: &mut dyn DiagnosticSink);
}

/// Sink that discards every value.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullSink;

impl DiagnosticSink for NullSink {
    fn record(&mut self, _group: &'static str, _name: &'static str, _value: DiagnosticValue) {}
}

/// Sink that buffers one `key=value` line and emits it through `log` on flush.
#[derive(Debug, Default)]
pub struct LogSink {
    line: String,
}

impl DiagnosticSink for LogSink {
    fn record(&mut self, group: &'static str, name: &'static str, value: DiagnosticValue) {
        if !self.line.is_empty() {
            self.line.push(' ');
        }
        let _ = write!(self.line, "{group}.{name}={value}");
    }

    fn flush(&mut self) {
        if !self.line.is_empty() {
            log::debug!(target: "fate.diagnostics", "{}", self.line);
            self.line.clear();
        }
    }
}

/// Frame diagnostics plus a structured sink for future exporters.
pub struct Diagnostics<S: DiagnosticSink = NullSink> {
    frame: FrameDiagnostics,
    sink: S,
}

impl<S: DiagnosticSink> Diagnostics<S> {
    pub fn new(frame: FrameDiagnostics, sink: S) -> Self {
        Self { frame, sink }
    }

    pub fn frame(&self) -> &FrameDiagnostics {
        &self.frame
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Record a frame, emit its values to the sink, and flush.
    pub fn record_frame(&mut self, sample: FrameSample) -> FrameReport {
        let report = self.frame.record(sample);
        self.emit_frame(&report);
        report
    }

    /// Emit an existing report to the sink and flush.
    pub fn emit_frame(&mut self, report: &FrameReport) {
        self.sink
            .record("frame", "sim_ms", DiagnosticValue::Millis(report.sim_ms));
        self.sink.record(
            "frame",
            "render_ms",
            DiagnosticValue::Millis(report.render_ms),
        );
        self.sink.record(
            "frame",
            "total_ms",
            DiagnosticValue::Millis(report.total_ms),
        );
        self.sink
            .record("frame", "avg_ms", DiagnosticValue::Millis(report.avg_ms));
        self.sink.record(
            "frame",
            "worst_ms",
            DiagnosticValue::Millis(report.worst_ms),
        );
        self.sink.record(
            "frame",
            "target_ms",
            DiagnosticValue::Millis(report.target_ms),
        );
        self.sink.record(
            "frame",
            "headroom_ms",
            DiagnosticValue::Millis(report.headroom_ms),
        );
        self.sink.record(
            "frame",
            "drawn",
            DiagnosticValue::Count(report.draw_count as u64),
        );
        self.sink.record(
            "frame",
            "draw_limit",
            DiagnosticValue::Count(report.draw_limit as u64),
        );
        self.sink.record(
            "frame",
            "draw_pressure_pct",
            DiagnosticValue::Gauge(report.draw_pressure_pct),
        );
        self.sink.record(
            "budget",
            "healthy_frames",
            DiagnosticValue::Count(report.healthy_frames as u64),
        );
        self.sink.record(
            "budget",
            "warning_frames",
            DiagnosticValue::Count(report.warning_frames as u64),
        );
        self.sink.record(
            "budget",
            "critical_frames",
            DiagnosticValue::Count(report.critical_frames as u64),
        );
        self.sink.record(
            "budget",
            "tier",
            DiagnosticValue::Text(report.policy.label()),
        );
        self.sink.flush();
    }

    pub fn record(&mut self, group: &'static str, name: &'static str, value: DiagnosticValue) {
        self.sink.record(group, name, value);
    }

    /// Sample every source and flush once.
    pub fn sample_sources(&mut self, sources: &[&dyn DiagnosticSource]) {
        for source in sources {
            source.record_diagnostics(&mut self.sink);
        }
        self.sink.flush();
    }

    pub fn flush(&mut self) {
        self.sink.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        entries: Vec<(&'static str, &'static str, DiagnosticValue)>,
        flushes: usize,
    }

    impl DiagnosticSink for RecordingSink {
        fn record(&mut self, group: &'static str, name: &'static str, value: DiagnosticValue) {
            self.entries.push((group, name, value));
        }

        fn flush(&mut self) {
            self.flushes += 1;
        }
    }

    struct TestSource;

    impl DiagnosticSource for TestSource {
        fn record_diagnostics(&self, sink: &mut dyn DiagnosticSink) {
            sink.record("world", "entities", DiagnosticValue::Count(3));
        }
    }

    #[test]
    fn frame_sample_totals_simulation_and_render_time() {
        let sample = FrameSample::new(3.0, 4.5, 12);
        assert!((sample.total_ms() - 7.5).abs() < 1e-6);
    }

    #[test]
    fn budget_policy_classifies_time_and_draw_pressure() {
        assert_eq!(
            BudgetPolicy::classify(7.0, 8.0, 0.25),
            BudgetPolicy::Healthy
        );
        assert_eq!(BudgetPolicy::classify(9.0, 8.0, 0.8), BudgetPolicy::Warning);
        assert_eq!(
            BudgetPolicy::classify(11.0, 8.0, 0.1),
            BudgetPolicy::Critical
        );
        assert_eq!(
            BudgetPolicy::classify(4.0, 8.0, 0.95),
            BudgetPolicy::Critical
        );
    }

    #[test]
    fn frame_stats_roll_and_average_recent_frames() {
        let mut stats = FrameStats::default();
        stats.add(4.0);
        stats.add(6.0);
        stats.add(8.0);

        assert!((stats.average_ms() - 6.0).abs() < 1e-5);
        assert!((stats.worst_ms() - 8.0).abs() < 1e-5);
        assert_eq!(stats.recent_count(), 3);
        assert_eq!(stats.snapshot(), (6.0, 8.0));
    }

    #[test]
    fn frame_stats_keep_the_most_recent_window_after_wrapping() {
        let mut stats = FrameStats::default();
        for _ in 0..DEFAULT_FRAME_WINDOW {
            stats.add(100.0);
        }
        for _ in 0..DEFAULT_FRAME_WINDOW {
            stats.add(1.0);
        }

        assert_eq!(stats.recent_count(), DEFAULT_FRAME_WINDOW);
        assert!((stats.average_ms() - 1.0).abs() < 1e-5);
        assert!((stats.worst_ms() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn frame_stats_track_budget_breakdown_and_peak_pressure() {
        let mut stats = FrameStats::default();
        stats.record(7.0, BudgetPolicy::Healthy, 0.25);
        stats.record(9.0, BudgetPolicy::Warning, 0.8);
        stats.record(12.0, BudgetPolicy::Critical, 1.0);

        assert_eq!(stats.budget_breakdown(), (1, 1, 1));
        assert!((stats.peak_draw_pressure_pct() - 100.0).abs() < 1e-5);
    }

    #[test]
    fn frame_diagnostics_reports_latest_frame_and_policy() {
        let mut diagnostics = FrameDiagnostics::new(8.0, 256);
        let report = diagnostics.record(FrameSample::new(3.0, 4.0, 128));

        assert_eq!(report.total_ms, 7.0);
        assert_eq!(report.avg_ms, 7.0);
        assert_eq!(report.target_ms, 8.0);
        assert_eq!(report.headroom_ms, 1.0);
        assert_eq!(report.draw_count, 128);
        assert_eq!(report.draw_limit, 256);
        assert_eq!(report.policy, BudgetPolicy::Healthy);
        assert_eq!(diagnostics.policy(), BudgetPolicy::Healthy);
        assert!(diagnostics.draw_pressure() > 0.49);
    }

    #[test]
    fn frame_report_summary_contains_machine_readable_fields() {
        let mut diagnostics = FrameDiagnostics::new(8.0, 256);
        let report = diagnostics.record(FrameSample::new(3.0, 4.0, 2));

        let summary = report.summary();
        assert!(summary.contains("drawn=2/256"));
        assert!(summary.contains("load=0.8%"));
        assert!(summary.contains("total=7.0ms"));
        assert!(summary.contains("target=8.0ms"));
        assert!(summary.contains("headroom=1.0ms"));
        assert!(summary.contains("tier=healthy"));
    }

    #[test]
    fn diagnostics_emits_frame_values_and_flushes_once() {
        let mut diagnostics =
            Diagnostics::new(FrameDiagnostics::new(8.0, 256), RecordingSink::default());
        diagnostics.record_frame(FrameSample::new(3.0, 4.0, 128));

        assert_eq!(diagnostics.sink().flushes, 1);
        assert!(diagnostics.sink().entries.contains(&(
            "frame",
            "total_ms",
            DiagnosticValue::Millis(7.0)
        )));
        assert!(diagnostics.sink().entries.contains(&(
            "budget",
            "tier",
            DiagnosticValue::Text("healthy")
        )));
    }

    #[test]
    fn diagnostics_samples_sources_and_flushes_once() {
        let mut diagnostics =
            Diagnostics::new(FrameDiagnostics::new(8.0, 256), RecordingSink::default());
        diagnostics.sample_sources(&[&TestSource]);

        assert_eq!(diagnostics.sink().flushes, 1);
        assert!(diagnostics.sink().entries.contains(&(
            "world",
            "entities",
            DiagnosticValue::Count(3)
        )));
    }

    #[test]
    fn diagnostic_values_display_with_units() {
        assert_eq!(DiagnosticValue::Count(3).to_string(), "3");
        assert_eq!(DiagnosticValue::Gauge(0.5).to_string(), "0.500");
        assert_eq!(DiagnosticValue::Millis(7.25).to_string(), "7.250ms");
        assert_eq!(DiagnosticValue::Text("healthy").to_string(), "healthy");
    }

    #[test]
    fn zero_draw_limit_reports_no_pressure() {
        let mut diagnostics = FrameDiagnostics::new(8.0, 0);
        diagnostics.record(FrameSample::new(1.0, 1.0, 10));
        assert_eq!(diagnostics.draw_pressure(), 0.0);
        assert_eq!(diagnostics.report().draw_pressure_pct, 0.0);
    }
}
