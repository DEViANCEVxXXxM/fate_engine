use std::time::{Duration, Instant};

use fate_engine::renderer::{
    GpuFrameKind, MAX_DRAWN_TILES, RenderDiagnosticEvent, RenderDiagnosticReceiver, Renderer,
    RendererOptions,
};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

use crate::{
    Command, DebugModel, DebugWindow, EventLevel, Fact, FrameSample, GpuSample, LogInbox, Page,
};

struct Source {
    name: String,
    receiver: RenderDiagnosticReceiver,
    facts: Vec<Fact>,
    gpu: bool,
}

/// Reusable lifecycle/telemetry adapter for any winit engine consumer.
///
/// Attach each renderer once; all existing render entry points then publish
/// counters automatically. Forward window events before the host dispatches
/// them, and call `update` from `about_to_wait` even when unfocused.
/// Application-specific inspection/commands are optional via `model_mut` and
/// `take_commands`. The adapter never synthesizes focus or input for the host.
pub struct DebugCompanion {
    enabled: bool,
    window: Option<DebugWindow>,
    model: DebugModel,
    sources: Vec<Source>,
    selected: usize,
    commands: Vec<Command>,
    next_refresh: Instant,
    logs: Option<LogInbox>,
}

impl DebugCompanion {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            window: None,
            model: DebugModel::default(),
            sources: Vec::new(),
            selected: 0,
            commands: Vec::new(),
            next_refresh: Instant::now(),
            logs: None,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn renderer_options(&self) -> RendererOptions {
        RendererOptions {
            gpu_timing: self.enabled,
        }
    }

    pub fn model(&self) -> &DebugModel {
        &self.model
    }
    pub fn model_mut(&mut self) -> &mut DebugModel {
        &mut self.model
    }
    pub fn set_logs(&mut self, inbox: LogInbox) {
        self.logs = Some(inbox);
    }
    pub fn is_open(&self) -> bool {
        self.window.is_some()
    }

