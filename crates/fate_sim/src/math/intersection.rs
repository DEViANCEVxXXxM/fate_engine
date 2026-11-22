use super::{Fixed, FixedVec3};

/// First forward intersection of a ray with the surface of a sphere.
///
/// The result is the parameter `t >= 0` in `origin + direction * t`, not a
/// distance unless `direction` is normalized. A ray starting inside hits the
/// exit surface; tangent and on-surface hits count.
///
/// Negative radii and directions whose squared length rounds to zero are
/// rejected. Inputs are subject to Q32.32 rounding; their intermediate
/// offsets, squared lengths, and parameters must fit in `Fixed`.
pub fn ray_sphere(
    origin: FixedVec3,
    direction: FixedVec3,
    center: FixedVec3,
    radius: Fixed,
) -> Option<Fixed> {
    let offset = center - origin;
    let direction_squared = direction.length_squared();
    if radius < Fixed::ZERO || direction_squared == Fixed::ZERO {
        return None;
    }
    let radius_squared = radius * radius;
    if offset.length_squared() == radius_squared {
        return Some(Fixed::ZERO);
    }
    let (entry, exit) = sphere_roots(offset, direction, direction_squared, radius_squared)?;
    if entry >= Fixed::ZERO {
        Some(entry)
    } else if exit >= Fixed::ZERO {
        Some(exit)
    } else {
        None
    }
}

/// First contact of a closed segment with a solid sphere.
///
/// Returns the fraction `t` in `start + (end - start) * t`, within `0..=1`.
/// An already overlapping start returns zero, including a stationary segment
/// on or inside the sphere. A negative radius is rejected. The same fixed
/// precision and range considerations as [`ray_sphere`] apply.
pub fn segment_sphere_contact(
    start: FixedVec3,
    end: FixedVec3,
    center: FixedVec3,
    radius: Fixed,
) -> Option<Fixed> {
    if radius < Fixed::ZERO {
        return None;
    }
    let offset = center - start;
    let radius_squared = radius * radius;
    if offset.length_squared() <= radius_squared {
        return Some(Fixed::ZERO);
    }
    let direction = end - start;
    let direction_squared = direction.length_squared();
    let entry = if direction_squared == Fixed::ZERO {
        None
    } else {
        sphere_roots(offset, direction, direction_squared, radius_squared)
            .map(|(entry, _)| entry)
            .filter(|entry| (Fixed::ZERO..=Fixed::ONE).contains(entry))
    };
    // Keep the closed endpoint even when the root rounds past it.
    entry.or_else(|| ((center - end).length_squared() <= radius_squared).then_some(Fixed::ONE))
}

/// First contact of a closed segment with an axis-aligned box, with the face
/// normal of the first crossing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AabbContact {
    /// Fraction along the segment (`0..=1`), rounded down to Q32.32.
    pub fraction: Fixed,
    /// Outward unit normal of the crossed face, as an axis vector. It is zero
    /// when the segment starts on or inside the box. Corner and edge ties
    /// choose the first axis in `x`, `y`, `z` order.
    pub normal: FixedVec3,
}

/// First contact of a closed segment with an axis-aligned box.
///
/// This is the fraction-only form of [`segment_aabb_hit`]; use that function
/// when a consumer needs the face direction.
pub fn segment_aabb_contact(
    start: FixedVec3,
    end: FixedVec3,
    min: FixedVec3,
    max: FixedVec3,
) -> Option<Fixed> {
    segment_aabb_hit(start, end, min, max).map(|hit| hit.fraction)
}

