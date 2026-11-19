use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use egui::{Color32, FontId, RichText, Stroke, Ui, pos2, vec2};

use crate::model::HISTORY_LIMIT;
use crate::{Command, ControlValue, DebugModel, EventLevel, Page, Statistics};

const BG: Color32 = Color32::from_rgb(11, 22, 21);
const PANEL: Color32 = Color32::from_rgb(16, 31, 29);
const RAIL: Color32 = Color32::from_rgb(13, 27, 26);
const LINE: Color32 = Color32::from_rgb(43, 64, 57);
const TEXT: Color32 = Color32::from_rgb(229, 238, 227);
const MUTED: Color32 = Color32::from_rgb(156, 178, 165);
const LIME: Color32 = Color32::from_rgb(192, 225, 136);
const TEAL: Color32 = Color32::from_rgb(99, 200, 184);
const AMBER: Color32 = Color32::from_rgb(232, 187, 112);
const RED: Color32 = Color32::from_rgb(244, 135, 126);

pub fn theme(ctx: &egui::Context) {
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = BG;
    style.visuals.faint_bg_color = PANEL;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = Color32::from_rgb(51, 76, 44);
    style.visuals.selection.stroke = Stroke::new(1.0, LIME);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(27, 47, 40);
    style.visuals.widgets.inactive.weak_bg_fill = PANEL;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(43, 67, 47);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(33, 55, 44);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, LIME);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(56, 80, 47);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, LIME);
    style.spacing.item_spacing = vec2(10.0, 6.0);
    style.spacing.button_padding = vec2(12.0, 8.0);
    style.spacing.interact_size.y = 28.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(24.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(13.0));
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

pub struct DebugUi {
    pub page: Page,
    filter: String,
    errors_only: bool,
    visible_only: bool,
    selected_entity: Option<String>,
    trace_count: usize,
    show_interval: bool,
    show_cpu: bool,
    export_dir: String,
    notice: Option<(bool, String)>,
}

impl Default for DebugUi {
    fn default() -> Self {
        Self {
            page: Page::Overview,
            filter: String::new(),
            errors_only: false,
            visible_only: false,
            selected_entity: None,
            trace_count: 300,
            show_interval: true,
            show_cpu: true,
            export_dir: "diagnostics-captures".into(),
            notice: None,
        }
    }
}

fn panel(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(7)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn muted(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).color(MUTED).size(12.0));
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(16.0).strong());
    ui.add_space(6.0);
}

