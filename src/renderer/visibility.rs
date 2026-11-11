//! Render-facing 2D visibility primitives.

use crate::math::{Mat4, Vec2, Vec3};

/// Axis-aligned world-space bounds used for render visibility checks.
///
/// Construction sanitizes degenerate input: non-finite components become zero
/// and negative half extents are made positive. Scene traversal and the choice
/// of which objects to test remain the consumer's responsibility.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds2d {
    pub center: Vec2,
    pub half_extent: Vec2,
}

impl Bounds2d {
    pub const ZERO: Self = Self {
        center: Vec2::ZERO,
        half_extent: Vec2::ZERO,
    };

    pub fn new(center: Vec2, half_extent: Vec2) -> Self {
        Self {
            center: Vec2::new(finite_or(center.x, 0.0), finite_or(center.y, 0.0)),
            half_extent: Vec2::new(
                finite_or(half_extent.x, 0.0).abs(),
                finite_or(half_extent.y, 0.0).abs(),
            ),
        }
    }

    pub fn from_center_size(center: Vec2, size: Vec2) -> Self {
        Self::new(center, size * 0.5)
    }

    pub fn min(self) -> Vec2 {
        self.center - self.half_extent
    }

    pub fn max(self) -> Vec2 {
        self.center + self.half_extent
    }

    /// Returns true when `point` lies inside or on the bounds. Non-finite
    /// points are never contained.
    pub fn contains_point(self, point: Vec2) -> bool {
        (point.x - self.center.x).abs() <= self.half_extent.x
            && (point.y - self.center.y).abs() <= self.half_extent.y
    }

    /// Returns true when the two axis-aligned bounds overlap or touch.
    pub fn intersects(self, other: Self) -> bool {
        (self.center.x - other.center.x).abs() <= self.half_extent.x + other.half_extent.x
            && (self.center.y - other.center.y).abs() <= self.half_extent.y + other.half_extent.y
    }

    /// Grows both half extents by `margin`. Negative and non-finite margins are
    /// treated as their absolute value or zero.
    pub fn expanded(self, margin: f32) -> Self {
        let margin = finite_or(margin, 0.0).abs();
        Self::new(self.center, self.half_extent + Vec2::new(margin, margin))
    }
}

/// Axis-aligned 3D world-space bounds used for render visibility checks.
///
/// Construction sanitizes degenerate input the same way as [`Bounds2d`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds3d {
    pub center: Vec3,
    pub half_extent: Vec3,
}

impl Bounds3d {
    pub const ZERO: Self = Self {
        center: Vec3::ZERO,
        half_extent: Vec3::ZERO,
    };

    pub fn new(center: Vec3, half_extent: Vec3) -> Self {
        Self {
            center: Vec3::new(
                finite_or(center.x, 0.0),
                finite_or(center.y, 0.0),
                finite_or(center.z, 0.0),
            ),
            half_extent: Vec3::new(
                finite_or(half_extent.x, 0.0).abs(),
                finite_or(half_extent.y, 0.0).abs(),
                finite_or(half_extent.z, 0.0).abs(),
            ),
        }
    }

    pub fn from_center_size(center: Vec3, size: Vec3) -> Self {
        Self::new(center, size * 0.5)
    }

    pub fn min(self) -> Vec3 {
        self.center - self.half_extent
    }

    pub fn max(self) -> Vec3 {
        self.center + self.half_extent
    }

    /// Returns true when `point` lies inside or on the bounds. Non-finite
    /// points are never contained.
    pub fn contains_point(self, point: Vec3) -> bool {
        (point.x - self.center.x).abs() <= self.half_extent.x
            && (point.y - self.center.y).abs() <= self.half_extent.y
            && (point.z - self.center.z).abs() <= self.half_extent.z
    }

    /// Returns true when the two axis-aligned bounds overlap or touch.
    pub fn intersects(self, other: Self) -> bool {
        (self.center.x - other.center.x).abs() <= self.half_extent.x + other.half_extent.x
            && (self.center.y - other.center.y).abs() <= self.half_extent.y + other.half_extent.y
            && (self.center.z - other.center.z).abs() <= self.half_extent.z + other.half_extent.z
    }

