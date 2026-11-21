//! Windowless scene capture into an RGBA8 sRGB image.

use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::Duration;

use super::frame::{FrameTarget, RecordedFrame, Scene2d, ScenePipelines, depth_target};
use super::texture::TextureUpload;
use super::timing::GpuTimer;
use super::{
    Camera2d, Camera3d, DepthFog3d, FlatLight3d, GpuFrameKind, GpuFrameTiming, Instance2d, Mesh3d,
    MeshCreateError, MeshVertex, NormalMeshVertex, Overlay, RendererOptions, SceneDraw3d, Sprite2d,
    Texture2d, TextureCreateError, TextureOptions, TileLayer2d, UvMeshVertex,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Scene submitted to a windowless capture target. Draw and overlay order are
/// identical to the corresponding windowed renderer entry points.
pub enum OffscreenScene<'a> {
    Triangles(&'a [Instance2d]),
    Sprites(&'a [Sprite2d<'a>]),
    Tiles {
        layers: &'a [TileLayer2d<'a>],
        sprites: &'a [Sprite2d<'a>],
    },
    Meshes(&'a [SceneDraw3d<'a>]),
}

#[derive(Debug)]
pub enum OffscreenError {
    InvalidSize { width: u32, height: u32 },
    Adapter(wgpu::RequestAdapterError),
    Device(wgpu::RequestDeviceError),
    Busy,
    Poll(wgpu::PollError),
    Readback,
}

impl std::fmt::Display for OffscreenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSize { width, height } => write!(
                f,
                "offscreen size {width}x{height} is empty or exceeds resource limits"
            ),
            Self::Adapter(err) => write!(f, "no offscreen GPU adapter: {err}"),
            Self::Device(err) => write!(f, "could not create offscreen device: {err}"),
            Self::Busy => write!(
                f,
                "a capture is already pending; retrieve it before submitting another"
            ),
            Self::Poll(err) => write!(f, "could not poll the offscreen device: {err}"),
            Self::Readback => write!(f, "could not read back the captured frame"),
        }
    }
}

impl std::error::Error for OffscreenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Adapter(error) => Some(error),
            Self::Device(error) => Some(error),
            Self::Poll(error) => Some(error),
            _ => None,
        }
    }
}

/// A completed frame in top-left row order, with tightly packed RGBA8 pixels.
/// RGB is sRGB-encoded. The renderer clears with opaque alpha.
#[derive(Debug)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
    /// Submitted scene instances; excludes HUD records.
    pub instances: usize,
    /// Scene draw calls; excludes the overlay pass.
    pub draw_calls: usize,
}

/// Scene counts for a submitted offscreen frame, excluding the HUD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OffscreenSubmission {
    pub instances: usize,
    pub draw_calls: usize,
}

struct ReadbackLayout {
    row: u32,
    padded_row: u32,
    len: u64,
}

impl ReadbackLayout {
    fn new((width, height): (u32, u32), limits: &wgpu::Limits) -> Result<Self, OffscreenError> {
        let invalid = OffscreenError::InvalidSize { width, height };
        if width == 0 || height == 0 || width.max(height) > limits.max_texture_dimension_2d {
            return Err(invalid);
        }
        let row = width.checked_mul(4);
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_row = row.and_then(|row| row.div_ceil(align).checked_mul(align));
        let (Some(row), Some(padded_row)) = (row, padded_row) else {
            return Err(invalid);
        };
        let len = u64::from(padded_row) * u64::from(height);
        if len > limits.max_buffer_size || len > isize::MAX as u64 {
            return Err(invalid);
        }
        Ok(Self {
            row,
            padded_row,
            len,
        })
    }

    fn unpack(&self, bytes: &[u8]) -> Vec<u8> {
        if self.row == self.padded_row {
            return bytes.to_vec();
        }
        let mut packed =
            Vec::with_capacity(bytes.len() / self.padded_row as usize * self.row as usize);
        for row in bytes.chunks_exact(self.padded_row as usize) {
            packed.extend_from_slice(&row[..self.row as usize]);
        }
        packed
    }
}

struct PendingCapture {
    submission: wgpu::SubmissionIndex,
    state: Receiver<Result<(), wgpu::BufferAsyncError>>,
    instances: usize,
    draw_calls: usize,
}

/// A fixed-size offscreen target with one reusable async readback buffer.
///
/// Requires a `wgpu` adapter, but not a window, surface, event loop or display
/// connection. Upload draw resources through this renderer, not a windowed
/// renderer: its device is separate. Image encoding and file output are left
/// to callers.
pub struct OffscreenRenderer {
    pub camera: Camera2d,
    pub camera_3d: Camera3d,
    /// Optional directional light for opaque 3D meshes. Defaults to unlit.
    pub mesh_light: Option<FlatLight3d>,
    /// Optional depth fog and background color for 3D scenes. Defaults to off.
    pub mesh_fog: Option<DepthFog3d>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: ScenePipelines,
    size: (u32, u32),
    color: wgpu::Texture,
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    buffer: Option<wgpu::Buffer>,
    layout: ReadbackLayout,
    pending: Option<PendingCapture>,
    gpu_timer: Option<GpuTimer>,
    adapter_info: Option<wgpu::AdapterInfo>,
}

impl OffscreenRenderer {
    pub async fn new(size: (u32, u32)) -> Result<Self, OffscreenError> {
        Self::new_with_options(size, RendererOptions::default()).await
    }