impl DebugUi {
    pub fn show(&mut self, root: &mut Ui, model: &mut DebugModel, commands: &mut Vec<Command>) {
        egui::Panel::bottom("session")
            .exact_size(34.0)
            .frame(egui::Frame::new().fill(RAIL).inner_margin(8))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    muted(
                        ui,
                        format!(
                            "SESSION  {:02}:{:02}",
                            model.uptime() as u64 / 60,
                            model.uptime() as u64 % 60
                        ),
                    );
                    ui.separator();
                    muted(
                        ui,
                        format!("{} submitted / {} skipped", model.submitted, model.skipped),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        muted(ui, "Fate engine  /  local companion");
                    });
                });
            });
        egui::Panel::left("navigation")
            .exact_size(185.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(RAIL).inner_margin(16))
            .show(root, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    // A drawn mark, not an emoji or a missing icon font.
                    let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 5, LIME);
                    for n in 0..3 {
                        let y = r.top() + 8.0 + n as f32 * 6.0;
                        ui.painter().line_segment(
                            [
                                pos2(r.left() + 7.0, y),
                                pos2(r.right() - 7.0 - n as f32 * 4.0, y),
                            ],
                            Stroke::new(2.0, BG),
                        );
                    }
                    ui.label(RichText::new("Fate").size(23.0).strong());
                });
                ui.add_space(8.0);
                muted(ui, "Engine diagnostics");
                ui.add_space(28.0);
                for page in Page::ALL {
                    if matches!(page, Page::Simulation | Page::Camera)
                        && !model.inspection.facts.iter().any(|f| f.page == page)
                        && !model.inspection.controls.iter().any(|c| c.page == page)
                    {
                        continue;
                    }
                    if page == Page::Entities && model.inspection.entities.is_empty() {
                        continue;
                    }
                    if page == Page::Events {
                        ui.add_space(18.0);
                    }
                    let selected = page == self.page;
                    let label =
                        RichText::new(page.title()).color(if selected { LIME } else { MUTED });
                    if ui
                        .add_sized(
                            [ui.available_width(), 38.0],
                            egui::Button::new(label).selected(selected),
                        )
                        .clicked()
                    {
                        self.page = page;
                        self.filter.clear();
                    }
                }
                ui.add_space(24.0);
                ui.separator();
                muted(ui, "Source");
                if model.sources.len() > 1 {
                    egui::ComboBox::from_id_salt("source")
                        .selected_text(&model.source)
                        .width(145.0)
                        .show_ui(ui, |ui| {
                            for (index, name) in model.sources.iter().enumerate() {
                                ui.selectable_value(&mut model.selected_source, index, name);
                            }
                        });
                } else {
                    ui.label(&model.source);
                }
                muted(ui, "CPU + asynchronous GPU");
            });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(24))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(self.page.title());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(if model.frozen { "Resume trace" } else { "Freeze trace" }).clicked() {
                            model.frozen = !model.frozen;
                        }
                        ui.label(RichText::new(if model.frozen { "FROZEN" } else { "LIVE" })
                            .color(if model.frozen { AMBER } else { LIME }).size(12.0));
                    });
                });
                muted(ui, match self.page {
                    Page::Overview => "Live engine telemetry. CPU work and GPU execution are measured separately.",
                    Page::Renderer => "Submission, visibility, device capabilities and asynchronous pass timings.",
                    Page::Simulation => "Inspect deterministic state and operate the host's simulation tools.",
                    Page::Camera => "Edit the scene camera. Values are applied to the running consumer.",
                    Page::Entities => "Live render candidates with world-space position and frustum visibility.",
                    Page::Events => "Bounded host event history. Filter input, lifecycle, verification and tool results.",
                    Page::Capture => "Export the retained trace, inspection report or host simulation snapshot.",
                });
                ui.add_space(12.0);
                egui::ScrollArea::vertical().id_salt(("page", self.page.title())).auto_shrink([false, false]).show(ui, |ui| {
                    match self.page {
                        Page::Overview => self.overview(ui, model),
                        Page::Renderer => {
                            self.gpu_chart(ui, model);
                            ui.add_space(18.0);
                            self.inspector(ui, model, commands, Page::Renderer);
                        }
                        Page::Simulation | Page::Camera => self.inspector(ui, model, commands, self.page),
                        Page::Entities => self.entities(ui, model),
                        Page::Events => self.events(ui, model),
                        Page::Capture => self.capture(ui, model, commands),
                    }
                });
            });
    }

    fn overview(&mut self, ui: &mut Ui, model: &mut DebugModel) {
        let stats = Statistics::from_values(
            model
                .frames
                .iter()
                .map(|f| f.interval_ms)
                .filter(|v| *v > 0.0),
        );
        let cpu = Statistics::from_values(model.frames.iter().map(|f| f.cpu_ms()));
        let gpu = Statistics::from_values(model.gpu.iter().map(|g| g.scene_ms));
        let last = model.frames.back().copied().unwrap_or_default();
        let cards = [
            (
                "Frame cadence",
                if stats.mean > 0.0 {
                    format!("{:.1} fps", 1000.0 / stats.mean)
                } else {
                    "Waiting".into()
                },
                format!("p95  {:.2} ms", stats.p95),
            ),
            (
                "CPU work",
                format!("{:.2} ms", cpu.p50),
                if model.host_timings_available {
                    "p50 / includes verification".into()
                } else {
                    "p50 / renderer call only".into()
                },
            ),
            (
                "GPU scene",
                if gpu.count > 0 {
                    format!("{:.3} ms", gpu.p50)
                } else {
                    "No sample".into()
                },
                if model.gpu_available {
                    format!("{} completed samples", gpu.count)
                } else {
                    "Timestamp queries unavailable".into()
                },
            ),
            (
                "Draw submission",
                format!("{} / {}", last.draw_calls, last.instances),
                "draw calls / instances".into(),
            ),
        ];
        let columns = if ui.available_width() < 720.0 { 2 } else { 4 };
        for chunk in cards.chunks(columns) {
            ui.columns(columns, |cols| {
                for (col, (label, value, detail)) in cols.iter_mut().zip(chunk) {
                    panel(col, |ui| {
                        muted(ui, *label);
                        ui.label(RichText::new(value).size(24.0).color(TEXT));
                        muted(ui, detail);
                    });
                }
            });
            ui.add_space(12.0);
        }
        panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                heading(ui, "Frame history");
                ui.selectable_value(&mut self.trace_count, 120, "120");
                ui.selectable_value(&mut self.trace_count, 300, "300");
                ui.selectable_value(&mut self.trace_count, HISTORY_LIMIT, "1,200 frames");
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(
                    &mut self.show_interval,
                    RichText::new("Frame interval").color(LIME),
                );
                ui.checkbox(&mut self.show_cpu, RichText::new("CPU work").color(TEAL));
                ui.add(
                    egui::DragValue::new(&mut model.frame_budget_ms)
                        .range(1.0..=1000.0)
                        .speed(0.25)
                        .suffix(" ms")
                        .prefix("Budget marker "),
                );
            });
            let frames: Vec<_> = model
                .frames
                .iter()
                .rev()
                .take(self.trace_count)
                .rev()
                .collect();
            let interval: Vec<_> = frames.iter().map(|f| f.interval_ms).collect();
            let cpu: Vec<_> = frames.iter().map(|f| f.cpu_ms()).collect();
            let mut series = Vec::new();
            if self.show_interval {
                series.push(("Interval", LIME, interval.as_slice()));
            }
            if self.show_cpu {
                series.push(("CPU", TEAL, cpu.as_slice()));
            }
            if let Some(i) = chart(ui, &series, Some(model.frame_budget_ms), 175.0)
                && let Some(f) = frames.get(i)
            {
                muted(
                    ui,
                    format!(
                        "Frame {} / tick {}  |  interval {:.3} ms  |  CPU {:.3} ms  |  {} instances",
                        f.id,
                        f.tick,
                        f.interval_ms,
                        f.cpu_ms(),
                        f.instances
                    ),
                );
            } else {
                muted(
                    ui,
                    format!(
                        "{} retained frames  /  hover to inspect  /  oldest to newest",
                        frames.len()
                    ),
                );
            }
        });
        ui.add_space(18.0);
        if ui.available_width() < 650.0 {
            phase_breakdown(ui, model);
            ui.add_space(18.0);
            runtime_facts(ui, model);
        } else {
            ui.columns(2, |cols| {
                phase_breakdown(&mut cols[0], model);
                runtime_facts(&mut cols[1], model);
            });
        }
    }

    fn gpu_chart(&self, ui: &mut Ui, model: &DebugModel) {
        panel(ui, |ui| {
            heading(ui, "GPU pass history");
            if !model.gpu_available {
                ui.label("Timestamp queries are unavailable on the scene device.");
                muted(ui, "CPU telemetry and rendering remain active.");
            } else if model.gpu.is_empty() {
                ui.label("Waiting for completed GPU readbacks.");
            } else {
                let values: Vec<_> = model
                    .gpu
                    .iter()
                    .rev()
                    .take(self.trace_count)
                    .rev()
                    .map(|g| g.scene_ms)
                    .collect();
                let stats = Statistics::from_values(values.iter().copied());
                ui.label(
                    RichText::new(format!("{:.3} ms", stats.p50))
                        .size(28.0)
                        .color(LIME),
                );
                muted(
                    ui,
                    format!(
                        "Scene p50  /  p95 {:.3} ms  /  max {:.3} ms",
                        stats.p95, stats.max
                    ),
                );
                chart(ui, &[("GPU scene", LIME, &values)], None, 180.0);
                muted(
                    ui,
                    "Completed samples in order. Excludes acquire, present and gaps; not aligned to CPU frames.",
                );
            }
        });
    }

    fn inspector(&self, ui: &mut Ui, model: &DebugModel, commands: &mut Vec<Command>, page: Page) {
        let has_controls = model.inspection.controls.iter().any(|c| c.page == page);
        let facts = |ui: &mut Ui| {
            panel(ui, |ui| {
                heading(ui, "Inspection");
                ui.spacing_mut().interact_size.y = 20.0;
                let width = ui.available_width();
                egui::Grid::new(("facts", page.title()))
                    .num_columns(2)
                    .max_col_width((width * 0.57).max(140.0))
                    .spacing([16.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for fact in model.inspection.facts.iter().filter(|f| f.page == page) {
                            ui.add(
                                egui::Label::new(RichText::new(&fact.label).color(MUTED)).wrap(),
                            );
                            ui.add(egui::Label::new(RichText::new(&fact.value).monospace()).wrap());
                            ui.end_row();
                        }
                    });
            });
        };
        if has_controls && ui.available_width() >= 760.0 {
            ui.columns(2, |cols| {
                facts(&mut cols[0]);
                panel(&mut cols[1], |ui| {
                    heading(ui, "Tools");
                    controls(ui, model, page, commands);
                });
            });
        } else {
            if has_controls {
                panel(ui, |ui| {
                    heading(ui, "Tools");
                    controls(ui, model, page, commands);
                });
                ui.add_space(14.0);
            }
            facts(ui);
        }
    }

    fn entities(&mut self, ui: &mut Ui, model: &DebugModel) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter entity ID")
                    .desired_width(220.0),
            );
            ui.checkbox(&mut self.visible_only, "Visible only");
        });
        if let Some(id) = &self.selected_entity
            && let Some(entity) = model.inspection.entities.iter().find(|e| &e.id == id)
        {
            panel(ui, |ui| {
                heading(ui, &format!("Selected: {}", entity.id));
                ui.monospace(format!(
                    "Position  {:.4}, {:.4}, {:.4}",
                    entity.position[0], entity.position[1], entity.position[2]
                ));
                ui.label(if entity.visible {
                    "Intersects camera frustum"
                } else {
                    "Outside camera frustum"
                });
                if ui.button("Copy position").clicked() {
                    ui.ctx().copy_text(format!("{:?}", entity.position));
                }
            });
        }
        let filter = self.filter.to_lowercase();
        let rows: Vec<_> = model
            .inspection
            .entities
            .iter()
            .filter(|e| (!self.visible_only || e.visible) && e.id.to_lowercase().contains(&filter))
            .collect();
        muted(
            ui,
            format!(
                "{} of {} candidates",
                rows.len(),
                model.inspection.entities.len()
            ),
        );
        if rows.is_empty() {
            ui.label("No entities match this filter.");
        }
        let cell = ((ui.available_width() - 40.0) / 5.0).max(65.0);
        ui.horizontal(|ui| {
            for text in ["Entity", "World X", "World Y", "World Z", "Frustum"] {
                ui.add_sized(
                    [cell, 24.0],
                    egui::Label::new(RichText::new(text).color(MUTED)),
                );
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("entity-rows")
            .max_height(ui.available_height().max(240.0))
            .show_rows(ui, 28.0, rows.len(), |ui, range| {
                for index in range {
                    let entity = rows[index];
                    ui.push_id(&entity.id, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .add_sized(
                                    [cell, 28.0],
                                    egui::Button::new(&entity.id).selected(
                                        self.selected_entity.as_ref() == Some(&entity.id),
                                    ),
                                )
                                .clicked()
                            {
                                self.selected_entity = Some(entity.id.clone());
                            }
                            for v in entity.position {
                                ui.add_sized(
                                    [cell, 28.0],
                                    egui::Label::new(RichText::new(format!("{v:.3}")).monospace()),
                                );
                            }
                            ui.add_sized(
                                [cell, 28.0],
                                egui::Label::new(
                                    RichText::new(if entity.visible {
                                        "Visible"
                                    } else {
                                        "Culled"
                                    })
                                    .color(if entity.visible { LIME } else { MUTED }),
                                ),
                            );
                        });
                    });
                }
            });
    }

    fn events(&mut self, ui: &mut Ui, model: &mut DebugModel) {
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter events")
                    .desired_width(250.0),
            );
            ui.checkbox(&mut self.errors_only, "Warnings + errors");
            if ui.button("Clear events").clicked() {
                model.events.clear();
            }
        });
        let filter = self.filter.to_lowercase();
        let events: Vec<_> = model
            .events
            .iter()
            .rev()
            .filter(|e| {
                (!self.errors_only || e.level != EventLevel::Info)
                    && e.message.to_lowercase().contains(&filter)
            })
            .collect();
        muted(
            ui,
            format!(
                "{} matching events / newest first / hover for full message",
                events.len()
            ),
        );
        if events.is_empty() {
            ui.label("No events match this filter.");
        }
        egui::ScrollArea::vertical()
            .id_salt("event-rows")
            .max_height(ui.available_height().max(200.0))
            .show_rows(ui, 28.0, events.len(), |ui, range| {
                for index in range {
                    let event = events[index];
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{:8.2}s", event.seconds));
                        ui.colored_label(
                            match event.level {
                                EventLevel::Info => TEAL,
                                EventLevel::Warning => AMBER,
                                EventLevel::Error => RED,
                            },
                            event.level.label(),
                        );
                        ui.add(egui::Label::new(&event.message).truncate())
                            .on_hover_text(&event.message);
                    });
                }
            });
    }

    fn capture(&mut self, ui: &mut Ui, model: &mut DebugModel, commands: &mut Vec<Command>) {
        panel(ui, |ui| {
            heading(ui, "Trace capture");
            ui.label(format!(
                "{} CPU frames and {} GPU samples retained.",
                model.frames.len(),
                model.gpu.len()
            ));
            muted(
                ui,
                "Freeze trace holds both streams. Inspection and events remain live.",
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button("Clear trace").clicked() {
                    model.clear_trace();
                }
                if ui.button("Copy report").clicked() {
                    ui.ctx().copy_text(model.report());
                }
            });
            ui.add_space(8.0);
            ui.label("Export directory");
            ui.add(egui::TextEdit::singleline(&mut self.export_dir).desired_width(f32::INFINITY));
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !model.frames.is_empty(),
                        egui::Button::new("Export CPU CSV"),
                    )
                    .clicked()
                {
                    self.export(model, "cpu", "csv", model.frames_csv());
                }
                if ui
                    .add_enabled(!model.gpu.is_empty(), egui::Button::new("Export GPU CSV"))
                    .clicked()
                {
                    self.export(model, "gpu", "csv", model.gpu_csv());
                }
                if ui.button("Export report").clicked() {
                    self.export(model, "report", "txt", model.report());
                }
            });
            if let Some((ok, text)) = &self.notice {
                ui.colored_label(if *ok { LIME } else { RED }, text);
            }
        });
        ui.add_space(18.0);
        if model
            .inspection
            .controls
            .iter()
            .any(|c| c.page == Page::Capture)
        {
            panel(ui, |ui| {
                heading(ui, "Host capture tools");
                controls(ui, model, Page::Capture, commands);
            });
        }
        ui.add_space(18.0);
        panel(ui, |ui| {
            heading(ui, "Measurement guide");
            for text in [
                "Frame interval is wall time between scene frames, including pacing and companion overhead.",
                "CPU work = simulation + draw preparation + submit + verification.",
                "CPU recording is part of submit; do not add it a second time.",
                "GPU samples are asynchronous pass durations, not total frame time.",
                "The companion renders on a separate device. Opening it can change system load.",
                "History is bounded to 1,200 samples per stream and 500 events. Oldest entries are evicted.",
            ] {
                ui.label(text);
            }
        });
    }

    fn export(&mut self, model: &mut DebugModel, name: &str, extension: &str, contents: String) {
        let result = (|| -> std::io::Result<PathBuf> {
            let directory = PathBuf::from(self.export_dir.trim());
            if directory.as_os_str().is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Enter an export directory",
                ));
            }
            std::fs::create_dir_all(&directory)?;
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = directory.join(format!("fate-{name}-{stamp}.{extension}"));
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(contents.as_bytes())?;
            Ok(path)
        })();
        match result {
            Ok(path) => {
                let message = format!("Saved {}", path.display());
                model.event(EventLevel::Info, &message);
                self.notice = Some((true, message));
            }
            Err(error) => {
                let message = format!("Export failed: {error}. Check the directory and retry.");
                model.event(EventLevel::Error, &message);
                self.notice = Some((false, message));
            }
        }
    }
}

