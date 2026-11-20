//! Engine renderer: owns the GPU context, the camera and the draw pipelines.

mod camera;
mod diagnostics;
mod frame;
mod gpu;
mod image_batch;
mod mesh;
mod offscreen;
mod overlay;
mod picking;
mod pipeline;
mod sprite;
#[cfg(test)]
mod test_support;
mod text;
mod texture;
mod tilemap;
mod timing;
mod ui;
mod visibility;

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::dpi::PhysicalSize;
use winit::window::Window;

pub use camera::{Camera2d, Camera3d};
pub use diagnostics::{RenderDiagnosticEvent, RenderDiagnosticReceiver, RenderFrameDiagnostics};
pub use gpu::{Gpu, GpuInitError};
pub use mesh::{
    DepthFog3d, Draw3d, FlatLight3d, Mesh3d, MeshCreateError, MeshVertex, NormalMeshVertex,
    SceneDraw3d, UvMeshVertex,
};
pub use offscreen::{CapturedFrame, OffscreenError, OffscreenRenderer, OffscreenScene};
pub use picking::{
    MeshHit3d, PickInstance3d, PickMesh3d, PickQueryStats, PickRay3d, PickScene3d, PickSceneHit3d,
    PickSceneStats,
};
pub use sprite::Sprite2d;
pub use text::{TextDraw, text_width, wrap_text};
pub use texture::{
    Texture2d, TextureAddress, TextureCreateError, TextureFilter, TextureOptions, TextureRegion,
    TextureUpdateError,
};
pub use tilemap::{MAX_DRAWN_TILES, MAX_TILE_LAYERS, TileLayer2d, TileMap2d, TileMapError};
pub use timing::{GpuFrameKind, GpuFrameTiming};
pub use ui::{MAX_UI_IMAGES, MAX_UI_RECTS, Overlay, UiClip, UiImage, UiRect};
pub use visibility::{Bounds2d, Bounds3d, Frustum3d, OrientedBounds2d};

use crate::math::{Mat4, Vec2};
use frame::{FrameTarget, RecordedFrame, Scene2d, ScenePipelines, depth_target};
use timing::GpuTimer;

pub const MAX_DRAWN_INSTANCES: usize = pipeline::MAX_INSTANCES;
const DRAW_WARNING_THRESHOLD: f32 = 0.75;

/// Optional renderer capabilities. Defaults leave GPU timing disabled.
#[derive(Clone, Copy, Debug, Default)]
pub struct RendererOptions {
    /// Request asynchronous GPU pass timestamps when supported by the adapter.
    pub gpu_timing: bool,
}

/// CPU-side report for a submitted 3D frame. Overlay draws are not counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshSubmission {
    /// Mesh instances submitted after applying the draw budget.
    pub instances: usize,
    /// Indexed draw calls recorded after combining adjacent compatible instances.
    pub draw_calls: usize,
    /// CPU time to prepare uploads and record commands, including the overlay.
    /// Excludes surface acquisition, queue submission, presentation, and GPU time.
    pub cpu_record_time: Duration,
}

/// A drawable instance of the built-in mesh, in world space.
#[derive(Clone, Copy, Debug, Default)]
pub struct Instance2d {
    pub position: Vec2,
    pub rotation: f32,
    pub scale: f32,
}

impl Instance2d {
    pub fn model_matrix(&self) -> Mat4 {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        Mat4::translation(crate::math::Vec3::new(
            self.position.x,
            self.position.y,
            0.0,
        ))
        .mul(Mat4::rotation_z(self.rotation))
        .mul(Mat4::scale(crate::math::Vec3::new(scale, scale, 1.0)))
    }
}

pub struct Renderer {
    pub gpu: Gpu,
    pub camera: Camera2d,
    #[allow(dead_code)]
    pub camera_3d: Camera3d,
    /// Optional directional light for opaque 3D meshes. Defaults to unlit.
    pub mesh_light: Option<FlatLight3d>,
    /// Optional depth fog and background color for 3D scenes. Defaults to off.
    pub mesh_fog: Option<DepthFog3d>,
    pipelines: ScenePipelines,
    gpu_timer: Option<GpuTimer>,
    diagnostic_sender: Option<diagnostics::RenderDiagnosticSender>,
    mirrored_timings: std::collections::VecDeque<GpuFrameTiming>,
    depth_view: wgpu::TextureView,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Result<Self, GpuInitError> {
        Self::new_with_options(window, RendererOptions::default()).await
    }