    /// Returns no deadline when disabled/closed. Use this to wake an otherwise
    /// idle host loop; background diagnostics refresh at most 20 times/second.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.window.as_ref().map(|_| self.next_refresh)
    }

    pub async fn open(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.enabled && self.window.is_none() {
            self.window = Some(DebugWindow::new(event_loop).await?);
            self.model
                .event(EventLevel::Info, "Diagnostics companion opened");
            self.next_refresh = Instant::now();
        }
        Ok(())
    }

    /// Replace all previous attachments to this named source, e.g. after a host
    /// recreates its renderer. Different names appear in the source selector.
    pub fn attach_renderer(&mut self, name: impl Into<String>, renderer: &mut Renderer) {
        if !self.enabled {
            return;
        }
        let name = name.into();
        let info = renderer.gpu.adapter.get_info();
        let facts = [
            ("Adapter", info.name),
            (
                "Backend / device",
                format!("{:?} / {:?}", info.backend, info.device_type),
            ),
            ("Driver", format!("{} {}", info.driver, info.driver_info)),
            ("Format", format!("{:?}", renderer.gpu.config.format)),
            (
                "Presentation",
                format!("{:?}", renderer.gpu.config.present_mode),
            ),
            (
                "Non-tile instance limit",
                Renderer::max_instance_budget().to_string(),
            ),
            ("Visible tile limit", MAX_DRAWN_TILES.to_string()),
            (
                "Max texture dimension",
                renderer
                    .gpu
                    .device
                    .limits()
                    .max_texture_dimension_2d
                    .to_string(),
            ),
        ]
        .into_iter()
        .map(|(label, value)| Fact {
            page: Page::Renderer,
            label: label.into(),
            value,
        })
        .collect();
        let source = Source {
            name: name.clone(),
            receiver: renderer.subscribe_diagnostics(),
            facts,
            gpu: renderer.gpu_timing_available(),
        };
        if let Some(index) = self.sources.iter().position(|s| s.name == name) {
            self.sources[index] = source;
        } else {
            self.sources.push(source);
        }
        self.model.sources = self.sources.iter().map(|s| s.name.clone()).collect();
        self.refresh_source();
        self.model
            .event(EventLevel::Info, format!("Attached renderer: {name}"));
    }

    fn refresh_source(&mut self) {
        if let Some(source) = self.sources.get(self.selected) {
            self.model.source.clone_from(&source.name);
            self.model.gpu_available = source.gpu;
            self.model
                .inspection
                .facts
                .retain(|f| f.page != Page::Renderer);
            self.model.inspection.facts.extend(source.facts.clone());
        }
    }

    /// True means the companion consumed this event. In particular, its close
    /// button must never be forwarded to a host that exits on any CloseRequested.
    pub fn on_window_event(&mut self, id: WindowId, event: &WindowEvent) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(window) = &mut self.window
            && window.id() == id
        {
            if window.on_event(event) {
                self.window = None;
                self.model.event(
                    EventLevel::Info,
                    "Companion closed; telemetry remains bounded",
                );
            } else if matches!(event, WindowEvent::RedrawRequested) {
                let commands = window.draw(&mut self.model);
                if self.commands.len() + commands.len() <= 128 {
                    self.commands.extend(commands);
                } else {
                    self.model.event(
                        EventLevel::Warning,
                        "Host command queue full; drain take_commands() before retrying",
                    );
                }
            }
            return true;
        }
        let description = match event {
            WindowEvent::Focused(value) => Some(format!("Host {id:?} focused: {value}")),
            WindowEvent::Resized(size) => Some(format!(
                "Host {id:?} resized: {} x {}",
                size.width, size.height
            )),
            WindowEvent::KeyboardInput { event, .. } if !event.repeat => Some(format!(
                "Host input: {:?} {:?}",
                event.physical_key, event.state
            )),
            WindowEvent::MouseInput { button, state, .. } => {
                Some(format!("Host input: {button:?} {state:?}"))
            }
            _ => None,
        };
        if let Some(description) = description {
            self.model.event(EventLevel::Info, description);
        }
        false
    }

    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    pub fn update(&mut self) {
        if !self.enabled {
            return;
        }
        if let Some(logs) = &self.logs {
            logs.drain_into(&mut self.model);
        }
        if self.model.selected_source != self.selected
            && self.model.selected_source < self.sources.len()
        {
            self.selected = self.model.selected_source;
            self.model.clear_trace();
            self.model.submitted = 0;
            self.model.skipped = 0;
            self.refresh_source();
        }
        for (index, source) in self.sources.iter().enumerate() {
            for event in source.receiver.drain() {
                if index != self.selected {
                    continue;
                }
                match event {
                    RenderDiagnosticEvent::Frame(frame) => {
                        self.model.record_frame(FrameSample {
                            interval_ms: frame.interval.map_or(0.0, |d| d.as_secs_f32() * 1000.0),
                            submit_ms: frame.render_time.as_secs_f32() * 1000.0,
                            record_ms: frame.record_time.as_secs_f32() * 1000.0,
                            instances: frame.submitted_instances,
                            draw_calls: frame.draw_calls,
                            visible: frame.requested_instances,
                            candidates: frame.requested_instances,
                            submitted: frame.submitted,
                            ..Default::default()
                        });
                        self.model
                            .inspection
                            .facts
                            .retain(|f| f.page != Page::Overview);
                        for (label, value) in [
                            ("Render path", format!("{:?}", frame.kind)),
                            (
                                "Surface",
                                format!("{} x {}", frame.surface_size.0, frame.surface_size.1),
                            ),
                            (
                                if frame.kind == GpuFrameKind::TileMap2d {
                                    "Map cells + sprites"
                                } else {
                                    "Requested instances"
                                },
                                frame.requested_instances.to_string(),
                            ),
                            ("Submitted instances", frame.submitted_instances.to_string()),
                            ("Draw calls", frame.draw_calls.to_string()),
                            (
                                "GPU timing",
                                if source.gpu {
                                    "Available"
                                } else {
                                    "Unavailable"
                                }
                                .into(),
                            ),
                        ] {
                            self.model.inspection.fact(Page::Overview, label, value);
                        }
                    }
                    RenderDiagnosticEvent::Gpu(sample) => self.model.record_gpu(GpuSample {
                        id: sample.sample_id,
                        scene_ms: sample.scene.as_secs_f32() * 1000.0,
                        overlay_ms: sample.overlay.map(|d| d.as_secs_f32() * 1000.0),
                    }),
                }
            }
            let dropped = source.receiver.take_dropped();
            if dropped > 0 {
                self.model.event(
                    EventLevel::Warning,
                    format!(
                        "{}: {dropped} telemetry events dropped; drain more often",
                        source.name
                    ),
                );
            }
        }
        if Instant::now() >= self.next_refresh {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            self.next_refresh = Instant::now() + Duration::from_millis(50);
        }
    }
}