fn controls(ui: &mut Ui, model: &DebugModel, page: Page, commands: &mut Vec<Command>) {
    for control in model.inspection.controls.iter().filter(|c| c.page == page) {
        ui.push_id(control.id, |ui| {
            ui.add_enabled_ui(control.enabled, |ui| {
                let value = match control.value {
                    ControlValue::Action => ui
                        .button(control.label)
                        .clicked()
                        .then_some(ControlValue::Action),
                    ControlValue::Toggle(mut value) => ui
                        .checkbox(&mut value, control.label)
                        .changed()
                        .then_some(ControlValue::Toggle(value)),
                    ControlValue::Number {
                        mut value,
                        min,
                        max,
                        speed,
                    } => {
                        let changed = ui
                            .horizontal(|ui| {
                                let response = ui.add(
                                    egui::DragValue::new(&mut value)
                                        .range(min..=max)
                                        .speed(speed)
                                        .max_decimals(3),
                                );
                                ui.label(control.label);
                                response.changed()
                            })
                            .inner;
                        changed.then_some(ControlValue::Number {
                            value,
                            min,
                            max,
                            speed,
                        })
                    }
                };
                if let Some(value) = value {
                    commands.push(Command {
                        id: control.id,
                        value,
                    });
                }
            });
        });
    }
}

