//! Ordered broad-phase frustum queries for retained candidate bounds.

use super::{Bounds3d, Frustum3d};

const GROUP_SIZE: usize = 16;

/// Work performed by one [`VisibilitySet3d::query`] call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisibilityStats3d {
    /// Number of group bounds tested.
    pub group_tests: usize,
    /// Number of individual candidates tested.
    pub bounds_tests: usize,
}

/// CPU-only frustum culling for a retained list of world-space bounds.
///
/// Groups are contiguous in input order; supplying nearby candidates
/// together makes queries more selective. Rebuild the set when bounds or
/// input order change. The renderer does not own or maintain this set.
#[derive(Default)]
pub struct VisibilitySet3d {
    bounds: Box<[Bounds3d]>,
    groups: Box<[Option<GroupBounds>]>,
}

impl VisibilitySet3d {
    /// Copy bounds in consumer draw order. Empty input is valid.
    pub fn new(bounds: impl IntoIterator<Item = Bounds3d>) -> Self {
        let bounds: Box<[_]> = bounds.into_iter().collect();
        let groups = bounds
            .chunks(GROUP_SIZE)
            .map(GroupBounds::enclose)
            .collect();
        Self { bounds, groups }
    }

    /// Replace `visible` with original input indices in ascending order,
    /// matching an individual [`Frustum3d::intersects_bounds`] scan.
    /// Reuse the vector to avoid allocating across queries.
    pub fn query(&self, frustum: Frustum3d, visible: &mut Vec<usize>) -> VisibilityStats3d {
        visible.clear();
        let mut stats = VisibilityStats3d::default();
        for (i, (bounds, group)) in self.bounds.chunks(GROUP_SIZE).zip(&self.groups).enumerate() {
            let start = i * GROUP_SIZE;
            if let Some(group) = group {
                stats.group_tests += 1;
                match group.classify(frustum) {
                    Relation::Outside => continue,
                    Relation::Inside => {
                        visible.extend(start..start + bounds.len());
                        continue;
                    }
                    Relation::Partial => {}
                }
            }
            stats.bounds_tests += bounds.len();
            visible.extend(bounds.iter().enumerate().filter_map(|(offset, &bounds)| {
                frustum.intersects_bounds(bounds).then_some(start + offset)
            }));
        }
        stats
    }
}

struct GroupBounds {
    min: [f64; 3],
    max: [f64; 3],
}

enum Relation {
    Inside,
    Outside,
    Partial,
}