/// First contact of a closed segment with an axis-aligned box, including the
/// crossed face normal.
///
/// Pass the box's minimum and maximum corners in world space. Reversed
/// bounds on any axis return `None`. Touching a face, edge, or corner counts;
/// a start on or inside the box returns fraction zero with a zero normal,
/// even for a stationary segment.
///
/// The fraction is rounded down to Q32.32. Hit/miss is decided before
/// rounding, over the full endpoint range of `Fixed`. The result is a
/// geometric query, not a collision response.
pub fn segment_aabb_hit(
    start: FixedVec3,
    end: FixedVec3,
    min: FixedVec3,
    max: FixedVec3,
) -> Option<AabbContact> {
    for (min, max) in [(min.x, max.x), (min.y, max.y), (min.z, max.z)] {
        if min > max {
            return None;
        }
    }
    // Starting on or inside the box has no first-crossing face to report.
    let inside = |value: Fixed, min: Fixed, max: Fixed| min <= value && value <= max;
    if inside(start.x, min.x, max.x)
        && inside(start.y, min.y, max.y)
        && inside(start.z, min.z, max.z)
    {
        return Some(AabbContact {
            fraction: Fixed::ZERO,
            normal: FixedVec3::ZERO,
        });
    }

    let mut entry = (0, 1);
    let mut exit = (1, 1);
    let mut face = (0, false);
    for (axis, (start, end, min, max)) in [
        (start.x, end.x, min.x, max.x),
        (start.y, end.y, min.y, max.y),
        (start.z, end.z, min.z, max.z),
    ]
    .into_iter()
    .enumerate()
    {
        let raw = |value: Fixed| i128::from(value.raw());
        let flipped = start > end;
        let (start, end, min, max) = if flipped {
            (-raw(start), -raw(end), -raw(max), -raw(min))
        } else {
            (raw(start), raw(end), raw(min), raw(max))
        };
        if start > max || end < min {
            return None;
        }
        if start == end {
            continue;
        }
        // Both clipped endpoints fall within [start, end], so differences
        // are nonnegative and fit u64 even across the full raw i64 range.
        let length = (end - start) as u64;
        let axis_entry = ((min.max(start) - start) as u64, length);
        let axis_exit = ((max.min(end) - start) as u64, length);
        if fraction_lt(entry, axis_entry) {
            entry = axis_entry;
            // Entering through the minimum face is a negative normal; a
            // flipped axis enters through the original maximum face.
            face = (axis, flipped);
        }
        if fraction_lt(axis_exit, exit) {
            exit = axis_exit;
        }
        if fraction_lt(exit, entry) {
            return None;
        }
    }
    let (num, den) = entry;
    let raw = u128::from(num) * Fixed::ONE.raw() as u128 / u128::from(den);
    let sign = if face.1 { Fixed::ONE } else { -Fixed::ONE };
    let mut normal = FixedVec3::ZERO;
    match face.0 {
        0 => normal.x = sign,
        1 => normal.y = sign,
        _ => normal.z = sign,
    }
    Some(AabbContact {
        fraction: Fixed::from_raw(raw as i64),
        normal,
    })
}

fn fraction_lt((a, b): (u64, u64), (c, d): (u64, u64)) -> bool {
    // Products of two u64 values fit in u128; no signed-product overflow.
    u128::from(a) * u128::from(d) < u128::from(c) * u128::from(b)
}

