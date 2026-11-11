use crate::math::{Mat4, Vec2, Vec3};
use crate::renderer::visibility::{Bounds2d, Frustum3d};

const MIN_VIEW_SIZE: f32 = 1.0;
const MAX_VIEW_SIZE: f32 = 128.0;
const MIN_ASPECT: f32 = 0.25;
const MAX_ASPECT: f32 = 4.0;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 8.0;
const MIN_FOV_Y: f32 = 10.0_f32.to_radians();
const MAX_FOV_Y: f32 = 120.0_f32.to_radians();
const MIN_NEAR: f32 = 0.01;
const MAX_NEAR: f32 = 1_000.0;
const MIN_CLIP_GAP: f32 = 0.01;
const MAX_FAR: f32 = 100_000.0;
const MAX_PITCH: f32 = 89.0_f32.to_radians();

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

    /// World-space bounds matching `view_projection(aspect)`.
    ///
    /// Consumers can use this to filter draw candidates before submitting them;
    /// the renderer does not cull automatically.
    pub fn visible_bounds(&self, aspect: f32) -> Bounds2d {
        let safe_aspect = Self::sanitize_aspect(aspect);
        let safe_zoom = Self::sanitize_zoom(self.zoom);
        let size = self.safe_visible_size() * (1.0 / safe_zoom);

        let (half_w, half_h) = if safe_aspect >= 1.0 {
            (size.x * 0.5, size.x * 0.5 / safe_aspect)
        } else {
            (size.y * 0.5 * safe_aspect, size.y * 0.5)
        };

        Bounds2d::new(self.center, Vec2::new(half_w, half_h))
    }
}

/// Perspective camera using +Y as world up and -Z as its forward direction
/// when yaw and pitch are zero.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct Camera3d {
    pub position: Vec3,
    /// Horizontal rotation in radians.
    pub yaw: f32,
    /// Vertical rotation in radians.
    pub pitch: f32,
    pub fov_y_radians: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera3d {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 1.6, 5.0),
            yaw: 0.0,
            pitch: 0.0,
            fov_y_radians: 75.0_f32.to_radians(),
            near: 0.1,
            far: 1_000.0,
        }
    }
}

#[allow(dead_code)]
impl Camera3d {
    fn finite_or(value: f32, fallback: f32) -> f32 {
        if value.is_finite() { value } else { fallback }
    }

    fn safe_position(&self) -> Vec3 {
        Vec3::new(
            Self::finite_or(self.position.x, 0.0),
            Self::finite_or(self.position.y, 0.0),
            Self::finite_or(self.position.z, 0.0),
        )
    }

    fn safe_yaw(&self) -> f32 {
        Self::finite_or(self.yaw, 0.0).rem_euclid(std::f32::consts::TAU)
    }

    fn safe_pitch(&self) -> f32 {
        Self::finite_or(self.pitch, 0.0).clamp(-MAX_PITCH, MAX_PITCH)
    }

    fn safe_fov_y(&self) -> f32 {
        Self::finite_or(self.fov_y_radians, 75.0_f32.to_radians()).clamp(MIN_FOV_Y, MAX_FOV_Y)
    }

    fn safe_clip_planes(&self) -> (f32, f32) {
        let near = Self::finite_or(self.near, 0.1).clamp(MIN_NEAR, MAX_NEAR);
        let far = Self::finite_or(self.far, 1_000.0).clamp(near + MIN_CLIP_GAP, MAX_FAR);
        (near, far)
    }

    fn safe_aspect(aspect: f32) -> f32 {
        Self::finite_or(aspect, 1.0)
            .abs()
            .clamp(MIN_ASPECT, MAX_ASPECT)
    }