    pub async fn new_with_options(
        window: Arc<Window>,
        options: RendererOptions,
    ) -> Result<Self, GpuInitError> {
        let gpu = Gpu::new_with_timestamps(window, options.gpu_timing).await?;
        let pipelines = ScenePipelines::new(&gpu.device, &gpu.queue, gpu.config.format);
        let depth_view = depth_target(&gpu.device, (gpu.config.width, gpu.config.height));
        let gpu_timer = if options.gpu_timing {
            GpuTimer::new(&gpu.device, &gpu.queue)
        } else {
            None
        };

        Ok(Self {
            gpu,
            camera: Camera2d::default(),
            camera_3d: Camera3d::default(),
            mesh_light: None,
            mesh_fog: None,
            pipelines,
            gpu_timer,
            diagnostic_sender: None,
            mirrored_timings: std::collections::VecDeque::new(),
            depth_view,
        })
    }

    /// Whether GPU timing was requested and is available on this device.
    pub fn gpu_timing_available(&self) -> bool {
        self.gpu_timer.is_some()
    }

    /// Drain completed GPU samples without waiting for in-flight submissions.
    ///
    /// Returns an empty list when disabled, unsupported, or not ready. Call
    /// regularly to free readback slots; frames are not sampled when the
    /// bounded pool is full.
    pub fn poll_gpu_timings(&mut self) -> Vec<GpuFrameTiming> {
        let mut ready: Vec<_> = self.mirrored_timings.drain(..).collect();
        let completed = self
            .gpu_timer
            .as_mut()
            .map_or_else(Vec::new, |timer| timer.poll(&self.gpu.device));
        if let Some(sender) = &self.diagnostic_sender {
            for &sample in &completed {
                sender.send(RenderDiagnosticEvent::Gpu(sample));
            }
        }
        ready.extend(completed);
        ready
    }

    /// Subscribe to automatic 2D/3D render telemetry. Replaces any previous
    /// subscription. No presentation dependency is introduced into the engine.
    ///
    /// GPU samples are forwarded when timing was enabled at creation. The
    /// normal `poll_gpu_timings` API remains usable, retaining up to 1,200
    /// mirrored samples while a subscription is active.
    pub fn subscribe_diagnostics(&mut self) -> RenderDiagnosticReceiver {
        let (sender, receiver) = diagnostics::channel();
        self.diagnostic_sender = Some(sender);
        receiver
    }

    fn diagnostic_start(&mut self) -> Option<(Instant, Option<Duration>, u64)> {
        let sender = self.diagnostic_sender.as_mut()?;
        let now = Instant::now();
        let interval = sender
            .last_start
            .replace(now)
            .map(|last| now.duration_since(last));
        let id = sender.next_id;
        sender.next_id = sender.next_id.wrapping_add(1);
        Some((now, interval, id))
    }

    fn diagnostic_finish(
        &mut self,
        probe: Option<(Instant, Option<Duration>, u64)>,
        kind: GpuFrameKind,
        requested: usize,
        submission: Option<MeshSubmission>,
    ) {
        let Some((start, interval, frame_id)) = probe else {
            return;
        };
        let Some(sender) = &self.diagnostic_sender else {
            return;
        };
        let connected = sender.send(RenderDiagnosticEvent::Frame(RenderFrameDiagnostics {
            frame_id,
            kind,
            interval,
            render_time: start.elapsed(),
            record_time: submission.map_or(Duration::ZERO, |s| s.cpu_record_time),
            requested_instances: requested,
            submitted_instances: submission.map_or(0, |s| s.instances),
            draw_calls: submission.map_or(0, |s| s.draw_calls),
            submitted: submission.is_some(),
            surface_size: self.surface_size(),
        }));
        if !connected {
            self.diagnostic_sender = None;
            self.mirrored_timings.clear();
            return;
        }
        if let Some(timer) = &mut self.gpu_timer {
            for timing in timer.poll(&self.gpu.device) {
                sender.send(RenderDiagnosticEvent::Gpu(timing));
                if self.mirrored_timings.len() == 1200 {
                    self.mirrored_timings.pop_front();
                }
                self.mirrored_timings.push_back(timing);
            }
        }
    }