fn phase_breakdown(ui: &mut Ui, model: &DebugModel) {
    panel(ui, |ui| {
        heading(ui, "CPU phase breakdown");
        ui.spacing_mut().interact_size.y = 18.0;
        let f = model.frames.back().copied().unwrap_or_default();
        let phases = [
            ("Simulation", f.sim_ms),
            ("Draw preparation", f.prepare_ms),
            ("Submit + present", f.submit_ms),
            ("Verification", f.verify_ms),
        ];
        let total = f.cpu_ms().max(0.001);
        for (name, value) in phases {
            if !model.host_timings_available && name != "Submit + present" {
                continue;
            }
            ui.horizontal(|ui| {
                ui.label(name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.monospace(format!("{value:.3} ms"));
                });
            });
            let (r, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 4.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 2, LINE);
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    r.min,
                    vec2(r.width() * (value / total).clamp(0.0, 1.0), 4.0),
                ),
                2,
                TEAL,
            );
        }
        muted(
            ui,
            format!("Recording {:.3} ms (included in submit)", f.record_ms),
        );
    });
}

fn runtime_facts(ui: &mut Ui, model: &DebugModel) {
    panel(ui, |ui| {
        heading(ui, "Runtime / live");
        ui.spacing_mut().interact_size.y = 18.0;
        for fact in model
            .inspection
            .facts
            .iter()
            .filter(|f| f.page == Page::Overview)
        {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&fact.label).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.monospace(&fact.value);
                });
            });
        }
    });
}

