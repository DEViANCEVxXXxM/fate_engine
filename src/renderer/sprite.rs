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
}
