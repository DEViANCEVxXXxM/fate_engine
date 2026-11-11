//! Render-facing 2D visibility primitives.

use crate::math::Vec2;

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
}
