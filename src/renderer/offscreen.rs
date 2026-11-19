//! Windowless scene capture into an RGBA8 sRGB image.

use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::Duration;

use super::frame::{FrameTarget, Scene2d, ScenePipelines, depth_target};
use super::texture::TextureUpload;
use super::{
    Camera2d, Camera3d, Instance2d, Mesh3d, MeshCreateError, MeshVertex, Overlay, SceneDraw3d,
    Sprite2d, Texture2d, TextureCreateError, TextureOptions, TileLayer2d, UvMeshVertex,
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
            Self::Poll(err) => write!(f, "could not wait for capture: {err}"),
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
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: ScenePipelines,
    size: (u32, u32),
    color: wgpu::Texture,
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    buffer: wgpu::Buffer,
    layout: ReadbackLayout,
    pending: Option<PendingCapture>,
}

impl OffscreenRenderer {
    pub async fn new((width, height): (u32, u32)) -> Result<Self, OffscreenError> {
        if width == 0 || height == 0 {
            return Err(OffscreenError::InvalidSize { width, height });
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .map_err(OffscreenError::Adapter)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(OffscreenError::Device)?;
        Self::with_device(device, queue, (width, height))
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
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen scene readback"),
            size: layout.len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let pipelines = ScenePipelines::new(&device, &queue, FORMAT);
        Ok(Self {
            camera: Camera2d::default(),
            camera_3d: Camera3d::default(),
            device,
            queue,
            pipelines,
            size,
            color,
            view,
            depth,
            buffer,
            layout,
            pending: None,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn aspect(&self) -> f32 {
        self.size.0 as f32 / self.size.1 as f32
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
        let is_3d = matches!(scene, OffscreenScene::Meshes(_));
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
            OffscreenScene::Meshes(draws) => {
                self.pipelines
                    .record_3d(target, draws.iter().copied(), overlay, None)
            }
            OffscreenScene::Sprites(sprites) => {
                self.pipelines
                    .record_2d(target, Scene2d::Sprites(sprites), overlay, None)
            }
            OffscreenScene::Tiles { layers, sprites } => self.pipelines.record_2d(
                target,
                Scene2d::Tiles {
                    layers,
                    sprites,
                    view: self.camera.visible_bounds(aspect),
                },
                overlay,
                None,
            ),
            OffscreenScene::Triangles(triangles) => {
                self.pipelines
                    .record_2d(target, Scene2d::Triangles(triangles), overlay, None)
            }
        };
        frame.encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.layout.padded_row),
                    rows_per_image: Some(self.size.1),
                },
            },
            extent(self.size),
        );
        let submission = self.queue.submit(Some(frame.encoder.finish()));
        let (sender, state) = sync_channel(1);
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.try_send(result);
            });
        self.pending = Some(PendingCapture {
            submission,
            state,
            instances: frame.instances,
            draw_calls: frame.draw_calls,
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
        let rgba8 = self
            .buffer
            .slice(..)
            .get_mapped_range()
            .map(|bytes| self.layout.unpack(bytes.as_ref()));
        self.buffer.unmap();
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

        // Each layer gets its own visible-tile budget.
        let dense = TileMap2d::new(
            (65, 65),
            Vec2::new(-9.0, 9.0),
            Vec2::new(0.25, 0.25),
            vec![Some(0); 65 * 65],
        )
        .unwrap();
        let dense_layers = [
            TileLayer2d::new(&dense, &red, &red_regions),
            TileLayer2d::new(&dense, &red, &red_regions),
        ];
        let frame = capture(&mut renderer, &dense_layers, Vec2::ZERO);
        assert_eq!(
            (frame.instances, frame.draw_calls),
            (MAX_DRAWN_TILES * 2, 1)
        );
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
}