/// Real sampled line chart with shared millisecond axis and hover cursor.
fn chart(
    ui: &mut Ui,
    series: &[(&str, Color32, &[f32])],
    budget: Option<f32>,
    height: f32,
) -> Option<usize> {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
    let plot = egui::Rect::from_min_max(rect.min + vec2(48.0, 10.0), rect.max - vec2(8.0, 20.0));
    let painter = ui.painter_at(rect);
    let max = series
        .iter()
        .flat_map(|(_, _, v)| v.iter().copied())
        .filter(|v| v.is_finite())
        .fold(budget.unwrap_or(0.0), f32::max)
        .max(0.01)
        * 1.15;
    for i in 0..=4 {
        let y = plot.bottom() - plot.height() * i as f32 / 4.0;
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0, LINE),
        );
        painter.text(
            pos2(plot.left() - 8.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.2}", max * i as f32 / 4.0),
            FontId::monospace(10.0),
            MUTED,
        );
    }
    let point = |i: usize, len: usize, value: f32| {
        pos2(
            plot.left() + plot.width() * i as f32 / len.saturating_sub(1).max(1) as f32,
            plot.bottom() - plot.height() * (value / max).clamp(0.0, 1.0),
        )
    };
    if let Some(budget) = budget {
        let y = point(0, 1, budget).y;
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0, AMBER),
        );
    }
    for (_, color, values) in series {
        let points: Vec<_> = values
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_finite())
            .map(|(i, v)| point(i, values.len(), *v))
            .collect();
        if points.len() >= 2 {
            painter.add(egui::Shape::line(points, Stroke::new(1.6, *color)));
        }
    }
    let len = series.iter().map(|s| s.2.len()).max().unwrap_or(0);
    if len == 0 {
        painter.text(
            plot.center(),
            egui::Align2::CENTER_CENTER,
            "Waiting for samples",
            FontId::proportional(14.0),
            MUTED,
        );
    }
    painter.text(
        pos2(plot.left(), rect.bottom()),
        egui::Align2::LEFT_BOTTOM,
        "older",
        FontId::proportional(11.0),
        MUTED,
    );
    painter.text(
        pos2(plot.right(), rect.bottom()),
        egui::Align2::RIGHT_BOTTOM,
        "newest / ms",
        FontId::proportional(11.0),
        MUTED,
    );
    if let Some(pointer) = response.hover_pos().filter(|p| plot.contains(*p))
        && len > 0
    {
        let i = (((pointer.x - plot.left()) / plot.width()) * len.saturating_sub(1) as f32).round()
            as usize;
        let x = point(i, len, 0.0).x;
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, MUTED),
        );
        response.on_hover_ui(|ui| {
            for (name, color, values) in series {
                if let Some(value) = values.get(i) {
                    ui.colored_label(*color, format!("{name}: {value:.3} ms"));
                }
            }
        });
        return Some(i);
    }
    None
}