    pub fn budgeted_instance_count_static(instance_count: usize) -> usize {
        let limit = MAX_DRAWN_INSTANCES;
        let count = instance_count.min(limit);
        let warning_threshold = (limit as f32 * DRAW_WARNING_THRESHOLD) as usize;

        if instance_count > limit {
            log::warn!(
                "render instance budget exceeded: {instance_count} visible objects > {limit} max draw instances; clipping to {count} for the current frame",
            );
        } else if instance_count >= warning_threshold {
            log::warn!(
                "render instance budget nearing cap: {instance_count}/{limit} visible objects; draw pressure may reduce frame stability",
            );
        }

        count
    }

    pub fn budgeted_instance_count(&self, instance_count: usize) -> usize {
        Self::budgeted_instance_count_static(instance_count)
    }

    pub fn max_instance_budget() -> usize {
        MAX_DRAWN_INSTANCES
    }

    /// Aspect ratio of the current surface configuration.
    ///
    /// Consumers that build visibility bounds or frusta need this; it avoids
    /// reaching into [`Gpu`] for the common case.
    pub fn aspect(&self) -> f32 {
        self.gpu.aspect()
    }

    /// Current drawable size in physical pixels.
    ///
    /// Text positions and sizes are expressed in these units, so consumers use
    /// this to lay out screen-anchored HUD elements.
    pub fn surface_size(&self) -> (u32, u32) {
        (self.gpu.config.width, self.gpu.config.height)
    }

    /// Upload a mesh through the renderer's device.
    ///
    /// This is the backend-neutral path for consumers that do not want to name
    /// `wgpu::Device`. The lower-level [`Mesh3d::new`] remains available for
    /// headless tools that create their own device.
    pub fn create_mesh(
        &self,
        label: &str,
        vertices: &[MeshVertex],
        indices: &[u32],
    ) -> Result<Mesh3d, MeshCreateError> {
        Mesh3d::new(&self.gpu.device, label, vertices, indices)
    }

    /// Upload indexed triangles with per-vertex texture coordinates. Non-finite
    /// UVs are rejected; finite UVs may extend beyond the texture bounds.
    pub fn create_mesh_with_uvs(
        &self,
        label: &str,
        vertices: &[UvMeshVertex],
        indices: &[u32],
    ) -> Result<Mesh3d, MeshCreateError> {
        Mesh3d::new_with_uvs(&self.gpu.device, label, vertices, indices)
    }

    /// Upload indexed triangles with UVs and smooth-shading normals.
    /// Zero and non-finite normals are rejected.
    pub fn create_mesh_with_normals(
        &self,
        label: &str,
        vertices: &[NormalMeshVertex],
        indices: &[u32],
    ) -> Result<Mesh3d, MeshCreateError> {
        Mesh3d::new_with_normals(&self.gpu.device, label, vertices, indices)
    }

    /// Upload the built-in unit cube through the renderer's device.
    pub fn cube_mesh(&self) -> Result<Mesh3d, MeshCreateError> {
        cube_mesh(&self.gpu.device)
    }

    /// Upload an RGBA8 sRGB texture through the renderer's device.
    ///
    /// `rgba8` must contain exactly `width * height * 4` bytes. This is a raw
    /// upload primitive: decoding files, atlases, mipmaps, and lifetime policy
    /// remain consumer decisions. Draw the result with a [`Sprite2d`],
    /// [`UiImage`], or [`Draw3d::with_texture`].
    pub fn create_texture(
        &self,
        label: &str,
        width: u32,
        height: u32,
        rgba8: &[u8],
    ) -> Result<Texture2d, TextureCreateError> {
        self.create_texture_with_options(label, width, height, rgba8, TextureOptions::default())
    }