    /// Create a windowless target, optionally requesting GPU pass timestamps.
    /// Unsupported timestamps do not prevent rendering.
    pub async fn new_with_options(
        (width, height): (u32, u32),
        options: RendererOptions,
    ) -> Result<Self, OffscreenError> {
        if width == 0 || height == 0 {
            return Err(OffscreenError::InvalidSize { width, height });
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .map_err(OffscreenError::Adapter)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: if options.gpu_timing {
                    adapter.features() & wgpu::Features::TIMESTAMP_QUERY
                } else {
                    wgpu::Features::empty()
                },
                ..Default::default()
            })
            .await
            .map_err(OffscreenError::Device)?;
        let mut renderer = Self::with_device(device, queue, (width, height))?;
        renderer.adapter_info = Some(adapter.get_info());
        if options.gpu_timing {
            renderer.gpu_timer = GpuTimer::new(&renderer.device, &renderer.queue);
        }
        Ok(renderer)
    }

    fn with_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        size: (u32, u32),
    ) -> Result<Self, OffscreenError> {
        let layout = ReadbackLayout::new(size, &device.limits())?;
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen scene color"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = color.create_view(&Default::default());
        let depth = depth_target(&device, size);
        let pipelines = ScenePipelines::new(&device, &queue, FORMAT);
        Ok(Self {
            camera: Camera2d::default(),
            camera_3d: Camera3d::default(),
            mesh_light: None,
            mesh_fog: None,
            device,
            queue,
            pipelines,
            size,
            color,
            view,
            depth,
            buffer: None,
            layout,
            pending: None,
            gpu_timer: None,
            adapter_info: None,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn aspect(&self) -> f32 {
        self.size.0 as f32 / self.size.1 as f32
    }

    /// The adapter selected for this target.
    pub fn adapter_info(&self) -> Option<&wgpu::AdapterInfo> {
        self.adapter_info.as_ref()
    }

    pub fn gpu_timing_available(&self) -> bool {
        self.gpu_timer.is_some()
    }

    /// Drain completed scene/overlay samples without blocking. Poll regularly
    /// to free the bounded timestamp pool; frames may be unsampled if it fills.
    pub fn poll_gpu_timings(&mut self) -> Vec<GpuFrameTiming> {
        self.gpu_timer
            .as_mut()
            .map_or_else(Vec::new, |timer| timer.poll(&self.device))
    }

    /// Block until submitted GPU work finishes, or return a polling error.
    /// This does not fetch images or GPU timing samples.
    pub fn wait_idle(&self, timeout: Duration) -> Result<(), OffscreenError> {
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(timeout),
            })
            .map_err(OffscreenError::Poll)?;
        Ok(())
    }

    pub fn create_mesh(
        &self,
        label: &str,
        vertices: &[MeshVertex],
        indices: &[u32],
    ) -> Result<Mesh3d, MeshCreateError> {
        Mesh3d::new(&self.device, label, vertices, indices)
    }

    pub fn create_mesh_with_uvs(
        &self,
        label: &str,
        vertices: &[UvMeshVertex],
        indices: &[u32],
    ) -> Result<Mesh3d, MeshCreateError> {
        Mesh3d::new_with_uvs(&self.device, label, vertices, indices)
    }

    /// Upload indexed triangles with UVs and smooth-shading normals.
    /// Zero and non-finite normals are rejected.
    pub fn create_mesh_with_normals(
        &self,
        label: &str,
        vertices: &[NormalMeshVertex],
        indices: &[u32],
    ) -> Result<Mesh3d, MeshCreateError> {
        Mesh3d::new_with_normals(&self.device, label, vertices, indices)
    }

    pub fn cube_mesh(&self) -> Result<Mesh3d, MeshCreateError> {
        super::cube_mesh(&self.device)
    }

    pub fn create_texture(
        &self,
        label: &str,
        width: u32,
        height: u32,
        rgba8: &[u8],
    ) -> Result<Texture2d, TextureCreateError> {
        self.create_texture_with_options(label, width, height, rgba8, TextureOptions::default())
    }

    pub fn create_texture_with_options(
        &self,
        label: &str,
        width: u32,
        height: u32,
        rgba8: &[u8],
        options: TextureOptions,
    ) -> Result<Texture2d, TextureCreateError> {
        Texture2d::new(
            &self.device,
            &self.queue,
            &self.pipelines.texture_layout,
            TextureUpload {
                label,
                width,
                height,
                rgba8,
                options,
            },
        )
    }

    /// Submit a frame without copying pixels or waiting for GPU completion.
    /// A pending capture is not disturbed: its copy runs first on the same queue.
    pub fn render(
        &mut self,
        scene: OffscreenScene<'_>,
        overlay: &Overlay<'_>,
    ) -> OffscreenSubmission {
        let (frame, timing) = self.record(scene, overlay);
        let counts = OffscreenSubmission {
            instances: frame.instances,
            draw_calls: frame.draw_calls,
        };
        self.submit(frame, timing);
        counts
    }

    fn record(
        &mut self,
        scene: OffscreenScene<'_>,
        overlay: &Overlay<'_>,
    ) -> (RecordedFrame, Option<(usize, GpuFrameKind)>) {
        let kind = match scene {
            OffscreenScene::Meshes(_) => GpuFrameKind::Meshes3d,
            OffscreenScene::Sprites(_) => GpuFrameKind::Sprites2d,
            OffscreenScene::Tiles { .. } => GpuFrameKind::TileMap2d,
            OffscreenScene::Triangles(_) => GpuFrameKind::Triangles2d,
        };
        let timer = self
            .gpu_timer
            .as_ref()
            .and_then(|timer| Some((timer, timer.free_slot()?)));
        let writes = timer.map(|(timer, _)| timer);
        let is_3d = kind == GpuFrameKind::Meshes3d;
        let aspect = self.aspect();
        let target = FrameTarget {
            device: &self.device,
            queue: &self.queue,
            size: self.size,
            color: &self.view,
            depth: is_3d.then_some(&self.depth),
            view_projection: if is_3d {
                self.camera_3d.view_projection(aspect)
            } else {
                self.camera.view_projection(aspect)
            },
        };
        let mut frame = match scene {
            OffscreenScene::Meshes(draws) => self.pipelines.record_3d(
                target,
                self.mesh_light,
                self.mesh_fog,
                draws.iter().copied(),
                overlay,
                writes,
            ),
            OffscreenScene::Sprites(sprites) => {
                self.pipelines
                    .record_2d(target, Scene2d::Sprites(sprites), overlay, writes)
            }
            OffscreenScene::Tiles { layers, sprites } => self.pipelines.record_2d(
                target,
                Scene2d::Tiles {
                    layers,
                    sprites,
                    view: self.camera.visible_bounds(aspect),
                },
                overlay,
                writes,
            ),
            OffscreenScene::Triangles(triangles) => {
                self.pipelines
                    .record_2d(target, Scene2d::Triangles(triangles), overlay, writes)
            }
        };
        let timing = timer.map(|(timer, slot)| {
            timer.resolve(&mut frame.encoder, slot, frame.has_overlay);
            (slot, kind)
        });
        (frame, timing)
    }

    fn submit(
        &mut self,
        frame: RecordedFrame,
        timing: Option<(usize, GpuFrameKind)>,
    ) -> wgpu::SubmissionIndex {
        let submission = self.queue.submit(Some(frame.encoder.finish()));
        if let Some((slot, kind)) = timing {
            self.gpu_timer.as_mut().expect("timer recorded").submitted(
                slot,
                kind,
                frame.has_overlay,
            );
        }
        submission
    }

    /// Submit a frame and start asynchronous readback. If the previous capture
    /// has not been retrieved yet, returns `Busy` without submitting anything.
    pub fn capture(
        &mut self,
        scene: OffscreenScene<'_>,
        overlay: &Overlay<'_>,
    ) -> Result<(), OffscreenError> {
        if self.pending.is_some() {
            return Err(OffscreenError::Busy);
        }
        let (mut frame, timing) = self.record(scene, overlay);
        let buffer = self.buffer.get_or_insert_with(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("offscreen scene readback"),
                size: self.layout.len,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        frame.encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.layout.padded_row),
                    rows_per_image: Some(self.size.1),
                },
            },
            extent(self.size),
        );
        let (instances, draw_calls) = (frame.instances, frame.draw_calls);
        let submission = self.submit(frame, timing);
        let (sender, state) = sync_channel(1);
        self.buffer
            .as_ref()
            .expect("capture buffer allocated")
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.try_send(result);
            });
        self.pending = Some(PendingCapture {
            submission,
            state,
            instances,
            draw_calls,
        });
        Ok(())
    }

    /// Retrieve a finished capture without waiting. `None` means there is no
    /// capture or the GPU has not completed it yet.
    pub fn poll_capture(&mut self) -> Result<Option<CapturedFrame>, OffscreenError> {
        self.collect(wgpu::PollType::Poll)
    }

    /// Wait for the pending capture on a native backend, for tools that can block. A timeout
    /// leaves the capture pending so it can still be polled or waited on.
    pub fn wait_capture(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedFrame>, OffscreenError> {
        let Some(pending) = &self.pending else {
            return Ok(None);
        };
        self.collect(wgpu::PollType::Wait {
            submission_index: Some(pending.submission.clone()),
            timeout: Some(timeout),
        })
    }

    fn collect(&mut self, poll: wgpu::PollType) -> Result<Option<CapturedFrame>, OffscreenError> {
        let Some(pending) = &self.pending else {
            return Ok(None);
        };
        self.device.poll(poll).map_err(OffscreenError::Poll)?;
        match pending.state.try_recv() {
            Err(TryRecvError::Empty) => return Ok(None),
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                self.pending = None;
                return Err(OffscreenError::Readback);
            }
        }
        let pending = self.pending.take().expect("finished capture");
        let buffer = self.buffer.as_ref().expect("pending capture buffer");
        let rgba8 = buffer
            .slice(..)
            .get_mapped_range()
            .map(|bytes| self.layout.unpack(bytes.as_ref()));
        buffer.unmap();
        Ok(Some(CapturedFrame {
            width: self.size.0,
            height: self.size.1,
            rgba8: rgba8.map_err(|_| OffscreenError::Readback)?,
            instances: pending.instances,
            draw_calls: pending.draw_calls,
        }))
    }
}