    /// Grows both half extents by `margin`.
    pub fn expanded(self, margin: f32) -> Self {
        let margin = finite_or(margin, 0.0).abs();
        Self::new(
            self.center,
            self.half_extent + Vec3::new(margin, margin, margin),
        )
    }
}

/// View frustum extracted from a column-major view-projection matrix.
///
/// Depth is expected in the wgpu `0..=1` convention. Degenerate matrices
/// produce non-culling planes instead of panicking.
#[derive(Clone, Copy, Debug)]
pub struct Frustum3d {
    planes: [Plane; 6],
}

impl Frustum3d {
    pub fn from_view_projection(view_projection: Mat4) -> Self {
        let m = view_projection.m;
        Self {
            planes: [
                Plane::from_coefficients(m[3] + m[0], m[7] + m[4], m[11] + m[8], m[15] + m[12]),
                Plane::from_coefficients(m[3] - m[0], m[7] - m[4], m[11] - m[8], m[15] - m[12]),
                Plane::from_coefficients(m[3] + m[1], m[7] + m[5], m[11] + m[9], m[15] + m[13]),
                Plane::from_coefficients(m[3] - m[1], m[7] - m[5], m[11] - m[9], m[15] - m[13]),
                Plane::from_coefficients(m[2], m[6], m[10], m[14]),
                Plane::from_coefficients(m[3] - m[2], m[7] - m[6], m[11] - m[10], m[15] - m[14]),
            ],
        }
    }

    pub fn contains_point(self, point: Vec3) -> bool {
        self.planes
            .iter()
            .all(|plane| plane.signed_distance(point) >= 0.0)
    }

    /// Conservative test: true when any part of the bounds may be visible.
    pub fn intersects_bounds(self, bounds: Bounds3d) -> bool {
        self.planes.iter().all(|plane| {
            let radius = bounds.half_extent.x * plane.normal.x.abs()
                + bounds.half_extent.y * plane.normal.y.abs()
                + bounds.half_extent.z * plane.normal.z.abs();
            plane.signed_distance(bounds.center) + radius >= 0.0
        })
    }

    /// Conservative sphere test. Non-finite radii are treated as zero.
    pub fn intersects_sphere(self, center: Vec3, radius: f32) -> bool {
        let radius = finite_or(radius, 0.0).abs();
        self.planes
            .iter()
            .all(|plane| plane.signed_distance(center) + radius >= 0.0)
    }
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    normal: Vec3,
    distance: f32,
}

impl Plane {
    fn from_coefficients(a: f32, b: f32, c: f32, d: f32) -> Self {
        let normal = Vec3::new(a, b, c);
        let length = normal.length();

        if length > f32::EPSILON && length.is_finite() {
            Self {
                normal: normal / length,
                distance: d / length,
            }
        } else {
            // A degenerate plane must never cull valid geometry.
            Self {
                normal: Vec3::ZERO,
                distance: f32::INFINITY,
            }
        }
    }

