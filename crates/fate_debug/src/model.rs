use std::collections::VecDeque;
use std::fmt::Write as _;
use std::time::Instant;

pub const HISTORY_LIMIT: usize = 1200;
pub const EVENT_LIMIT: usize = 500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Overview,
    Renderer,
    Simulation,
    Camera,
    Entities,
    Events,
    Capture,
}

impl Page {
    pub const ALL: [Self; 7] = [
        Self::Overview,
        Self::Renderer,
        Self::Simulation,
        Self::Camera,
        Self::Entities,
        Self::Events,
        Self::Capture,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Renderer => "Renderer",
            Self::Simulation => "Simulation",
            Self::Camera => "Camera",
            Self::Entities => "Entities",
            Self::Events => "Events",
            Self::Capture => "Capture",
        }
    }
}

/// Timing domains are intentionally separate. Submit includes acquire/present;
/// recording is a subset of submit. GPU samples arrive on an independent stream.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameSample {
    pub id: u64,
    pub tick: u64,
    pub interval_ms: f32,
    pub sim_ms: f32,
    pub prepare_ms: f32,
    pub submit_ms: f32,
    pub record_ms: f32,
    pub verify_ms: f32,
    pub instances: usize,
    pub draw_calls: usize,
    pub visible: usize,
    pub candidates: usize,
    pub submitted: bool,
}

impl FrameSample {
    pub fn cpu_ms(&self) -> f32 {
        self.sim_ms + self.prepare_ms + self.submit_ms + self.verify_ms
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GpuSample {
    pub id: u64,
    pub scene_ms: f32,
    pub overlay_ms: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct Fact {
    pub page: Page,
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub enum ControlValue {
    Action,
    Toggle(bool),
    Number {
        value: f64,
        min: f64,
        max: f64,
        speed: f64,
    },
}

#[derive(Clone, Debug)]
pub struct Control {
    pub page: Page,
    pub id: &'static str,
    pub label: &'static str,
    pub value: ControlValue,
    pub enabled: bool,
}

impl Control {
    pub fn action(page: Page, id: &'static str, label: &'static str) -> Self {
        Self {
            page,
            id,
            label,
            value: ControlValue::Action,
            enabled: true,
        }
    }

    pub fn toggle(page: Page, id: &'static str, label: &'static str, value: bool) -> Self {
        Self {
            value: ControlValue::Toggle(value),
            ..Self::action(page, id, label)
        }
    }

    pub fn number(
        page: Page,
        id: &'static str,
        label: &'static str,
        value: f64,
        range: std::ops::RangeInclusive<f64>,
    ) -> Self {
        Self {
            value: ControlValue::Number {
                value,
                min: *range.start(),
                max: *range.end(),
                speed: 0.1,
            },
            ..Self::action(page, id, label)
        }
    }
}

#[derive(Clone, Debug)]
pub struct EntityRow {
    pub id: String,
    pub position: [f32; 3],
    pub visible: bool,
}

/// Owned inspection snapshot. The consumer controls which tools exist.
#[derive(Clone, Debug, Default)]
pub struct Inspection {
    pub facts: Vec<Fact>,
    pub controls: Vec<Control>,
    pub entities: Vec<EntityRow>,
}

impl Inspection {
    /// Import any engine `DiagnosticSource` without project-specific UI code.
    pub fn source(&mut self, page: Page, source: &dyn fate_engine::diagnostics::DiagnosticSource) {
        struct Sink<'a> {
            inspection: &'a mut Inspection,
            page: Page,
        }
        impl fate_engine::diagnostics::DiagnosticSink for Sink<'_> {
            fn record(
                &mut self,
                group: &'static str,
                name: &'static str,
                value: fate_engine::diagnostics::DiagnosticValue,
            ) {
                self.inspection
                    .fact(self.page, format!("{group}.{name}"), value);
            }
        }
        source.record_diagnostics(&mut Sink {
            inspection: self,
            page,
        });
    }

    pub fn fact(&mut self, page: Page, label: impl Into<String>, value: impl ToString) {
        self.facts.push(Fact {
            page,
            label: label.into(),
            value: value.to_string(),
        });
    }
}

#[derive(Clone, Debug)]
pub struct Command {
    pub id: &'static str,
    pub value: ControlValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventLevel {
    Info,
    Warning,
    Error,
}

impl EventLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARN",
            Self::Error => "ERROR",
        }
    }
}