fn sphere_roots(
    offset: FixedVec3,
    direction: FixedVec3,
    direction_squared: Fixed,
    radius_squared: Fixed,
) -> Option<(Fixed, Fixed)> {
    let midpoint = offset.dot(direction) / direction_squared;
    // Measure from the closest point directly to avoid subtracting two
    // potentially large squared distances for a distant, nearly aligned ray.
    let perpendicular = offset - direction * midpoint;
    let chord_squared = radius_squared - perpendicular.length_squared();
    if chord_squared < Fixed::ZERO {
        return None;
    }
    let extent = chord_squared.sqrt() / direction_squared.sqrt();
    Some((midpoint - extent, midpoint + extent))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: i64, y: i64, z: i64) -> FixedVec3 {
        FixedVec3::new(Fixed::from_int(x), Fixed::from_int(y), Fixed::from_int(z))
    }

    #[test]
    fn box_contact_includes_faces_corners_and_closed_endpoints() {
        let min = v(2, -1, -1);
        let max = v(3, 1, 1);
        let hit = |start, end| segment_aabb_contact(start, end, min, max);
        assert_eq!(hit(v(0, 0, 0), v(4, 0, 0)), Some(Fixed::from_ratio(1, 2)));
        assert_eq!(hit(v(5, 1, 1), v(1, 1, 1)), Some(Fixed::from_ratio(1, 2)));
        assert_eq!(hit(v(1, 0, 0), v(2, 0, 0)), Some(Fixed::ONE));
        assert_eq!(hit(v(2, 1, 1), v(1, 1, 1)), Some(Fixed::ZERO));
        assert_eq!(hit(v(2, 1, 1), v(2, 1, 1)), Some(Fixed::ZERO));
        assert_eq!(hit(v(1, 0, 0), v(1, 0, 0)), None);
        assert_eq!(hit(v(0, 2, 0), v(4, 2, 0)), None);
        assert_eq!(hit(v(0, 0, 0), v(0, 4, 0)), None);
        assert_eq!(segment_aabb_contact(v(0, 0, 0), v(4, 0, 0), max, min), None);
        assert_eq!(
            segment_aabb_contact(v(0, 0, 0), v(4, 2, 2), v(2, 1, 1), v(2, 1, 1)),
            Some(Fixed::from_ratio(1, 2))
        );
    }

    #[test]
    fn box_hit_reports_the_face_normal_of_the_first_crossing() {
        let min = v(2, -1, -1);
        let max = v(3, 1, 1);
        let hit = |start, end| segment_aabb_hit(start, end, min, max).unwrap();
        assert_eq!(
            hit(v(0, 0, 0), v(4, 0, 0)),
            AabbContact {
                fraction: Fixed::from_ratio(1, 2),
                normal: v(-1, 0, 0)
            }
        );
        assert_eq!(
            hit(v(5, 0, 0), v(1, 0, 0)),
            AabbContact {
                fraction: Fixed::from_ratio(1, 2),
                normal: v(1, 0, 0)
            }
        );
        assert_eq!(
            hit(v(2, -3, 0), v(2, 3, 0)),
            AabbContact {
                fraction: Fixed::from_ratio(1, 3),
                normal: v(0, -1, 0)
            }
        );
        assert_eq!(
            hit(v(2, 3, 0), v(2, -3, 0)),
            AabbContact {
                fraction: Fixed::from_ratio(1, 3),
                normal: v(0, 1, 0)
            }
        );
        assert_eq!(
            hit(v(2, 0, -3), v(2, 0, 3)),
            AabbContact {
                fraction: Fixed::from_ratio(1, 3),
                normal: v(0, 0, -1)
            }
        );
        // A corner tie resolves to the first axis.
        assert_eq!(
            segment_aabb_hit(v(0, 0, 0), v(4, 4, 0), v(2, 2, 0), v(3, 3, 1)),
            Some(AabbContact {
                fraction: Fixed::from_ratio(1, 2),
                normal: v(-1, 0, 0)
            })
        );
        // Starting on or inside the box has no first-crossing face.
        assert_eq!(
            hit(v(2, 0, 0), v(4, 0, 0)),
            AabbContact {
                fraction: Fixed::ZERO,
                normal: FixedVec3::ZERO
            }
        );
        assert_eq!(
            hit(v(2, 1, 1), v(2, 1, 1)),
            AabbContact {
                fraction: Fixed::ZERO,
                normal: FixedVec3::ZERO
            }
        );
        assert_eq!(segment_aabb_hit(v(0, 0, 0), v(4, 0, 0), max, min), None);
        assert_eq!(
            segment_aabb_contact(v(0, 0, 0), v(4, 0, 0), min, max),
            Some(hit(v(0, 0, 0), v(4, 0, 0)).fraction)
        );
    }

    #[test]
    fn box_contact_compares_intervals_before_rounding() {
        let raw =
            |x, y, z| FixedVec3::new(Fixed::from_raw(x), Fixed::from_raw(y), Fixed::from_raw(z));
        let zero = FixedVec3::ZERO;
        let high = raw(i64::MAX, i64::MAX, 0);
        assert_eq!(
            segment_aabb_contact(zero, high, raw(2, 5, 0), raw(4, 6, 0)),
            None
        );
        assert_eq!(
            segment_aabb_contact(zero, high, raw(2, 4, 0), raw(4, 6, 0)),
            Some(Fixed::ZERO)
        );

        // Endpoints span more than a representable Fixed delta. Exact ratios
        // still work, and reversing the segment changes the rounded fraction.
        let lo = raw(i64::MIN, i64::MIN, i64::MIN);
        let hi = raw(i64::MAX, i64::MAX, i64::MAX);
        let half = Fixed::from_ratio(1, 2);
        assert_eq!(segment_aabb_contact(lo, hi, zero, zero), Some(half));
        assert_eq!(
            segment_aabb_contact(hi, lo, zero, zero),
            Some(half - Fixed::from_raw(1))
        );
    }

    #[test]
    fn ray_returns_a_parameter_for_non_unit_directions() {
        let origin = FixedVec3::ZERO;
        let center = v(5, 0, 0);
        assert_eq!(
            ray_sphere(origin, v(2, 0, 0), center, Fixed::ONE),
            Some(Fixed::from_int(2))
        );
        assert_eq!(ray_sphere(origin, v(-2, 0, 0), center, Fixed::ONE), None);
        assert_eq!(ray_sphere(origin, origin, center, Fixed::ONE), None);
        assert_eq!(ray_sphere(origin, v(2, 0, 0), center, -Fixed::ONE), None);
    }

    #[test]
    fn ray_handles_tangency_surface_start_and_inside_exit() {
        let origin = FixedVec3::ZERO;
        let dir = v(1, 0, 0);
        assert_eq!(
            ray_sphere(origin, dir, v(10, 1, 0), Fixed::ONE),
            Some(Fixed::from_int(10))
        );
        assert_eq!(ray_sphere(origin, dir, v(10, 2, 0), Fixed::ONE), None);
        assert_eq!(
            ray_sphere(origin, dir, v(-1, 0, 0), Fixed::ONE),
            Some(Fixed::ZERO)
        );
        assert_eq!(
            ray_sphere(origin, dir, v(-1, 0, 0), Fixed::from_int(2)),
            Some(Fixed::ONE)
        );
    }

    #[test]
    fn an_oblique_ray_detects_a_tangent_without_normalizing_the_direction() {
        assert_eq!(
            ray_sphere(FixedVec3::ZERO, v(3, 4, 0), v(2, 11, 0), Fixed::from_int(5)),
            Some(Fixed::from_int(2))
        );
    }

    #[test]
    fn segment_detects_crossing_between_two_outside_endpoints() {
        let origin = FixedVec3::ZERO;
        let center = v(5, 0, 0);
        assert_eq!(
            segment_sphere_contact(origin, v(8, 0, 0), center, Fixed::ONE),
            Some(Fixed::from_ratio(1, 2))
        );
        assert_eq!(
            segment_sphere_contact(origin, v(4, 0, 0), center, Fixed::ONE),
            Some(Fixed::ONE)
        );
        assert_eq!(
            segment_sphere_contact(origin, v(3, 0, 0), center, Fixed::ONE),
            None
        );
        assert_eq!(
            segment_sphere_contact(origin, v(-8, 0, 0), center, Fixed::ONE),
            None
        );
    }

    #[test]
    fn stationary_and_overlapping_segments_have_defined_results() {
        let origin = FixedVec3::ZERO;
        assert_eq!(
            segment_sphere_contact(origin, origin, origin, Fixed::ZERO),
            Some(Fixed::ZERO)
        );
        assert_eq!(
            segment_sphere_contact(origin, origin, v(2, 0, 0), Fixed::ONE),
            None
        );
        assert_eq!(
            segment_sphere_contact(origin, v(4, 0, 0), origin, Fixed::ONE),
            Some(Fixed::ZERO)
        );
        assert_eq!(
            segment_sphere_contact(origin, origin, origin, -Fixed::ONE),
            None
        );
    }
}
