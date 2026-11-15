use std::ops::Mul;

use crate::math::{Fixed, FixedQuat, FixedVec3};

/// A 3D spatial transform in fixed-point (Q32.32) composed of translation, rotation, and uniform scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FixedTransform {
    pub translation: FixedVec3,
    pub rotation: FixedQuat,
    pub scale: Fixed,
}

impl Default for FixedTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl FixedTransform {
    pub const IDENTITY: Self = Self {
        translation: FixedVec3::ZERO,
        rotation: FixedQuat::IDENTITY,
        scale: Fixed::ONE,
    };

    pub const fn new(translation: FixedVec3, rotation: FixedQuat, scale: Fixed) -> Self {
        Self {
            translation,
            rotation,
            scale,
        }
    }

    pub const fn from_translation(translation: FixedVec3) -> Self {
        Self {
            translation,
            rotation: FixedQuat::IDENTITY,
            scale: Fixed::ONE,
        }
    }

    pub const fn from_rotation(rotation: FixedQuat) -> Self {
        Self {
            translation: FixedVec3::ZERO,
            rotation,
            scale: Fixed::ONE,
        }
    }

    pub const fn from_translation_rotation(translation: FixedVec3, rotation: FixedQuat) -> Self {
        Self {
            translation,
            rotation,
            scale: Fixed::ONE,
        }
    }

    /// Transform a 3D point (applies scale, rotation, then translation).
    pub fn transform_point(self, point: FixedVec3) -> FixedVec3 {
        self.translation + self.rotation.rotate_vec3(point * self.scale)
    }

    /// Transform a 3D vector (applies scale and rotation, but not translation).
    pub fn transform_vector(self, vector: FixedVec3) -> FixedVec3 {
        self.rotation.rotate_vec3(vector * self.scale)
    }

    /// Inverse transform. Returns IDENTITY if scale is zero.
    pub fn inverse(self) -> Self {
        if self.scale == Fixed::ZERO {
            return Self::IDENTITY;
        }
        let inv_scale = Fixed::ONE / self.scale;
        let inv_rot = self.rotation.conjugate();
        let inv_translation = inv_rot.rotate_vec3(-self.translation) * inv_scale;
        Self {
            translation: inv_translation,
            rotation: inv_rot,
            scale: inv_scale,
        }
    }

    /// Combine two transforms: `self * rhs` (applies `rhs` first, then `self`).
    pub fn mul_transform(self, rhs: Self) -> Self {
        Self {
            translation: self.transform_point(rhs.translation),
            rotation: self.rotation * rhs.rotation,
            scale: self.scale * rhs.scale,
        }
    }
}

