//! Surface-independent scene and overlay command recording.

use super::{
    Bounds2d, DepthFog3d, FlatLight3d, Instance2d, Overlay, Renderer, SceneDraw3d, Sprite2d,
    TileLayer2d, image_batch::ImageBatch, mesh::MeshPipeline, overlay::OverlayRenderer,
    pipeline::TrianglePipeline, sprite, tilemap, timing::GpuTimer,
};
use crate::math::Mat4;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub(super) const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.04,
    g: 0.05,
    b: 0.07,
    a: 1.0,
};

pub(super) enum Scene2d<'a> {
    Triangles(&'a [Instance2d]),
    Sprites(&'a [Sprite2d<'a>]),
    Tiles {
        layers: &'a [TileLayer2d<'a>],
        view: Bounds2d,
        sprites: &'a [Sprite2d<'a>],
    },
}

pub(super) struct FrameTarget<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub size: (u32, u32),
    pub color: &'a wgpu::TextureView,
    pub depth: Option<&'a wgpu::TextureView>,
    pub view_projection: Mat4,
}

pub(super) struct RecordedFrame {
    pub encoder: wgpu::CommandEncoder,
    pub instances: usize,
    pub draw_calls: usize,
    pub has_overlay: bool,
}

pub(super) struct ScenePipelines {
    triangle: TrianglePipeline,
    sprites: Option<ImageBatch>,
    tiles: Option<ImageBatch>,
    meshes: MeshPipeline,
    overlay: OverlayRenderer,
    pub texture_layout: wgpu::BindGroupLayout,
}

impl ScenePipelines {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let texture_layout = super::texture::texture_layout(device);
        Self {
            triangle: TrianglePipeline::new(device, format),
            sprites: None,
            tiles: None,
            meshes: MeshPipeline::new(device, format, &texture_layout),
            overlay: OverlayRenderer::new(device, queue, format, &texture_layout),
            texture_layout,
        }
    }

    pub fn record_2d(
        &mut self,
        target: FrameTarget<'_>,
        scene: Scene2d<'_>,
        overlay: &Overlay<'_>,
        timer: Option<&GpuTimer>,
    ) -> RecordedFrame {
        let count = match scene {
            Scene2d::Triangles(instances) => {
                let count = Renderer::budgeted_instance_count_static(instances.len());
                self.triangle.write_instances(
                    target.queue,
                    target.view_projection,
                    instances[..count].iter().map(Instance2d::model_matrix),
                );
                count
            }
            Scene2d::Sprites(sprites) => self.prepare_sprites(&target, sprites),
            Scene2d::Tiles {
                layers,
                view,
                sprites,
            } => {
                let tile_count = tilemap::write_tile_layers(
                    self.tiles
                        .get_or_insert_with(|| self.overlay.tile_batch(target.device)),
                    target.queue,
                    target.size,
                    target.view_projection,
                    layers,
                    view,
                );
                tile_count + self.prepare_sprites(&target, sprites)
            }
        };
        self.record(
            target,
            count,
            CLEAR_COLOR,
            overlay,
            timer,
            |this, pass| match scene {
                Scene2d::Triangles(_) => this.triangle.draw(pass),
                Scene2d::Sprites(sprites) => this.draw_sprites(pass, sprites),
                Scene2d::Tiles {
                    layers, sprites, ..
                } => {
                    let tiles = tilemap::draw_tiles(
                        this.tiles.as_ref().expect("tile batch prepared"),
                        pass,
                        layers,
                    );
                    tiles + this.draw_sprites(pass, sprites)
                }
            },
        )
    }

    fn prepare_sprites(&mut self, target: &FrameTarget<'_>, sprites: &[Sprite2d<'_>]) -> usize {
        if sprites.is_empty() {
            return 0;
        }
        sprite::write_sprites(
            self.sprites
                .get_or_insert_with(|| self.overlay.sprite_batch(target.device)),
            target.queue,
            target.size,
            target.view_projection,
            sprites,
        )
    }

    fn draw_sprites(&self, pass: &mut wgpu::RenderPass<'_>, sprites: &[Sprite2d<'_>]) -> usize {
        if sprites.is_empty() {
            return 0;
        }
        sprite::draw_sprites(
            self.sprites.as_ref().expect("sprite batch prepared"),
            pass,
            sprites,
        )
    }

    pub fn record_3d<'a>(
        &mut self,
        target: FrameTarget<'_>,
        light: Option<FlatLight3d>,
        fog: Option<DepthFog3d>,
        draws: impl ExactSizeIterator<Item = SceneDraw3d<'a>> + Clone,
        overlay: &Overlay<'_>,
        timer: Option<&GpuTimer>,
    ) -> RecordedFrame {
        let count = Renderer::budgeted_instance_count_static(draws.len());
        self.meshes.write_draws(
            target.queue,
            target.view_projection,
            light,
            fog,
            draws.clone().take(count),
        );
        let clear = fog.map_or(CLEAR_COLOR, DepthFog3d::clear_color);
        self.record(target, count, clear, overlay, timer, |this, pass| {
            this.meshes.draw(pass, draws.take(count))
        })
    }

    fn record(
        &mut self,
        target: FrameTarget<'_>,
        instances: usize,
        clear: wgpu::Color,
        overlay: &Overlay<'_>,
        timer: Option<&GpuTimer>,
        draw: impl FnOnce(&Self, &mut wgpu::RenderPass<'_>) -> usize,
    ) -> RecordedFrame {
        let mut encoder = target
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("scene frame encoder"),
            });
        let draw_calls = {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: target.depth.map(|view| {
                    wgpu::RenderPassDepthStencilAttachment {
                        view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }
                }),
                timestamp_writes: timer.map(|t| t.writes(false)),
                ..Default::default()
            });
            draw(self, &mut pass)
        };
        let has_overlay = self.overlay.record(
            target.queue,
            target.size,
            &mut encoder,
            target.color,
            overlay,
            timer.map(|t| t.writes(true)),
        );
        RecordedFrame {
            encoder,
            instances,
            draw_calls,
            has_overlay,
        }
    }
}

pub(super) fn depth_target(
    device: &wgpu::Device,
    (width, height): (u32, u32),
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("scene depth target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}
