use crate::math::{Mat4, Vec2, Vec3};

const MIN_VIEW_SIZE: f32 = 1.0;
const MAX_VIEW_SIZE: f32 = 128.0;
const MIN_ASPECT: f32 = 0.25;
const MAX_ASPECT: f32 = 4.0;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 8.0;

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
    fn sanitize_aspect(aspect: f32) -> f32 {
        if !aspect.is_finite() {
            return 1.0;
        }

        let magnitude = aspect.abs();
        magnitude.clamp(MIN_ASPECT, MAX_ASPECT)
    }

    fn sanitize_zoom(zoom: f32) -> f32 {
        if !zoom.is_finite() {
            return 1.0;
        }

        let magnitude = zoom.abs();
        magnitude.clamp(MIN_ZOOM, MAX_ZOOM)
    }

    pub fn safe_visible_size(&self) -> Vec2 {
        let x = if self.view_size.x.is_finite() {
            self.view_size.x.abs().clamp(MIN_VIEW_SIZE, MAX_VIEW_SIZE)
        } else {
            20.0
        };
        let y = if self.view_size.y.is_finite() {
            self.view_size.y.abs().clamp(MIN_VIEW_SIZE, MAX_VIEW_SIZE)
        } else {
            20.0
        };

        Vec2::new(x, y)
    }

    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        let safe_aspect = Self::sanitize_aspect(aspect);
        let safe_zoom = Self::sanitize_zoom(self.zoom);
        let safe_view_size = self.safe_visible_size() * (1.0 / safe_zoom);

        let projection = Mat4::orthographic_2d(safe_view_size, safe_aspect);
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

    #[test]
    fn camera_clamps_extreme_view_and_zoom_values() {
        let camera = Camera2d {
            center: Vec2::ZERO,
            view_size: Vec2::new(1_000.0, 1_000.0),
            zoom: 0.01,
        };

        let size = camera.safe_visible_size();
        assert!(size.x <= MAX_VIEW_SIZE);
        assert!(size.y <= MAX_VIEW_SIZE);
        assert!(
            camera
                .view_projection(0.0)
                .m
                .iter()
                .all(|value| value.is_finite())
        );
        assert!(
            camera
                .view_projection(100.0)
                .m
                .iter()
                .all(|value| value.is_finite())
        );
    }
}