    pub fn forward(&self) -> Vec3 {
        let yaw = self.safe_yaw();
        let pitch = self.safe_pitch();
        let cos_pitch = pitch.cos();
        Vec3::new(yaw.sin() * cos_pitch, pitch.sin(), -yaw.cos() * cos_pitch).normalize()
    }

    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        let position = self.safe_position();
        let view = Mat4::look_at(position, position + self.forward(), Vec3::Y);
        let (near, far) = self.safe_clip_planes();
        let projection = Mat4::perspective(self.safe_fov_y(), Self::safe_aspect(aspect), near, far);
        projection.mul(view)
    }

    /// View frustum matching `view_projection(aspect)`.
    ///
    /// Consumers can use this to filter draw candidates; the renderer does not
    /// cull automatically.
    pub fn frustum(&self, aspect: f32) -> Frustum3d {
        Frustum3d::from_view_projection(self.view_projection(aspect))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::visibility::Bounds3d;

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
    fn visible_bounds_follow_aspect_and_zoom() {
        let camera = Camera2d::default();
        assert_eq!(
            camera.visible_bounds(1.0),
            Bounds2d::new(Vec2::ZERO, Vec2::new(10.0, 10.0))
        );
        assert_eq!(camera.visible_bounds(2.0).half_extent, Vec2::new(10.0, 5.0));
        assert_eq!(camera.visible_bounds(0.5).half_extent, Vec2::new(5.0, 10.0));

        let zoomed = Camera2d {
            zoom: 2.0,
            ..Default::default()
        };
        assert_eq!(zoomed.visible_bounds(1.0).half_extent, Vec2::new(5.0, 5.0));
    }

    #[test]
    fn visible_bounds_corners_match_projection_edges() {
        let camera = Camera2d {
            center: Vec2::new(3.0, -4.0),
            view_size: Vec2::new(24.0, 12.0),
            zoom: 1.5,
        };

        for aspect in [0.25, 0.5, 1.0, 2.0, 4.0] {
            let view_projection = camera.view_projection(aspect);
            let bounds = camera.visible_bounds(aspect);
            let min = bounds.min();
            let max = bounds.max();

            for (x, y) in [
                (min.x, min.y),
                (max.x, min.y),
                (min.x, max.y),
                (max.x, max.y),
            ] {
                let clip_x =
                    view_projection.m[0] * x + view_projection.m[4] * y + view_projection.m[12];
                let clip_y =
                    view_projection.m[1] * x + view_projection.m[5] * y + view_projection.m[13];

                assert!((clip_x.abs() - 1.0).abs() < 1e-4, "aspect {aspect}");
                assert!((clip_y.abs() - 1.0).abs() < 1e-4, "aspect {aspect}");
            }
        }
    }

    #[test]
    fn visible_bounds_stay_finite_for_degenerate_camera_inputs() {
        let camera = Camera2d {
            center: Vec2::new(f32::NAN, f32::INFINITY),
            view_size: Vec2::ZERO,
            zoom: 0.0,
        };

        for aspect in [0.0, f32::NAN, f32::INFINITY, 100.0] {
            let bounds = camera.visible_bounds(aspect);
            assert!(bounds.center.x.is_finite());
            assert!(bounds.center.y.is_finite());
            assert!(bounds.half_extent.x.is_finite());
            assert!(bounds.half_extent.y.is_finite());
        }
    }

    #[test]
    fn camera_3d_frustum_culls_behind_side_and_far_boxes() {
        let camera = Camera3d::default();
        let frustum = camera.frustum(16.0 / 9.0);
        let box_at = |x, y, z| Bounds3d::new(Vec3::new(x, y, z), Vec3::ONE);

        assert!(frustum.intersects_bounds(box_at(0.0, 1.6, 0.0)));
        assert!(!frustum.intersects_bounds(box_at(0.0, 1.6, 10.0)));
        assert!(!frustum.intersects_bounds(box_at(100.0, 1.6, 0.0)));
        assert!(!frustum.intersects_bounds(box_at(0.0, 1.6, -2000.0)));
    }

    #[test]
    fn camera_3d_frustum_follows_aspect_ratio() {
        let camera = Camera3d::default();
        let bounds = Bounds3d::new(Vec3::new(20.0, 1.6, -10.0), Vec3::ONE);

        assert!(camera.frustum(4.0).intersects_bounds(bounds));
        assert!(!camera.frustum(0.25).intersects_bounds(bounds));
    }

    #[test]
    fn camera_3d_frustum_handles_degenerate_inputs() {
        let camera = Camera3d {
            position: Vec3::new(f32::NAN, f32::INFINITY, f32::NEG_INFINITY),
            yaw: f32::NAN,
            pitch: f32::INFINITY,
            fov_y_radians: 0.0,
            near: f32::NAN,
            far: 0.0,
        };

        for aspect in [0.0, f32::NAN, f32::INFINITY, 100.0] {
            let frustum = camera.frustum(aspect);
            let _ = frustum.contains_point(Vec3::ZERO);
            let _ = frustum.intersects_bounds(Bounds3d::ZERO);
            let _ = frustum.intersects_sphere(Vec3::ZERO, f32::NAN);
        }
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

    #[test]
    fn default_3d_camera_faces_negative_z() {
        let camera = Camera3d::default();
        assert!((camera.forward() - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5);

        let view_projection = camera.view_projection(16.0 / 9.0);
        assert!(view_projection.m.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn camera_3d_yaw_and_pitch_rotate_forward_direction() {
        let camera = Camera3d {
            yaw: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        assert!((camera.forward() - Vec3::X).length() < 1e-5);

        let camera = Camera3d {
            pitch: std::f32::consts::FRAC_PI_4,
            ..Default::default()
        };
        assert!((camera.forward().y - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    }

    #[test]
    fn camera_3d_sanitizes_invalid_projection_inputs() {
        let camera = Camera3d {
            position: Vec3::new(f32::NAN, f32::INFINITY, f32::NEG_INFINITY),
            yaw: f32::NAN,
            pitch: f32::INFINITY,
            fov_y_radians: 0.0,
            near: f32::NAN,
            far: 0.0,
        };

        for aspect in [0.0, f32::NAN, f32::INFINITY, 100.0] {
            let view_projection = camera.view_projection(aspect);
            assert!(view_projection.m.iter().all(|value| value.is_finite()));
        }
        assert!((camera.forward().length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn camera_3d_projection_maps_depth_to_wgpu_zero_one_range() {
        let camera = Camera3d {
            position: Vec3::ZERO,
            near: 0.1,
            far: 100.0,
            ..Default::default()
        };
        let view_projection = camera.view_projection(1.0);
        let projected_depth = |distance: f32| {
            let clip_z = view_projection.m[10] * -distance + view_projection.m[14];
            let clip_w = view_projection.m[11] * -distance;
            clip_z / clip_w
        };

        assert!(projected_depth(camera.near).abs() < 1e-5);
        assert!((projected_depth(camera.far) - 1.0).abs() < 1e-5);
    }
}
