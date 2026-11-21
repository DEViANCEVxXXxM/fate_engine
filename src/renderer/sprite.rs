use super::image_batch::{ImageBatch, ImageInstance, Quad};
use super::ui::Scissor;
use super::{OrientedBounds2d, Texture2d, TextureRegion};
use crate::math::{Mat4, Vec2};

/// A textured, alpha-blended quad in 2D world space, with +Y up.
#[derive(Clone, Copy)]
pub struct Sprite2d<'a> {
    pub texture: &'a Texture2d,
    pub center: Vec2,
    pub size: Vec2,
    /// Counterclockwise rotation around the center, in radians.
    pub rotation: f32,
    /// Linear RGB and straight alpha multiplied into the sampled texture.
    pub tint: [f32; 4],
    pub region: TextureRegion,
}

impl<'a> Sprite2d<'a> {
    pub const fn new(texture: &'a Texture2d, center: Vec2, size: Vec2) -> Self {
        Self {
            texture,
            center,
            size,
            rotation: 0.0,
            tint: [1.0; 4],
            region: TextureRegion::FULL,
        }
    }

    pub const fn with_rotation(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }

    pub const fn with_tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }

    pub const fn with_region(mut self, region: TextureRegion) -> Self {
        self.region = region;
        self
    }

    /// The world-space quad for picking or visibility checks. This tests
    /// geometry only, not the texture's transparent texels.
    pub fn world_bounds(&self) -> Option<OrientedBounds2d> {
        OrientedBounds2d::from_center_size(self.center, self.size, self.rotation)
    }
}

/// Return top-left, top-right, bottom-right, bottom-left in clip space.
/// The projection is the axis-aligned affine matrix provided by `Camera2d`.
pub(super) fn project_corners(
    view_projection: Mat4,
    center: Vec2,
    size: Vec2,
    radians: f32,
) -> Option<[[f32; 2]; 4]> {
    let bounds = OrientedBounds2d::from_center_size(center, size, radians)?;
    let m = view_projection.to_cols_array();
    let clip_center = [m[0] * center.x + m[12], m[5] * center.y + m[13]];
    let corners = bounds
        .corner_offsets()
        .map(|[x, y]| [clip_center[0] + m[0] * x, clip_center[1] + m[5] * y]);
    corners
        .iter()
        .flatten()
        .all(|v| v.is_finite())
        .then_some(corners)
}

/// Build and upload the world-space sprite batch.
pub(super) fn write_sprites(
    batch: &mut ImageBatch,
    queue: &wgpu::Queue,
    surface: (u32, u32),
    view_projection: Mat4,
    sprites: &[Sprite2d<'_>],
) -> usize {
    let scissor = Scissor {
        x: 0,
        y: 0,
        width: surface.0,
        height: surface.1,
    };
    batch.write_quads(
        queue,
        sprites.iter().enumerate().filter_map(|(source, sprite)| {
            if !sprite.region.is_finite() || !sprite.tint.iter().all(|v| v.is_finite()) {
                return None;
            }
            let corners =
                project_corners(view_projection, sprite.center, sprite.size, sprite.rotation)?;
            Some(Quad {
                source,
                texture: sprite.texture,
                scissor,
                instance: ImageInstance::new(corners, sprite.region, sprite.tint),
            })
        }),
    )
}

/// Record the prepared sprite batch. Returns the number of draws recorded.
pub(super) fn draw_sprites(
    batch: &ImageBatch,
    pass: &mut wgpu::RenderPass<'_>,
    sprites: &[Sprite2d<'_>],
) -> usize {
    batch.draw_with(pass, |index| sprites[index].texture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::Camera2d;

    #[test]
    fn sprite_corners_follow_camera_and_rotate_counterclockwise() {
        let camera = Camera2d {
            center: Vec2::new(4.0, 2.0),
            zoom: 2.0,
            ..Default::default()
        };
        let vp = camera.view_projection(2.0);
        let corners = project_corners(vp, camera.center, Vec2::new(2.0, 2.0), 0.0).unwrap();
        let assert_point = |actual: [f32; 2], expected: [f32; 2]| {
            assert!((actual[0] - expected[0]).abs() < 1e-6, "{actual:?}");
            assert!((actual[1] - expected[1]).abs() < 1e-6, "{actual:?}");
        };
        assert_point(corners[0], [-0.2, 0.4]);
        assert_point(corners[2], [0.2, -0.4]);
        let rotated = project_corners(
            vp,
            camera.center + Vec2::new(1.0, 0.0),
            Vec2::new(2.0, 2.0),
            std::f32::consts::FRAC_PI_2,
        )
        .unwrap();
        assert_point(rotated[0], [0.0, -0.4]);
    }

    #[test]
    fn sprite_projection_rejects_invalid_geometry() {
        for (center, size, radians) in [
            (Vec2::ZERO, Vec2::ZERO, 0.0),
            (Vec2::ZERO, Vec2::new(f32::INFINITY, 1.0), 0.0),
            (Vec2::new(f32::NAN, 0.0), Vec2::ONE, 0.0),
            (Vec2::ZERO, Vec2::ONE, f32::INFINITY),
        ] {
            assert!(project_corners(Mat4::IDENTITY, center, size, radians).is_none());
        }
    }

    #[test]
    fn world_sprites_sample_atlas_and_blend_in_one_draw() {
        use crate::renderer::test_support::{
            read_quad_pixels, skip_without_adapter, test_device, test_image_texture,
        };
        use crate::renderer::{Camera2d, MAX_DRAWN_INSTANCES, TextureOptions};

        let Some((device, queue)) = test_device() else {
            skip_without_adapter();
            return;
        };
        let layout = crate::renderer::texture::texture_layout(&device);
        let images = ImageBatch::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb, &layout);
        let mut batch = images.for_sprites(&device);
        let atlas = test_image_texture(&device, &queue, &layout, TextureOptions::NEAREST);
        let camera = Camera2d {
            center: Vec2::new(5.0, -3.0),
            ..Default::default()
        };
        let red = Sprite2d::new(&atlas, camera.center, Vec2::new(10.0, 10.0))
            .with_region(atlas.region(0, 0, 1, 1));
        let green = Sprite2d::new(&atlas, camera.center, Vec2::new(4.0, 4.0))
            .with_region(atlas.region(1, 0, 1, 1))
            .with_tint([1.0, 1.0, 1.0, 0.75]);
        let invalid = Sprite2d::new(&atlas, camera.center, Vec2::ZERO);
        let sprites = [red, invalid, green];
        let side = 64;
        let vp = camera.view_projection(1.0);
        assert_eq!(
            write_sprites(&mut batch, &queue, (side, side), vp, &sprites),
            2
        );
        let (pixels, draws) = read_quad_pixels(&device, &queue, side, |pass| {
            draw_sprites(&batch, pass, &sprites)
        });
        assert_eq!(draws, 1);
        let at = |x, y| pixels[(side * y + x) as usize];
        assert_eq!(at(2, 32), [0, 0, 0, 255]);
        assert_eq!(at(20, 32), [255, 0, 0, 255]);
        let center = at(32, 32);
        assert!(
            center[1] > center[0] && center[0] > 120 && center[2] == 0,
            "{center:?}"
        );

        let over_budget = [red; MAX_DRAWN_INSTANCES + 1];
        assert_eq!(
            write_sprites(&mut batch, &queue, (side, side), vp, &over_budget),
            MAX_DRAWN_INSTANCES
        );
        assert_eq!(batch.run_count(), 1);
    }
}