impl Mul for FixedTransform {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        self.mul_transform(rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_transform_preserves_points_and_vectors() {
        let id = FixedTransform::IDENTITY;
        let p = FixedVec3::new(Fixed::ONE, Fixed::from_int(2), Fixed::from_int(3));
        assert_eq!(id.transform_point(p), p);
        assert_eq!(id.transform_vector(p), p);
        assert_eq!(id * id, id);
    }

    #[test]
    fn transform_point_and_vector_semantics() {
        let transform = FixedTransform::new(
            FixedVec3::new(Fixed::from_int(10), Fixed::ZERO, Fixed::ZERO),
            FixedQuat::from_axis_angle(
                FixedVec3::new(Fixed::ZERO, Fixed::ZERO, Fixed::ONE),
                Fixed::FRAC_PI_2,
            ),
            Fixed::from_int(2),
        );

        let p = FixedVec3::new(Fixed::ONE, Fixed::ZERO, Fixed::ZERO);

        // Vector: scale by 2 -> (2, 0, 0), rotate 90 deg around Z -> (0, 2, 0), no translation
        let v_transformed = transform.transform_vector(p);
        assert!(
            v_transformed.x.raw().abs_diff(0) <= 128,
            "vx diff: {}",
            v_transformed.x.raw().abs_diff(0)
        );
        assert!(
            v_transformed.y.raw().abs_diff(Fixed::from_int(2).raw()) <= 128,
            "vy diff: {}",
            v_transformed.y.raw().abs_diff(Fixed::from_int(2).raw())
        );
        assert_eq!(v_transformed.z, Fixed::ZERO);

        // Point: includes translation (10, 0, 0) -> (10, 2, 0)
        let p_transformed = transform.transform_point(p);
        assert!(
            p_transformed.x.raw().abs_diff(Fixed::from_int(10).raw()) <= 128,
            "px diff: {}",
            p_transformed.x.raw().abs_diff(Fixed::from_int(10).raw())
        );
        assert!(
            p_transformed.y.raw().abs_diff(Fixed::from_int(2).raw()) <= 128,
            "py diff: {}",
            p_transformed.y.raw().abs_diff(Fixed::from_int(2).raw())
        );
        assert_eq!(p_transformed.z, Fixed::ZERO);

        // Inverse restores original point
        let p_restored = transform.inverse().transform_point(p_transformed);
        assert!(
            p_restored.x.raw().abs_diff(p.x.raw()) <= 128,
            "p_restored.x diff: {}",
            p_restored.x.raw().abs_diff(p.x.raw())
        );
        assert!(
            p_restored.y.raw().abs_diff(p.y.raw()) <= 128,
            "p_restored.y diff: {}",
            p_restored.y.raw().abs_diff(p.y.raw())
        );
        assert!(
            p_restored.z.raw().abs_diff(p.z.raw()) <= 128,
            "p_restored.z diff: {}",
            p_restored.z.raw().abs_diff(p.z.raw())
        );
    }

    #[test]
    fn transform_composition() {
        let t1 = FixedTransform::from_translation(FixedVec3::new(
            Fixed::from_int(5),
            Fixed::ZERO,
            Fixed::ZERO,
        ));
        let t2 = FixedTransform::from_translation(FixedVec3::new(
            Fixed::ZERO,
            Fixed::from_int(3),
            Fixed::ZERO,
        ));

        let combined = t1 * t2;
        let p = FixedVec3::ZERO;
        assert_eq!(
            combined.transform_point(p),
            FixedVec3::new(Fixed::from_int(5), Fixed::from_int(3), Fixed::ZERO)
        );
    }

    #[test]
    fn zero_scale_inverse_is_safe() {
        let mut t = FixedTransform::from_translation(FixedVec3::new(
            Fixed::from_int(1),
            Fixed::from_int(2),
            Fixed::from_int(3),
        ));
        t.scale = Fixed::ZERO;
        assert_eq!(t.inverse(), FixedTransform::IDENTITY);
    }

    #[test]
    fn transform_associativity() {
        let t1 = FixedTransform::new(
            FixedVec3::new(Fixed::from_int(1), Fixed::ZERO, Fixed::ZERO),
            FixedQuat::from_axis_angle(
                FixedVec3::new(Fixed::ZERO, Fixed::ONE, Fixed::ZERO),
                Fixed::from_ratio(1, 4),
            ),
            Fixed::from_int(2),
        );
        let t2 = FixedTransform::new(
            FixedVec3::new(Fixed::ZERO, Fixed::from_int(2), Fixed::ZERO),
            FixedQuat::from_axis_angle(
                FixedVec3::new(Fixed::ONE, Fixed::ZERO, Fixed::ZERO),
                Fixed::from_ratio(1, 3),
            ),
            Fixed::from_ratio(3, 2),
        );
        let t3 = FixedTransform::new(
            FixedVec3::new(Fixed::ZERO, Fixed::ZERO, Fixed::from_int(3)),
            FixedQuat::from_axis_angle(
                FixedVec3::new(Fixed::ZERO, Fixed::ZERO, Fixed::ONE),
                Fixed::from_ratio(1, 2),
            ),
            Fixed::from_ratio(4, 5),
        );

        let t_a = (t1 * t2) * t3;
        let t_b = t1 * (t2 * t3);

        let p = FixedVec3::new(Fixed::ONE, Fixed::from_int(2), Fixed::from_int(3));
        let p_a = t_a.transform_point(p);
        let p_b = t_b.transform_point(p);

        assert!(p_a.x.raw().abs_diff(p_b.x.raw()) <= 64);
        assert!(p_a.y.raw().abs_diff(p_b.y.raw()) <= 64);
        assert!(p_a.z.raw().abs_diff(p_b.z.raw()) <= 64);
    }
}