    /// Upload an RGBA8 sRGB texture with explicit sampling behavior.
    ///
    /// Use [`TextureOptions::NEAREST`] for crisp pixel art and
    /// [`TextureOptions::NEAREST_REPEAT`] (or a repeat address mode) with a
    /// [`TextureRegion`] wider than `1.0` to tile an image. Validation and
    /// lifetime rules match [`Renderer::create_texture`].
    pub fn create_texture_with_options(
        &self,
        label: &str,
        width: u32,
        height: u32,
        rgba8: &[u8],
        options: TextureOptions,
    ) -> Result<Texture2d, TextureCreateError> {
        Texture2d::new(
            &self.gpu.device,
            &self.gpu.queue,
            &self.pipelines.texture_layout,
            texture::TextureUpload {
                label,
                width,
                height,
                rgba8,
                options,
            },
        )
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        let previous_size = (self.gpu.config.width, self.gpu.config.height);
        self.gpu.resize(size);
        if size.width > 0
            && size.height > 0
            && previous_size != (self.gpu.config.width, self.gpu.config.height)
        {
            self.depth_view = depth_target(&self.gpu.device, self.surface_size());
        }
    }

    /// Upload and draw indexed meshes using the perspective camera and a depth
    /// buffer. Meshes and transforms remain owned by the caller.
    pub fn render_meshes(&mut self, draws: &[Draw3d<'_>]) -> bool {
        self.render_meshes_with_text(draws, &[])
    }

    /// Draw indexed meshes, then a text overlay on top of them.
    ///
    /// Both stages share one surface acquisition and one submission. Text
    /// positions are in physical pixels from the top-left of the window, so
    /// consumers can anchor a HUD to [`Renderer::surface_size`]. Use
    /// [`Renderer::render_meshes_with_overlay`] to add panels behind the text.
    pub fn render_meshes_with_text(&mut self, draws: &[Draw3d<'_>], text: &[TextDraw<'_>]) -> bool {
        self.render_meshes_with_overlay(
            draws,
            &Overlay {
                text,
                ..Default::default()
            },
        )
    }

    /// Draw indexed meshes, then the screen-space [`Overlay`] on top.
    ///
    /// All stages share one surface acquisition and one submission. Overlay
    /// positions are in physical pixels from the top-left of the window, so
    /// consumers can anchor a HUD to [`Renderer::surface_size`]. The layers
    /// draw in order: rectangles, images, then text. They are alpha blended and
    /// skip depth testing, so a translucent panel behind a label just goes
    /// first. At most [`MAX_UI_RECTS`] rectangles and [`MAX_UI_IMAGES`] images
    /// are submitted.
    pub fn render_meshes_with_overlay(
        &mut self,
        draws: &[Draw3d<'_>],
        overlay: &Overlay<'_>,
    ) -> bool {
        self.render_meshes_with_overlay_report(draws, overlay)
            .is_some()
    }

    /// Like [`Self::render_meshes_with_overlay`], returning submission counters
    /// and CPU recording time. Returns `None` when the surface skipped the frame.
    pub fn render_meshes_with_overlay_report(
        &mut self,
        draws: &[Draw3d<'_>],
        overlay: &Overlay<'_>,
    ) -> Option<MeshSubmission> {
        self.render_scene_inner(draws.iter().copied().map(SceneDraw3d::from), overlay)
    }

    /// Draw a mixed list of opaque textured and untextured meshes, then the
    /// screen-space overlay, in one submission. All meshes share a depth
    /// buffer and draw budget. A texture's RGB multiplies vertex color and
    /// draw tint; its alpha does not affect the opaque mesh.
    pub fn render_scene(&mut self, draws: &[SceneDraw3d<'_>], overlay: &Overlay<'_>) -> bool {
        self.render_scene_inner(draws.iter().copied(), overlay)
            .is_some()
    }

    fn render_scene_inner<'a>(
        &mut self,
        draws: impl ExactSizeIterator<Item = SceneDraw3d<'a>> + Clone,
        overlay: &Overlay<'_>,
    ) -> Option<MeshSubmission> {
        let probe = self.diagnostic_start();
        let requested = draws.len();
        let Some(frame) = self.gpu.acquire_frame() else {
            self.diagnostic_finish(probe, GpuFrameKind::Meshes3d, requested, None);
            return None;
        };
        let record_start = Instant::now();
        let timing = self
            .gpu_timer
            .as_ref()
            .and_then(|timer| timer.free_slot().map(|slot| (timer, slot)));

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let RecordedFrame {
            mut encoder,
            instances,
            draw_calls,
            has_overlay,
        } = self.pipelines.record_3d(
            FrameTarget {
                device: &self.gpu.device,
                queue: &self.gpu.queue,
                size: (self.gpu.config.width, self.gpu.config.height),
                color: &view,
                depth: Some(&self.depth_view),
                view_projection: self.camera_3d.view_projection(self.gpu.aspect()),
            },
            self.mesh_light,
            self.mesh_fog,
            draws,
            overlay,
            timing.map(|(timer, _)| timer),
        );

        if let Some((timer, slot)) = timing {
            timer.resolve(&mut encoder, slot, has_overlay);
        }
        let commands = encoder.finish();
        let cpu_record_time = record_start.elapsed();
        self.gpu.queue.submit(Some(commands));
        if let Some(slot) = timing.map(|(_, slot)| slot) {
            self.gpu_timer.as_mut().expect("timing enabled").submitted(
                slot,
                GpuFrameKind::Meshes3d,
                has_overlay,
            );
        }
        self.gpu.queue.present(frame);
        let submission = MeshSubmission {
            instances,
            draw_calls,
            cpu_record_time,
        };
        self.diagnostic_finish(probe, GpuFrameKind::Meshes3d, requested, Some(submission));
        Some(submission)
    }

    /// Draw the built-in 2D triangles, in input order, with one instanced draw.
    /// An empty list still clears and presents. Returns `false` when the
    /// surface was not ready (minimized, timed out, lost).
    pub fn render(&mut self, instances: &[Instance2d]) -> bool {
        self.render_with_overlay(instances, &Overlay::default())
    }

    /// Draw the built-in 2D triangles, then rectangles, images, and text in
    /// physical pixels, with one surface acquisition and queue submission.
    ///
    /// An empty instance list is valid and renders the overlay over the clear
    /// color. Scene draw counters exclude overlay work. With GPU timing enabled,
    /// the overlay pass is reported separately when recorded.
    pub fn render_with_overlay(&mut self, instances: &[Instance2d], overlay: &Overlay<'_>) -> bool {
        self.render_2d(Scene2d::Triangles(instances), overlay)
    }

    /// Draw world-space sprites through [`Camera2d`], then the screen-space overlay.
    ///
    /// Alpha-blended sprites are drawn in input order. Adjacent sprites using
    /// the same texture share a draw, even when atlas regions or tints differ.
    /// Invalid sprites are skipped before the scene instance limit is applied.
    /// An empty list clears the scene and still presents the overlay.
    pub fn render_sprites(&mut self, sprites: &[Sprite2d<'_>], overlay: &Overlay<'_>) -> bool {
        self.render_2d(Scene2d::Sprites(sprites), overlay)
    }

    /// Draw one atlas-backed tile layer through [`Camera2d`], then the HUD.
    ///
    /// This is a one-layer convenience wrapper around
    /// [`Self::render_tile_layers_with_sprites`].
    pub fn render_tiles(&mut self, layer: TileLayer2d<'_>, overlay: &Overlay<'_>) -> bool {
        self.render_tile_layers(std::slice::from_ref(&layer), overlay)
    }

    /// Draw ordered atlas-backed tile layers through [`Camera2d`], then the HUD.
    ///
    /// Layers composite in slice order, up to [`MAX_TILE_LAYERS`]. Only cells
    /// covered by the camera are visited, and each layer submits at most
    /// [`MAX_DRAWN_TILES`] visible tiles in row-major order. A layer's
    /// [`TileLayer2d::parallax`] scales its camera travel, and its
    /// [`TileLayer2d::offset`] adds a world-space shift on top.
    pub fn render_tile_layers(
        &mut self,
        layers: &[TileLayer2d<'_>],
        overlay: &Overlay<'_>,
    ) -> bool {
        self.render_tile_layers_with_sprites(layers, &[], overlay)
    }

    /// Draw tile layers, then world-space sprites, then the screen-space HUD
    /// in one submission. Tile and sprite limits apply independently.
    pub fn render_tiles_with_sprites(
        &mut self,
        layer: TileLayer2d<'_>,
        sprites: &[Sprite2d<'_>],
        overlay: &Overlay<'_>,
    ) -> bool {
        self.render_tile_layers_with_sprites(std::slice::from_ref(&layer), sprites, overlay)
    }

    /// Draw ordered tile layers, then world-space sprites, then the HUD.
    ///
    /// Sprites draw above every tile layer. Each layer has its own visible-tile
    /// budget, and sprites use the ordinary [`MAX_DRAWN_INSTANCES`] budget.
    pub fn render_tile_layers_with_sprites(
        &mut self,
        layers: &[TileLayer2d<'_>],
        sprites: &[Sprite2d<'_>],
        overlay: &Overlay<'_>,
    ) -> bool {
        self.render_2d(
            Scene2d::Tiles {
                layers,
                sprites,
                view: self.camera.visible_bounds(self.aspect()),
            },
            overlay,
        )
    }

    fn render_2d(&mut self, scene: Scene2d<'_>, overlay: &Overlay<'_>) -> bool {
        let (kind, requested) = match scene {
            Scene2d::Triangles(items) => (GpuFrameKind::Triangles2d, items.len()),
            Scene2d::Sprites(items) => (GpuFrameKind::Sprites2d, items.len()),
            Scene2d::Tiles {
                layers, sprites, ..
            } => (
                GpuFrameKind::TileMap2d,
                layers
                    .iter()
                    .take(MAX_TILE_LAYERS)
                    .fold(sprites.len(), |total, layer| {
                        total.saturating_add(layer.map.cell_count())
                    }),
            ),
        };
        let probe = self.diagnostic_start();
        let Some(frame) = self.gpu.acquire_frame() else {
            self.diagnostic_finish(probe, kind, requested, None);
            return false;
        };
        let record_start = probe.map(|_| Instant::now());
        let timing = self
            .gpu_timer
            .as_ref()
            .and_then(|timer| timer.free_slot().map(|slot| (timer, slot)));

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let RecordedFrame {
            mut encoder,
            instances,
            draw_calls,
            has_overlay,
        } = self.pipelines.record_2d(
            FrameTarget {
                device: &self.gpu.device,
                queue: &self.gpu.queue,
                size: (self.gpu.config.width, self.gpu.config.height),
                color: &view,
                depth: None,
                view_projection: self.camera.view_projection(self.gpu.aspect()),
            },
            scene,
            overlay,
            timing.map(|(timer, _)| timer),
        );
        if let Some((timer, slot)) = timing {
            timer.resolve(&mut encoder, slot, has_overlay);
        }
        let commands = encoder.finish();
        let cpu_record_time = record_start.map_or(Duration::ZERO, |start| start.elapsed());
        self.gpu.queue.submit(Some(commands));
        if let Some(slot) = timing.map(|(_, slot)| slot) {
            self.gpu_timer
                .as_mut()
                .expect("timing enabled")
                .submitted(slot, kind, has_overlay);
        }
        self.gpu.queue.present(frame);
        self.diagnostic_finish(
            probe,
            kind,
            requested,
            Some(MeshSubmission {
                instances,
                draw_calls,
                cpu_record_time,
            }),
        );
        true
    }
}

pub fn cube_mesh(device: &wgpu::Device) -> Result<Mesh3d, MeshCreateError> {
    let vertices = [
        MeshVertex {
            position: [-0.5, -0.5, -0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [0.5, -0.5, -0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [0.5, 0.5, -0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [-0.5, 0.5, -0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [-0.5, -0.5, 0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [0.5, -0.5, 0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [0.5, 0.5, 0.5],
            color: [0.75, 0.82, 1.0],
        },
        MeshVertex {
            position: [-0.5, 0.5, 0.5],
            color: [0.75, 0.82, 1.0],
        },
    ];
    let indices: [u32; 36] = [
        0, 2, 1, 0, 3, 2, // back
        4, 5, 6, 4, 6, 7, // front
        0, 1, 5, 0, 5, 4, // bottom
        3, 7, 6, 3, 6, 2, // top
        1, 2, 6, 1, 6, 5, // right
        0, 4, 7, 0, 7, 3, // left
    ];
    Mesh3d::new(device, "unit cube", &vertices, &indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgeted_instance_count_clips_beyond_render_limit() {
        assert_eq!(Renderer::budgeted_instance_count_static(10), 10);
        assert_eq!(
            Renderer::budgeted_instance_count_static(MAX_DRAWN_INSTANCES + 7),
            MAX_DRAWN_INSTANCES
        );
        assert_eq!(
            Renderer::budgeted_instance_count_static((MAX_DRAWN_INSTANCES * 3) / 4),
            (MAX_DRAWN_INSTANCES * 3) / 4
        );
    }
}