pub struct Event {
    pub seconds: f64,
    pub level: EventLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Statistics {
    pub count: usize,
    pub mean: f32,
    pub p50: f32,
    pub p95: f32,
    pub max: f32,
}

impl Statistics {
    pub fn from_values(values: impl Iterator<Item = f32>) -> Self {
        let mut values: Vec<_> = values.filter(|v| v.is_finite() && *v >= 0.0).collect();
        values.sort_unstable_by(f32::total_cmp);
        let count = values.len();
        if count == 0 {
            return Self::default();
        }
        Self {
            count,
            mean: (values.iter().map(|&v| f64::from(v)).sum::<f64>() / count as f64) as f32,
            p50: values[(count - 1) / 2],
            p95: values[(count * 95).div_ceil(100) - 1],
            max: values[count - 1],
        }
    }
}

/// CPU-only bounded storage. Freezing holds both frame and GPU trace streams;
/// inspection and events remain live so tools can still be used.
pub struct DebugModel {
    pub source: String,
    pub sources: Vec<String>,
    pub selected_source: usize,
    /// True only when a host supplies simulation/preparation/verification work.
    pub host_timings_available: bool,
    pub inspection: Inspection,
    pub frames: VecDeque<FrameSample>,
    pub gpu: VecDeque<GpuSample>,
    pub events: VecDeque<Event>,
    pub frozen: bool,
    pub gpu_available: bool,
    pub frame_budget_ms: f32,
    pub submitted: u64,
    pub skipped: u64,
    started: Instant,
    next_frame: u64,
}

impl Default for DebugModel {
    fn default() -> Self {
        Self {
            source: "Engine consumer".into(),
            sources: Vec::new(),
            selected_source: 0,
            host_timings_available: false,
            inspection: Inspection::default(),
            frames: VecDeque::with_capacity(HISTORY_LIMIT),
            gpu: VecDeque::with_capacity(HISTORY_LIMIT),
            events: VecDeque::with_capacity(EVENT_LIMIT),
            frozen: false,
            gpu_available: false,
            frame_budget_ms: 16.667,
            submitted: 0,
            skipped: 0,
            started: Instant::now(),
            next_frame: 0,
        }
    }
}

fn push_bounded<T>(queue: &mut VecDeque<T>, item: T, limit: usize) {
    if queue.len() == limit {
        queue.pop_front();
    }
    queue.push_back(item);
}

impl DebugModel {
    pub fn record_frame(&mut self, mut sample: FrameSample) {
        sample.id = self.next_frame;
        self.next_frame = self.next_frame.wrapping_add(1);
        if sample.submitted {
            self.submitted += 1;
        } else {
            self.skipped += 1;
        }
        if !self.frozen {
            push_bounded(&mut self.frames, sample, HISTORY_LIMIT);
        }
    }

    pub fn record_gpu(&mut self, sample: GpuSample) {
        if !self.frozen {
            push_bounded(&mut self.gpu, sample, HISTORY_LIMIT);
        }
    }

    pub fn event(&mut self, level: EventLevel, message: impl Into<String>) {
        let mut message = message.into();
        // Bound individual payloads too; truncate only at a UTF-8 boundary.
        if message.len() > 2048 {
            let mut end = 2048;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
            message.push_str("...");
        }
        let event = Event {
            seconds: self.started.elapsed().as_secs_f64(),
            level,
            message,
        };
        push_bounded(&mut self.events, event, EVENT_LIMIT);
    }