    fn signed_distance(self, point: Vec3) -> f32 {
        self.normal.dot(point) + self.distance
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_point_includes_edges_and_rejects_outside() {
        let bounds = Bounds2d::new(Vec2::new(5.0, -2.0), Vec2::new(2.0, 1.0));

        assert!(bounds.contains_point(Vec2::new(5.0, -2.0)));
        assert!(bounds.contains_point(Vec2::new(7.0, -1.0)));
        assert!(bounds.contains_point(Vec2::new(3.0, -3.0)));
        assert!(!bounds.contains_point(Vec2::new(7.1, -2.0)));
        assert!(!bounds.contains_point(Vec2::new(5.0, -3.1)));
        assert!(!bounds.contains_point(Vec2::new(f32::NAN, -2.0)));
    }

    #[test]
    fn intersects_detects_overlap_and_touching_edges() {
        let bounds = Bounds2d::new(Vec2::ZERO, Vec2::new(1.0, 1.0));

        assert!(bounds.intersects(Bounds2d::new(Vec2::new(1.5, 0.0), Vec2::ONE)));
        assert!(bounds.intersects(Bounds2d::new(Vec2::new(2.0, 0.0), Vec2::ONE)));
        assert!(!bounds.intersects(Bounds2d::new(Vec2::new(2.1, 0.0), Vec2::ONE)));
        assert!(!bounds.intersects(Bounds2d::new(Vec2::new(0.0, 2.1), Vec2::ONE)));
    }

    #[test]
    fn construction_and_expansion_sanitize_degenerate_inputs() {
        let bounds = Bounds2d::new(
            Vec2::new(f32::NAN, f32::INFINITY),
            Vec2::new(-3.0, f32::NAN),
        );

        assert_eq!(bounds.center, Vec2::ZERO);
        assert_eq!(bounds.half_extent, Vec2::new(3.0, 0.0));
        assert_eq!(bounds.expanded(f32::NAN).half_extent, bounds.half_extent);
        assert_eq!(bounds.expanded(-5.0).half_extent, Vec2::new(8.0, 5.0));
        assert!(Bounds2d::ZERO.contains_point(Vec2::ZERO));
        assert!(!Bounds2d::ZERO.contains_point(Vec2::new(0.1, 0.0)));
    }

    #[test]
    fn bounds3d_contains_edges_and_detects_overlap() {
        let bounds = Bounds3d::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(2.0, 1.0, 0.5));

        assert!(bounds.contains_point(Vec3::new(3.0, 3.0, 3.5)));
        assert!(bounds.contains_point(Vec3::new(-1.0, 1.0, 2.5)));
        assert!(!bounds.contains_point(Vec3::new(3.1, 2.0, 3.0)));
        assert!(!bounds.contains_point(Vec3::new(1.0, 2.0, f32::NAN)));

        assert!(bounds.intersects(Bounds3d::new(Vec3::new(3.5, 2.0, 3.0), Vec3::ONE)));
        assert!(bounds.intersects(Bounds3d::new(Vec3::new(4.0, 2.0, 3.0), Vec3::ONE)));
        assert!(!bounds.intersects(Bounds3d::new(Vec3::new(4.1, 2.0, 3.0), Vec3::ONE)));
        assert!(!bounds.intersects(Bounds3d::new(Vec3::new(1.0, 2.0, 4.6), Vec3::ONE)));
    }

    #[test]
    fn bounds3d_sanitizes_degenerate_input() {
        let bounds = Bounds3d::new(
            Vec3::new(f32::NAN, 0.0, f32::INFINITY),
            Vec3::new(-1.0, f32::NAN, -2.0),
        );

        assert_eq!(bounds.center, Vec3::ZERO);
        assert_eq!(bounds.half_extent, Vec3::new(1.0, 0.0, 2.0));
        assert_eq!(bounds.expanded(f32::NAN).half_extent, bounds.half_extent);
        assert_eq!(bounds.expanded(-1.0).half_extent, Vec3::new(2.0, 1.0, 3.0));
    }

    #[test]
    fn frustum_from_identity_uses_wgpu_depth_range() {
        let frustum = Frustum3d::from_view_projection(Mat4::IDENTITY);

        assert!(frustum.contains_point(Vec3::new(0.0, 0.0, 0.5)));
        assert!(frustum.contains_point(Vec3::new(0.9, -0.9, 0.0)));
        assert!(frustum.contains_point(Vec3::new(0.0, 0.0, 1.0)));
        assert!(!frustum.contains_point(Vec3::new(0.0, 0.0, -0.01)));
        assert!(!frustum.contains_point(Vec3::new(0.0, 0.0, 1.01)));
        assert!(!frustum.contains_point(Vec3::new(1.01, 0.0, 0.5)));
    }

    #[test]
    fn frustum_bounds_test_is_conservative_for_partial_overlap() {
        let frustum = Frustum3d::from_view_projection(Mat4::IDENTITY);

        assert!(frustum.intersects_bounds(Bounds3d::new(Vec3::new(1.5, 0.0, 0.5), Vec3::ONE)));
        assert!(!frustum.intersects_bounds(Bounds3d::new(Vec3::new(2.5, 0.0, 0.5), Vec3::ONE)));
        assert!(frustum.intersects_sphere(Vec3::new(1.5, 0.0, 0.5), 1.0));
        assert!(!frustum.intersects_sphere(Vec3::new(2.5, 0.0, 0.5), 1.0));
    }
}