impl GroupBounds {
    fn enclose(bounds: &[Bounds3d]) -> Option<Self> {
        let mut group = Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        };
        for &bounds in bounds {
            let center = bounds.center.to_array().map(f64::from);
            let extent = bounds.half_extent.to_array().map(f64::from);
            for axis in 0..3 {
                // Bounds3d's public fields may have bypassed its constructor.
                // Leave such groups to the same checks as a direct scan.
                if !center[axis].is_finite() || !extent[axis].is_finite() || extent[axis] < 0.0 {
                    return None;
                }
                group.min[axis] = group.min[axis].min(center[axis] - extent[axis]);
                group.max[axis] = group.max[axis].max(center[axis] + extent[axis]);
            }
        }
        Some(group)
    }

    fn classify(&self, frustum: Frustum3d) -> Relation {
        let mut inside = true;
        for plane in frustum.planes {
            let d = f64::from(plane.distance);
            // Ignore disabled planes. For any other non-finite coefficient,
            // defer to the per-candidate test.
            if d == f64::INFINITY && plane.normal == crate::math::Vec3::ZERO {
                continue;
            }
            let normal = plane.normal.to_array().map(f64::from);
            let (mut near, mut far, mut magnitude) = (d, d, d.abs());
            for (axis, n) in normal.into_iter().enumerate() {
                let a = self.min[axis] * n;
                let b = self.max[axis] * n;
                near += a.min(b);
                far += a.max(b);
                magnitude += a.abs().max(b.abs());
            }
            if !d.is_finite()
                || !normal.into_iter().all(f64::is_finite)
                || magnitude > f64::from(f32::MAX) * 0.5
            {
                inside = false;
                continue;
            }
            // The individual center/radius test uses f32 arithmetic. Don't
            // short-circuit when rounding could put a candidate across a plane.
            let margin =
                8.0 * f64::from(f32::EPSILON) * magnitude + 8.0 * f64::from(f32::MIN_POSITIVE);
            if far < -margin {
                return Relation::Outside;
            }
            inside &= near > margin;
        }
        if inside {
            Relation::Inside
        } else {
            Relation::Partial
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Mat4, Vec3};
    use crate::renderer::Camera3d;

    fn scan(frustum: Frustum3d, bounds: &[Bounds3d]) -> Vec<usize> {
        bounds
            .iter()
            .enumerate()
            .filter_map(|(i, &bound)| frustum.intersects_bounds(bound).then_some(i))
            .collect()
    }

    #[test]
    fn groups_accept_and_reject_while_preserving_order() {
        let frustum = Frustum3d::from_view_projection(Mat4::IDENTITY);
        let bounds: Vec<_> = (0..GROUP_SIZE * 2 + 1)
            .map(|i| {
                let x = if i < GROUP_SIZE { 0.0 } else { 3.0 };
                Bounds3d::new(Vec3::new(x, 0.0, 0.5), Vec3::ONE * 0.1)
            })
            .collect();
        let set = VisibilitySet3d::new(bounds.iter().copied());
        let mut visible = vec![usize::MAX];
        let stats = set.query(frustum, &mut visible);
        assert_eq!(visible, scan(frustum, &bounds));
        assert_eq!(visible.len(), GROUP_SIZE);
        assert_eq!(
            stats,
            VisibilityStats3d {
                group_tests: 3,
                bounds_tests: 0
            }
        );
        assert_eq!(
            VisibilitySet3d::default().query(frustum, &mut visible),
            VisibilityStats3d::default()
        );
        assert!(visible.is_empty());
    }

    #[test]
    fn partial_groups_match_scan_for_camera_motion_and_boundary_values() {
        let mut bounds: Vec<_> = (0..2049)
            .map(|i| {
                let x = (i % 64) as f32 - 32.0;
                let y = (i / 64) as f32 - 16.0;
                Bounds3d::new(Vec3::new(x, y, -8.0), Vec3::ONE * 0.4)
            })
            .collect();
        bounds.extend([
            Bounds3d::new(Vec3::new(1.0, 1.0, 1.0), Vec3::ZERO),
            Bounds3d::new(
                Vec3::new(f32::MAX, f32::MAX, -f32::MAX),
                Vec3::ONE * f32::MAX,
            ),
        ]);
        bounds.resize(bounds.len().next_multiple_of(GROUP_SIZE), Bounds3d::ZERO);
        bounds.extend([
            Bounds3d {
                center: Vec3::new(f32::NAN, 0.0, 0.0),
                half_extent: Vec3::ONE,
            },
            Bounds3d {
                center: Vec3::ZERO,
                half_extent: -Vec3::ONE,
            },
        ]);
        let set = VisibilitySet3d::new(bounds.iter().copied());
        let mut visible = Vec::new();
        for i in -12..=12 {
            let camera = Camera3d {
                position: Vec3::new(i as f32, 3.0, 3.0),
                yaw: i as f32 * 0.09,
                ..Default::default()
            };
            let frustum = camera.frustum(1.3);
            set.query(frustum, &mut visible);
            assert_eq!(visible, scan(frustum, &bounds));
        }
        let frustum = Frustum3d::from_view_projection(Mat4::IDENTITY);
        set.query(frustum, &mut visible);
        assert_eq!(visible, scan(frustum, &bounds));
        let disabled = Frustum3d::from_view_projection(Mat4 { m: [0.0; 16] });
        set.query(disabled, &mut visible);
        assert_eq!(visible, scan(disabled, &bounds));
    }
}