    pub fn uptime(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn clear_trace(&mut self) {
        self.frames.clear();
        self.gpu.clear();
    }

    pub fn frames_csv(&self) -> String {
        let mut csv = String::from(
            "frame,tick,interval_ms,sim_ms,prepare_ms,submit_ms,record_ms,verify_ms,instances,indexed_draw_calls,visible,candidates,submitted\n",
        );
        for f in &self.frames {
            let _ = writeln!(
                csv,
                "{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{},{},{}",
                f.id,
                f.tick,
                f.interval_ms,
                f.sim_ms,
                f.prepare_ms,
                f.submit_ms,
                f.record_ms,
                f.verify_ms,
                f.instances,
                f.draw_calls,
                f.visible,
                f.candidates,
                f.submitted
            );
        }
        csv
    }

    pub fn gpu_csv(&self) -> String {
        let mut csv = String::from("gpu_sample,scene_ms,overlay_ms\n");
        for f in &self.gpu {
            let overlay = f.overlay_ms.map(|v| format!("{v:.6}")).unwrap_or_default();
            let _ = writeln!(csv, "{},{:.6},{}", f.id, f.scene_ms, overlay);
        }
        csv
    }

    pub fn report(&self) -> String {
        let mut text = format!(
            "Fate diagnostics\nTrace: {} CPU frames, {} GPU samples\n",
            self.frames.len(),
            self.gpu.len()
        );
        for fact in &self.inspection.facts {
            let _ = writeln!(
                text,
                "{} / {}: {}",
                fact.page.title(),
                fact.label,
                fact.value
            );
        }
        text.push_str("\nEvents\n");
        for event in &self.events {
            let _ = writeln!(
                text,
                "{:.3}s [{}] {}",
                event.seconds,
                event.level.label(),
                event.message
            );
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_trace_freeze_and_clear_preserve_sample_identity() {
        let mut model = DebugModel::default();
        for _ in 0..HISTORY_LIMIT + 3 {
            model.record_frame(FrameSample::default());
        }
        assert_eq!(model.frames.len(), HISTORY_LIMIT);
        assert_eq!(model.frames[0].id, 3);
        model.frozen = true;
        model.record_frame(FrameSample::default());
        model.record_gpu(GpuSample {
            id: 4,
            scene_ms: 2.0,
            overlay_ms: None,
        });
        assert!(model.gpu.is_empty());
        assert_eq!(model.frames.back().unwrap().id, (HISTORY_LIMIT + 2) as u64);
        model.clear_trace();
        model.frozen = false;
        model.record_frame(FrameSample::default());
        assert_eq!(model.frames[0].id, (HISTORY_LIMIT + 4) as u64);
    }

    #[test]
    fn quantiles_ignore_invalid_data_and_use_nearest_rank() {
        let stats = Statistics::from_values([f32::NAN, -1.0, 1.0, 2.0, 3.0, 40.0].into_iter());
        assert_eq!(stats.count, 4);
        assert_eq!(stats.p50, 2.0);
        assert_eq!(stats.p95, 40.0);
        assert_eq!(stats.mean, 11.5);
        assert_eq!(Statistics::from_values(std::iter::empty()).count, 0);
    }

    #[test]
    fn exports_keep_cpu_and_async_gpu_domains_separate() {
        let mut model = DebugModel::default();
        model.record_frame(FrameSample {
            tick: 42,
            submitted: true,
            ..Default::default()
        });
        model.record_gpu(GpuSample {
            id: 8,
            scene_ms: 0.5,
            overlay_ms: None,
        });
        assert!(model.frames_csv().contains("0,42,"));
        assert!(model.gpu_csv().contains("8,0.500000,\n"));
        for _ in 0..EVENT_LIMIT + 2 {
            model.event(EventLevel::Info, "é".repeat(2000));
        }
        assert_eq!(model.events.len(), EVENT_LIMIT);
        assert!(model.events[0].message.len() <= 2051);
    }
}
