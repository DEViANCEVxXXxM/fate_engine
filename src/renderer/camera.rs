use crate::math::{Mat4, Vec2, Vec3};

/// 2D camera: a world-space size plus a center position.
///
/// The projection is rebuilt every frame from the window aspect, so resizing
/// never stretches the scene.
#[derive(Clone, Copy, Debug)]
pub struct Camera2d {
    pub center: Vec2,
    /// Visible world size. The larger axis is kept fixed, the other follows the
    /// window aspect ratio.
    pub view_size: Vec2,
    pub zoom: f32,
}

impl Default for Camera2d {
    fn default() -> Self {
        Self {
            center: Vec2::ZERO,
            view_size: Vec2::new(20.0, 20.0),
            zoom: 1.0,
        }
    }
}

impl Camera2d {
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        let safe_aspect = aspect.abs().max(f32::EPSILON);
        let safe_zoom = self.zoom.abs().max(f32::EPSILON);
        let safe_view_size = Vec2::new(
            self.view_size.x.abs().max(f32::EPSILON),
            self.view_size.y.abs().max(f32::EPSILON),
        );

        let size = safe_view_size * (1.0 / safe_zoom);
        let projection = Mat4::orthographic_2d(size, safe_aspect);
        let view = Mat4::translation(Vec3::new(-self.center.x, -self.center.y, 0.0));
        projection.mul(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_of_view_maps_to_origin() {
        let camera = Camera2d {
            center: Vec2::new(5.0, 5.0),
            ..Default::default()
        };
        let vp = camera.view_projection(1.0);
        // 20 world units across -> 0.1 clip units per world unit, so the
        // camera offset of 5 lands at -0.5 in clip space.
        assert!((vp.m[12] + 0.5).abs() < 1e-5);
        assert!((vp.m[13] + 0.5).abs() < 1e-5);
    }

    #[test]
    fn zero_zoom_and_aspect_still_produce_finite_projection() {
        let camera = Camera2d {
            center: Vec2::ZERO,
            view_size: Vec2::ZERO,
            zoom: 0.0,
        };
        let vp = camera.view_projection(0.0);
        assert!(vp.m.iter().all(|value| value.is_finite()));
    }
}