fn extent((width, height): (u32, u32)) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Mat4, Vec2, Vec3};
    use crate::renderer::{
        Draw3d, UiImage, UiRect,
        test_support::{skip_without_adapter, test_device},
    };

    #[test]
    fn render_only_does_not_allocate_readback_or_replace_pending_capture() {
        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };
        let mut renderer = OffscreenRenderer::with_device(device, queue, (32, 32)).unwrap();
        let timeout = Duration::from_secs(5);
        let empty = Overlay::default();
        assert_eq!(
            renderer.render(OffscreenScene::Triangles(&[]), &empty),
            OffscreenSubmission {
                instances: 0,
                draw_calls: 0
            },
        );
        assert!(renderer.buffer.is_none());
        assert!(renderer.poll_gpu_timings().is_empty());
        assert!(renderer.poll_capture().unwrap().is_none());
        let overlay = Overlay {
            rects: &[UiRect::new(
                Vec2::ZERO,
                Vec2::new(32.0, 32.0),
                [1.0, 0.0, 0.0, 1.0],
            )],
            ..empty
        };
        renderer
            .capture(OffscreenScene::Triangles(&[]), &overlay)
            .unwrap();
        renderer.render(OffscreenScene::Triangles(&[]), &empty);
        renderer.wait_idle(timeout).unwrap();
        let captured = renderer.wait_capture(timeout).unwrap().unwrap();
        assert_eq!(&captured.rgba8[..4], &[255, 0, 0, 255]);
        assert!(renderer.poll_capture().unwrap().is_none());
    }

    #[test]
    fn windowless_timestamp_samples_distinguish_scene_and_overlay() {
        use crate::renderer::test_support::test_device_with_features;
        let Some((device, queue)) = test_device_with_features(wgpu::Features::TIMESTAMP_QUERY)
        else {
            skip_without_adapter();
            return;
        };
        let mut renderer = OffscreenRenderer::with_device(device, queue, (32, 32)).unwrap();
        renderer.gpu_timer = GpuTimer::new(&renderer.device, &renderer.queue);
        if !renderer.gpu_timing_available() {
            return;
        }
        let timeout = Duration::from_secs(5);
        renderer.render(OffscreenScene::Meshes(&[]), &Overlay::default());
        renderer.render(
            OffscreenScene::Triangles(&[]),
            &Overlay {
                rects: &[UiRect::new(Vec2::ZERO, Vec2::ONE, [1.0; 4])],
                ..Overlay::default()
            },
        );
        renderer.wait_idle(timeout).unwrap();
        let timings = renderer.poll_gpu_timings();
        assert_eq!(timings.len(), 2);
        assert_eq!(
            (timings[0].kind, timings[0].overlay),
            (GpuFrameKind::Meshes3d, None)
        );
        assert_eq!(timings[1].kind, GpuFrameKind::Triangles2d);
        assert!(timings[1].overlay.is_some());
        assert!(renderer.buffer.is_none());
    }

    #[test]
    fn tile_capture_samples_atlas_and_bounds_the_visible_batch() {
        use crate::renderer::{MAX_DRAWN_TILES, TileMap2d};
        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };
        let mut renderer = OffscreenRenderer::with_device(device, queue, (64, 64)).unwrap();
        let atlas = renderer
            .create_texture_with_options(
                "test tiles",
                2,
                1,
                &[255, 0, 0, 255, 0, 255, 0, 255],
                TextureOptions::NEAREST,
            )
            .unwrap();
        let regions = [atlas.region(0, 0, 1, 1), atlas.region(1, 0, 1, 1)];
        let mut map = TileMap2d::new(
            (2, 2),
            Vec2::new(-10.0, 10.0),
            Vec2::new(10.0, 10.0),
            vec![Some(0), Some(1), None, Some(7)],
        )
        .unwrap();
        let capture = |renderer: &mut OffscreenRenderer, map: &TileMap2d| {
            let layers = [TileLayer2d::new(map, &atlas, &regions)];
            renderer
                .capture(
                    OffscreenScene::Tiles {
                        layers: &layers,
                        sprites: &[],
                    },
                    &Overlay::default(),
                )
                .unwrap();
            renderer
                .wait_capture(Duration::from_secs(5))
                .unwrap()
                .expect("tile capture")
        };
        let pixel = |frame: &CapturedFrame, x: usize, y: usize| {
            frame.rgba8.as_chunks::<4>().0[y * frame.width as usize + x]
        };
        let frame = capture(&mut renderer, &map);
        assert_eq!((frame.instances, frame.draw_calls), (2, 1));
        assert_eq!(pixel(&frame, 16, 16), [255, 0, 0, 255]);
        assert_eq!(pixel(&frame, 48, 16), [0, 255, 0, 255]);
        let clear = pixel(&frame, 16, 48);
        assert_eq!(pixel(&frame, 48, 48), clear);
        for x in 0..64 {
            assert_ne!(pixel(&frame, x, 16), clear, "gap between atlas tiles");
        }
        let marker = Sprite2d::new(&atlas, Vec2::new(-5.0, 5.0), Vec2::new(5.0, 5.0))
            .with_region(regions[1]);
        renderer
            .capture(
                OffscreenScene::Tiles {
                    layers: &[TileLayer2d::new(&map, &atlas, &regions)],
                    sprites: &[marker],
                },
                &Overlay::default(),
            )
            .unwrap();
        let combined = renderer
            .wait_capture(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!((combined.instances, combined.draw_calls), (3, 2));
        assert_eq!(pixel(&combined, 16, 16), [0, 255, 0, 255]);
        assert_eq!(pixel(&combined, 4, 4), [255, 0, 0, 255]);

        assert!(map.set((0, 1), Some(1)));
        renderer.camera.center.x = -10.0;
        let frame = capture(&mut renderer, &map);
        assert_eq!((frame.instances, frame.draw_calls), (2, 1));
        assert_eq!(pixel(&frame, 16, 16), clear);
        assert_eq!(pixel(&frame, 48, 48), [0, 255, 0, 255]);

        renderer.camera.center.x = 40.0;
        let frame = capture(&mut renderer, &map);
        assert_eq!((frame.instances, frame.draw_calls), (0, 0));
        assert_eq!(pixel(&frame, 48, 48), clear);

        renderer.camera.center = Vec2::ZERO;
        let dense = TileMap2d::new(
            (65, 65),
            Vec2::new(-9.0, 9.0),
            Vec2::new(0.25, 0.25),
            vec![Some(0); 65 * 65],
        )
        .unwrap();
        let frame = capture(&mut renderer, &dense);
        assert_eq!((frame.instances, frame.draw_calls), (MAX_DRAWN_TILES, 1));
    }

    #[test]
    fn tile_layers_composite_scroll_and_enforce_limits() {
        use crate::renderer::{MAX_DRAWN_TILES, MAX_TILE_LAYERS, TileMap2d};
        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };
        let mut renderer = OffscreenRenderer::with_device(device, queue, (64, 64)).unwrap();
        let red = renderer
            .create_texture_with_options(
                "red tile",
                1,
                1,
                &[255, 0, 0, 255],
                TextureOptions::NEAREST,
            )
            .unwrap();
        let blue = renderer
            .create_texture_with_options(
                "blue tile",
                1,
                1,
                &[0, 0, 255, 255],
                TextureOptions::NEAREST,
            )
            .unwrap();
        let red_regions = [red.region(0, 0, 1, 1)];
        let blue_regions = [blue.region(0, 0, 1, 1)];
        let map = TileMap2d::new(
            (1, 1),
            Vec2::new(0.0, 10.0),
            Vec2::new(10.0, 10.0),
            vec![Some(0)],
        )
        .unwrap();
        let pixel = |frame: &CapturedFrame, x: usize, y: usize| {
            frame.rgba8.as_chunks::<4>().0[y * frame.width as usize + x]
        };
        let capture =
            |renderer: &mut OffscreenRenderer, layers: &[TileLayer2d<'_>], center: Vec2| {
                renderer.camera.center = center;
                renderer
                    .capture(
                        OffscreenScene::Tiles {
                            layers,
                            sprites: &[],
                        },
                        &Overlay::default(),
                    )
                    .unwrap();
                renderer
                    .wait_capture(Duration::from_secs(5))
                    .unwrap()
                    .expect("tile layer capture")
            };
        let clear = pixel(&capture(&mut renderer, &[], Vec2::ZERO), 48, 16);

        // Later layers composite over earlier ones, and different atlases split
        // the tile batch into separate draws.
        let red_layer = TileLayer2d::new(&map, &red, &red_regions);
        let blue_layer = TileLayer2d::new(&map, &blue, &blue_regions);
        let frame = capture(&mut renderer, &[red_layer, blue_layer], Vec2::ZERO);
        assert_eq!((frame.instances, frame.draw_calls), (2, 2));
        assert_eq!(pixel(&frame, 48, 16), [0, 0, 255, 255]);
        let frame = capture(&mut renderer, &[blue_layer, red_layer], Vec2::ZERO);
        assert_eq!((frame.instances, frame.draw_calls), (2, 2));
        assert_eq!(pixel(&frame, 48, 16), [255, 0, 0, 255]);

        // A world-fixed layer scrolls with the camera; a screen-fixed layer
        // shifts by the camera center so it stays in place.
        let world_layer = TileLayer2d::new(&map, &red, &red_regions);
        let screen_layer = world_layer.with_parallax(Vec2::ZERO);
        let frame = capture(&mut renderer, &[world_layer], Vec2::new(10.0, 0.0));
        assert_eq!(pixel(&frame, 16, 16), [255, 0, 0, 255]);
        assert_eq!(pixel(&frame, 48, 16), clear);
        let frame = capture(&mut renderer, &[screen_layer], Vec2::new(10.0, 0.0));
        assert_eq!(pixel(&frame, 16, 16), clear);
        assert_eq!(pixel(&frame, 48, 16), [255, 0, 0, 255]);
        assert_eq!(
            screen_layer.cell_at_world(Vec2::new(15.0, 5.0), Vec2::new(10.0, 0.0)),
            Some((0, 0))
        );
        assert_eq!(
            screen_layer.cell_at_world(Vec2::new(5.0, 5.0), Vec2::new(10.0, 0.0)),
            None
        );

        // Extra layers beyond the public cap are ignored, while identical
        // adjacent layers still batch into one draw.
        let capped: Vec<_> = (0..=MAX_TILE_LAYERS)
            .map(|_| TileLayer2d::new(&map, &red, &red_regions))
            .collect();
        let frame = capture(&mut renderer, &capped, Vec2::ZERO);
        assert_eq!((frame.instances, frame.draw_calls), (MAX_TILE_LAYERS, 1));
        assert_eq!(pixel(&frame, 48, 16), [255, 0, 0, 255]);

        // Fill the entire batch, then verify that a second draw starts at
        // the last layer's nonzero instance offset.
        let dense = TileMap2d::new(
            (65, 65),
            Vec2::new(-9.0, 9.0),
            Vec2::new(0.25, 0.25),
            vec![Some(0); 65 * 65],
        )
        .unwrap();
        let mut dense_layers = [TileLayer2d::new(&dense, &red, &red_regions); MAX_TILE_LAYERS];
        dense_layers[MAX_TILE_LAYERS - 1] = TileLayer2d::new(&dense, &blue, &blue_regions);
        let frame = capture(&mut renderer, &dense_layers, Vec2::ZERO);
        assert_eq!(
            (frame.instances, frame.draw_calls),
            (MAX_DRAWN_TILES * MAX_TILE_LAYERS, 2)
        );
        assert_eq!(pixel(&frame, 32, 32), [0, 0, 255, 255]);
    }

    #[test]
    fn readback_layout_validates_limits_and_strips_row_padding() {
        let limits = wgpu::Limits {
            max_texture_dimension_2d: 128,
            max_buffer_size: 512,
            ..Default::default()
        };
        for size in [(0, 1), (129, 1), (65, 2), (1, 3)] {
            assert!(matches!(
                ReadbackLayout::new(size, &limits),
                Err(OffscreenError::InvalidSize { .. })
            ));
        }
        let layout = ReadbackLayout::new((1, 2), &limits).unwrap();
        assert_eq!((layout.row, layout.padded_row, layout.len), (4, 256, 512));
        let mut bytes = [99; 512];
        bytes[..4].copy_from_slice(&[1, 2, 3, 4]);
        bytes[256..260].copy_from_slice(&[5, 6, 7, 8]);
        assert_eq!(layout.unpack(&bytes), [1, 2, 3, 4, 5, 6, 7, 8]);
        let aligned = ReadbackLayout::new((64, 2), &limits).unwrap();
        assert_eq!(aligned.unpack(&bytes), bytes);
    }

    #[test]
    fn capture_works_without_a_window_and_reuses_readback_for_a_different_scene() {
        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };
        let mut renderer = OffscreenRenderer::with_device(device, queue, (67, 49)).unwrap();
        let capture_timeout = Duration::from_secs(5);
        assert!(renderer.poll_capture().unwrap().is_none());
        let atlas = renderer
            .create_texture("test atlas", 1, 1, &[0, 0, 255, 255])
            .unwrap();
        let rects = [UiRect::new(
            Vec2::ZERO,
            Vec2::new(14.0, 14.0),
            [1.0, 0.0, 0.0, 1.0],
        )];
        let images = [UiImage::new(
            &atlas,
            Vec2::new(6.0, 6.0),
            Vec2::new(6.0, 6.0),
        )];
        let overlay = Overlay {
            rects: &rects,
            images: &images,
            ..Default::default()
        };
        let sprites = [Sprite2d::new(&atlas, Vec2::ZERO, Vec2::new(8.0, 8.0))];
        renderer
            .capture(OffscreenScene::Sprites(&sprites), &overlay)
            .unwrap();
        assert!(matches!(
            renderer.capture(OffscreenScene::Triangles(&[]), &Overlay::default()),
            Err(OffscreenError::Busy)
        ));
        let first = match renderer.poll_capture().unwrap() {
            Some(frame) => frame,
            None => renderer
                .wait_capture(capture_timeout)
                .unwrap()
                .expect("sprite frame"),
        };
        let pixel = |frame: &CapturedFrame, x: u32, y: u32| -> [u8; 4] {
            let offset = (4 * (frame.width * y + x)) as usize;
            frame.rgba8[offset..offset + 4].try_into().unwrap()
        };
        assert_eq!(
            (first.width, first.height, first.rgba8.len()),
            (67, 49, 67 * 49 * 4)
        );
        assert_eq!((first.instances, first.draw_calls), (1, 1));
        assert_eq!(pixel(&first, 2, 2), [255, 0, 0, 255]);
        assert_eq!(pixel(&first, 8, 8), [0, 0, 255, 255]);
        assert_eq!(pixel(&first, 33, 24), [0, 0, 255, 255]);

        let triangle = renderer
            .create_mesh(
                "depth probe",
                &[
                    MeshVertex {
                        position: [-1.0, -1.0, 0.0],
                        color: [1.0; 3],
                    },
                    MeshVertex {
                        position: [1.0, -1.0, 0.0],
                        color: [1.0; 3],
                    },
                    MeshVertex {
                        position: [0.0, 1.0, 0.0],
                        color: [1.0; 3],
                    },
                ],
                &[0, 1, 2],
            )
            .unwrap();
        renderer.camera_3d.position = Vec3::ZERO;
        let draws = [
            Draw3d {
                mesh: &triangle,
                model: Mat4::translation(Vec3::new(0.0, 0.0, -2.0)),
                tint: [1.0, 0.0, 0.0],
            }
            .into(),
            Draw3d {
                mesh: &triangle,
                model: Mat4::translation(Vec3::new(0.0, 0.0, -4.0))
                    .mul(Mat4::scale(Vec3::new(2.0, 2.0, 1.0))),
                tint: [0.0, 1.0, 0.0],
            }
            .into(),
        ];
        renderer
            .capture(OffscreenScene::Meshes(&draws), &Overlay::default())
            .unwrap();
        let second = renderer
            .wait_capture(capture_timeout)
            .unwrap()
            .expect("3D frame");
        assert_eq!((second.instances, second.draw_calls), (2, 1));
        assert_eq!(pixel(&second, 33, 24), [255, 0, 0, 255]);
        assert_ne!(pixel(&first, 2, 2), pixel(&second, 2, 2));
        assert!(renderer.poll_capture().unwrap().is_none());
    }

    #[test]
    fn flat_light_shades_textured_and_color_meshes_and_can_be_disabled() {
        use crate::math::{Mat4, Vec3};
        use crate::renderer::Draw3d;

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };
        let mut renderer = OffscreenRenderer::with_device(device, queue, (64, 64)).unwrap();
        renderer.camera_3d.position = Vec3::ZERO;
        let mesh = renderer
            .create_mesh(
                "flat light triangle",
                &[[-0.65, -0.65, 0.0], [0.65, -0.65, 0.0], [0.0, 0.65, 0.0]].map(|position| {
                    MeshVertex {
                        position,
                        color: [1.0; 3],
                    }
                }),
                &[0, 1, 2],
            )
            .unwrap();
        let texture = renderer
            .create_texture_with_options(
                "blue albedo",
                1,
                1,
                &[0, 0, 255, 255],
                TextureOptions::NEAREST,
            )
            .unwrap();
        let draws = [
            Draw3d {
                mesh: &mesh,
                model: Mat4::translation(Vec3::new(-0.8, 0.0, -3.0)),
                tint: [1.0, 0.0, 0.0],
            }
            .into(),
            Draw3d {
                mesh: &mesh,
                // Mirror the winding without changing the visible-side normal.
                model: Mat4::translation(Vec3::new(0.8, 0.0, -3.0))
                    .mul(Mat4::scale(Vec3::new(-1.0, 1.0, 1.0))),
                tint: [1.0; 3],
            }
            .with_texture(&texture),
        ];
        let capture = |renderer: &mut OffscreenRenderer, light| {
            renderer.mesh_light = light;
            renderer
                .capture(OffscreenScene::Meshes(&draws), &Overlay::default())
                .unwrap();
            let frame = renderer
                .wait_capture(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert_eq!((frame.instances, frame.draw_calls), (2, 2));
            let pixels = frame.rgba8.as_chunks::<4>().0;
            [pixels[32 * 64 + 17], pixels[32 * 64 + 47]]
        };
        let unlit = [[255, 0, 0, 255], [0, 0, 255, 255]];
        assert_eq!(capture(&mut renderer, None), unlit);
        let toward = FlatLight3d::new(Vec3::new(0.0, 0.0, 1.0), 0.25);
        assert_eq!(capture(&mut renderer, toward), unlit);
        let away = FlatLight3d::new(Vec3::new(0.0, 0.0, -1.0), 0.25);
        let [red, blue] = capture(&mut renderer, away);
        // A linear brightness of 0.25 encodes to approximately 137 in sRGB.
        assert!((136..=138).contains(&red[0]), "{red:?}");
        assert_eq!(red, [red[0], 0, 0, 255]);
        assert_eq!(blue, [0, 0, red[0], 255]);
        let oblique = FlatLight3d::new(Vec3::new(3.0_f32.sqrt(), 0.0, 1.0), 0.25);
        let [red, blue] = capture(&mut renderer, oblique);
        // A cosine of 0.5 gives 0.625 linear brightness, about 207 in sRGB.
        assert!((206..=208).contains(&red[0]), "{red:?}");
        assert_eq!(blue, [0, 0, red[0], 255]);
        assert_eq!(capture(&mut renderer, None), unlit);
    }

    #[test]
    fn smooth_mesh_lighting_interpolates_and_transforms_normals() {
        use crate::math::{Mat4, Vec3};
        use crate::renderer::Draw3d;

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };
        let mut renderer = OffscreenRenderer::with_device(device, queue, (64, 64)).unwrap();
        renderer.camera_3d.position = Vec3::ZERO;
        let positions = [[-1.2, -0.8, 0.0], [1.2, -0.8, 0.0], [0.0, 1.2, 0.0]];
        let vertices = |normals: [[f32; 3]; 3]| {
            std::array::from_fn::<_, 3, _>(|i| NormalMeshVertex {
                position: positions[i],
                color: [1.0; 3],
                uv: [0.0; 2],
                normal: normals[i],
            })
        };
        let gradient = renderer
            .create_mesh_with_normals(
                "gradient normals",
                &vertices([[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]),
                &[0, 1, 2],
            )
            .unwrap();
        let slanted = renderer
            .create_mesh_with_normals(
                "slanted normals",
                &vertices([[1.0, 0.0, 1.0]; 3]),
                &[0, 1, 2],
            )
            .unwrap();
        let white = renderer
            .create_texture_with_options("white albedo", 1, 1, &[255; 4], TextureOptions::NEAREST)
            .unwrap();
        let capture =
            |renderer: &mut OffscreenRenderer, mesh: &Mesh3d, texture, scale, direction| {
                renderer.mesh_light = FlatLight3d::new(direction, 0.2);
                let draws = [SceneDraw3d {
                    draw: Draw3d {
                        mesh,
                        model: Mat4::translation(Vec3::new(0.0, 0.0, -3.0)).mul(Mat4::scale(scale)),
                        tint: [1.0; 3],
                    },
                    texture,
                }];
                renderer
                    .capture(OffscreenScene::Meshes(&draws), &Overlay::default())
                    .unwrap();
                let frame = renderer
                    .wait_capture(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!((frame.instances, frame.draw_calls), (1, 1));
                let pixels = frame.rgba8.as_chunks::<4>().0;
                [23, 32, 41].map(|x| pixels[39 * 64 + x][0])
            };
        let toward = Vec3::new(0.0, 0.0, 1.0);
        let [left, middle, right] = capture(
            &mut renderer,
            &gradient,
            Some(&white),
            Vec3::new(1.0, 1.0, 1.0),
            toward,
        );
        assert!(
            left > middle && middle > right && right > 130,
            "{left}, {middle}, {right}"
        );

        // The inverse transpose of the mirrored, stretched X scale sends
        // (1, 0, 1) toward (-0.5, 0, 1); the -X light sees cosine 1/sqrt(5).
        let transformed = capture(
            &mut renderer,
            &slanted,
            None,
            Vec3::new(-2.0, 1.0, 1.0),
            Vec3::new(-1.0, 0.0, 0.0),
        );
        assert!(
            transformed.into_iter().all(|p| (195..=199).contains(&p)),
            "{transformed:?}"
        );
        // Flattening Z makes the normal transform singular, but leaves the
        // XY geometry intact so the geometric fallback still works.
        assert_eq!(
            capture(
                &mut renderer,
                &slanted,
                None,
                Vec3::new(1.0, 1.0, 0.0),
                toward
            ),
            [255; 3]
        );
    }

    #[test]
    fn depth_fog_blends_by_view_depth_and_leaves_the_hud_unfogged() {
        use crate::math::{Mat4, Vec2, Vec3};
        use crate::renderer::{Draw3d, UiRect};

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };
        let mut renderer = OffscreenRenderer::with_device(device, queue, (64, 64)).unwrap();
        renderer.camera_3d.position = Vec3::ZERO;
        let mesh = renderer
            .create_mesh(
                "fog test quad",
                &[
                    [-0.18, -0.18, 0.0],
                    [0.18, -0.18, 0.0],
                    [0.18, 0.18, 0.0],
                    [-0.18, 0.18, 0.0],
                ]
                .map(|position| MeshVertex {
                    position,
                    color: [1.0; 3],
                }),
                &[0, 1, 2, 0, 2, 3],
            )
            .unwrap();
        let red = renderer
            .create_texture_with_options(
                "red albedo",
                1,
                1,
                &[255, 0, 0, 255],
                TextureOptions::NEAREST,
            )
            .unwrap();
        let draws = std::array::from_fn::<_, 3, _>(|i| {
            let depth = [2.0, 6.0, 10.0][i];
            let textured = i == 1;
            SceneDraw3d {
                draw: Draw3d {
                    mesh: &mesh,
                    model: Mat4::translation(Vec3::new(
                        (i as f32 - 1.0) * 0.38 * depth,
                        0.0,
                        -depth,
                    ))
                    .mul(Mat4::scale(Vec3::new(depth, depth, 1.0))),
                    tint: if textured { [1.0; 3] } else { [1.0, 0.0, 0.0] },
                },
                texture: textured.then_some(&red),
            }
        });
        let hud = Overlay {
            rects: &[UiRect::new(
                Vec2::ZERO,
                Vec2::new(8.0, 8.0),
                [0.0, 1.0, 0.0, 1.0],
            )],
            ..Overlay::default()
        };
        let capture = |renderer: &mut OffscreenRenderer, scene| {
            renderer.capture(scene, &hud).unwrap();
            renderer
                .wait_capture(Duration::from_secs(5))
                .unwrap()
                .unwrap()
        };
        let pixel =
            |frame: &CapturedFrame, x: usize, y: usize| frame.rgba8.as_chunks::<4>().0[y * 64 + x];
        let scene = || OffscreenScene::Meshes(&draws);
        let unlit = capture(&mut renderer, scene());
        renderer.mesh_fog = DepthFog3d::new([0.0, 0.0, 1.0], 4.0, 8.0);
        // Fog uses forward view depth, not the perspective depth buffer value.
        renderer.camera_3d.far = 40.0;
        let fogged = capture(&mut renderer, scene());
        assert_eq!((unlit.instances, unlit.draw_calls), (3, 3));
        assert_eq!((fogged.instances, fogged.draw_calls), (3, 3));
        for x in [16, 32, 48] {
            assert_eq!(pixel(&unlit, x, 32), [255, 0, 0, 255]);
        }
        assert_eq!(pixel(&fogged, 16, 32), [255, 0, 0, 255]);
        let mid: [u8; 4] = pixel(&fogged, 32, 32);
        assert!((187..=189).contains(&mid[0]) && mid[0] == mid[2], "{mid:?}");
        assert_eq!([mid[1], mid[3]], [0, 255]);
        let blue = [0, 0, 255, 255];
        assert_eq!(pixel(&fogged, 48, 32), blue);
        assert_eq!(pixel(&fogged, 63, 63), blue);
        assert_eq!(pixel(&fogged, 4, 4), [0, 255, 0, 255]);
        let plane = capture(&mut renderer, OffscreenScene::Triangles(&[]));
        assert_eq!(pixel(&plane, 63, 63), pixel(&unlit, 63, 63));
        let empty = capture(&mut renderer, OffscreenScene::Meshes(&[]));
        assert_eq!(pixel(&empty, 63, 63), blue);
        renderer.mesh_fog = None;
        assert_eq!(capture(&mut renderer, scene()).rgba8, unlit.rgba8);
    }
}
