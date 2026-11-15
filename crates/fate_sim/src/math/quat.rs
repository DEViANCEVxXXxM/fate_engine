use std::ops::{Mul, Neg};

use crate::math::{Fixed, FixedVec3};

/// A unit quaternion representing a 3D rotation in fixed-point (Q32.32).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FixedQuat {
    pub x: Fixed,
    pub y: Fixed,
    pub z: Fixed,
    pub w: Fixed,
}

impl Default for FixedQuat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl FixedQuat {
    pub const IDENTITY: Self = Self {
        x: Fixed::ZERO,
        y: Fixed::ZERO,
        z: Fixed::ZERO,
        w: Fixed::ONE,
    };

    pub const fn from_xyzw(x: Fixed, y: Fixed, z: Fixed, w: Fixed) -> Self {
        Self { x, y, z, w }
    }

    /// Construct a rotation quaternion from an axis and an angle in radians.
    pub fn from_axis_angle(axis: FixedVec3, angle: Fixed) -> Self {
        let (sin_half, cos_half) = (angle * Fixed::from_ratio(1, 2)).sin_cos();
        let axis = axis.normalize();
        Self {
            x: axis.x * sin_half,
            y: axis.y * sin_half,
            z: axis.z * sin_half,
            w: cos_half,
        }
    }

    /// Construct a rotation from Euler angles (yaw around Y, pitch around X, roll around Z).
    pub fn from_yaw_pitch_roll(yaw: Fixed, pitch: Fixed, roll: Fixed) -> Self {
        let half = Fixed::from_ratio(1, 2);
        let (sy, cy) = (yaw * half).sin_cos();
        let (sp, cp) = (pitch * half).sin_cos();
        let (sr, cr) = (roll * half).sin_cos();

        Self {
            x: cy * sp * cr + sy * cp * sr,
            y: sy * cp * cr - cy * sp * sr,
            z: cy * cp * sr - sy * sp * cr,
            w: cy * cp * cr + sy * sp * sr,
        }
    }

    /// Norm squared of the quaternion: x^2 + y^2 + z^2 + w^2.
    pub fn length_squared(self) -> Fixed {
        self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w
    }

    /// Norm of the quaternion.
    pub fn length(self) -> Fixed {
        self.length_squared().sqrt()
    }

    /// Normalize into a unit quaternion. Returns IDENTITY if length is zero.
    pub fn normalize(self) -> Self {
        let len = self.length();
        if len == Fixed::ZERO {
            Self::IDENTITY
        } else {
            Self {
                x: self.x / len,
                y: self.y / len,
                z: self.z / len,
                w: self.w / len,
            }
        }
    }

    /// Quaternion conjugate (reverses rotation direction if normalized).
    pub fn conjugate(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: self.w,
        }
    }

    /// Quaternion inverse. For unit quaternions, this is equivalent to `conjugate`.
    pub fn inverse(self) -> Self {
        let len_sq = self.length_squared();
        if len_sq == Fixed::ZERO {
            Self::IDENTITY
        } else {
            let conj = self.conjugate();
            Self {
                x: conj.x / len_sq,
                y: conj.y / len_sq,
                z: conj.z / len_sq,
                w: conj.w / len_sq,
            }
        }
    }

    /// Rotate a 3D vector by this quaternion: q * v * q^-1.
    pub fn rotate_vec3(self, v: FixedVec3) -> FixedVec3 {
        let q_vec = FixedVec3::new(self.x, self.y, self.z);
        let two = Fixed::from_int(2);
        let t = q_vec.cross(v) * two;
        v + t * self.w + q_vec.cross(t)
    }

    /// Convert to a renderer-facing float 4-tuple [x, y, z, w].
    pub fn to_f32x4(self) -> [f32; 4] {
        [
            self.x.to_f32(),
            self.y.to_f32(),
            self.z.to_f32(),
            self.w.to_f32(),
        ]
    }
}

impl Mul for FixedQuat {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        Self {
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
        }
    }
}

impl Neg for FixedQuat {
    type Output = Self;

    fn neg(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: -self.w,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_quaternion_does_not_alter_vectors() {
        let id = FixedQuat::IDENTITY;
        let v = FixedVec3::new(Fixed::ONE, Fixed::from_int(2), Fixed::from_int(3));
        assert_eq!(id.rotate_vec3(v), v);
        assert_eq!(id * id, id);
        assert_eq!(id.length(), Fixed::ONE);
    }

    #[test]
    fn axis_angle_rotations_match_cardinal_axes() {
        // 90 degrees around Z rotates (1, 0, 0) to (0, 1, 0)
        let q_z90 = FixedQuat::from_axis_angle(
            FixedVec3::new(Fixed::ZERO, Fixed::ZERO, Fixed::ONE),
            Fixed::FRAC_PI_2,
        );
        let x_axis = FixedVec3::new(Fixed::ONE, Fixed::ZERO, Fixed::ZERO);
        let rotated = q_z90.rotate_vec3(x_axis);

        assert!(
            rotated.x.raw().abs_diff(0) <= 64,
            "x diff: {}",
            rotated.x.raw().abs_diff(0)
        );
        assert!(
            rotated.y.raw().abs_diff(Fixed::ONE.raw()) <= 64,
            "y diff: {}",
            rotated.y.raw().abs_diff(Fixed::ONE.raw())
        );
        assert!(
            rotated.z.raw().abs_diff(0) <= 64,
            "z diff: {}",
            rotated.z.raw().abs_diff(0)
        );

        // 90 degrees around Y rotates (0, 0, 1) to (1, 0, 0)
        let q_y90 = FixedQuat::from_axis_angle(
            FixedVec3::new(Fixed::ZERO, Fixed::ONE, Fixed::ZERO),
            Fixed::FRAC_PI_2,
        );
        let z_axis = FixedVec3::new(Fixed::ZERO, Fixed::ZERO, Fixed::ONE);
        let rotated_z = q_y90.rotate_vec3(z_axis);

        assert!(
            rotated_z.x.raw().abs_diff(Fixed::ONE.raw()) <= 64,
            "x diff: {}",
            rotated_z.x.raw().abs_diff(Fixed::ONE.raw())
        );
        assert!(
            rotated_z.y.raw().abs_diff(0) <= 64,
            "y diff: {}",
            rotated_z.y.raw().abs_diff(0)
        );
        assert!(
            rotated_z.z.raw().abs_diff(0) <= 64,
            "z diff: {}",
            rotated_z.z.raw().abs_diff(0)
        );
    }

    #[test]
    fn quaternion_composition_and_inverse() {
        let q1 = FixedQuat::from_axis_angle(
            FixedVec3::new(Fixed::ONE, Fixed::ZERO, Fixed::ZERO),
            Fixed::from_ratio(1, 3),
        );
        let q2 = FixedQuat::from_axis_angle(
            FixedVec3::new(Fixed::ZERO, Fixed::ONE, Fixed::ZERO),
            Fixed::from_ratio(1, 2),
        );

        let q_combined = q2 * q1;
        let v = FixedVec3::new(Fixed::ONE, Fixed::from_int(3), -Fixed::from_int(2));

        let rotated_step = q2.rotate_vec3(q1.rotate_vec3(v));
        let rotated_comb = q_combined.rotate_vec3(v);

        assert!(rotated_step.x.raw().abs_diff(rotated_comb.x.raw()) <= 16);
        assert!(rotated_step.y.raw().abs_diff(rotated_comb.y.raw()) <= 16);
        assert!(rotated_step.z.raw().abs_diff(rotated_comb.z.raw()) <= 16);

        // Inverse restores original vector
        let restored = q_combined.inverse().rotate_vec3(rotated_comb);
        assert!(restored.x.raw().abs_diff(v.x.raw()) <= 32);
        assert!(restored.y.raw().abs_diff(v.y.raw()) <= 32);
        assert!(restored.z.raw().abs_diff(v.z.raw()) <= 32);
    }

    #[test]
    fn from_yaw_pitch_roll_matches_axis_angles() {
        let yaw = Fixed::from_ratio(1, 4);
        let pitch = Fixed::from_ratio(1, 5);
        let roll = Fixed::from_ratio(1, 6);

        let q_euler = FixedQuat::from_yaw_pitch_roll(yaw, pitch, roll);

        let q_yaw =
            FixedQuat::from_axis_angle(FixedVec3::new(Fixed::ZERO, Fixed::ONE, Fixed::ZERO), yaw);
        let q_pitch =
            FixedQuat::from_axis_angle(FixedVec3::new(Fixed::ONE, Fixed::ZERO, Fixed::ZERO), pitch);
        let q_roll =
            FixedQuat::from_axis_angle(FixedVec3::new(Fixed::ZERO, Fixed::ZERO, Fixed::ONE), roll);
        // Standard extrinsic Y-X-Z order or intrinsic roll then pitch then yaw (q_yaw * q_pitch * q_roll)
        let q_combined = q_yaw * q_pitch * q_roll;

        assert!(q_euler.x.raw().abs_diff(q_combined.x.raw()) <= 16);
        assert!(q_euler.y.raw().abs_diff(q_combined.y.raw()) <= 16);
        assert!(q_euler.z.raw().abs_diff(q_combined.z.raw()) <= 16);
        assert!(q_euler.w.raw().abs_diff(q_combined.w.raw()) <= 16);
    }

    #[test]
    fn to_f32x4_matches_fixed_representation() {
        let q = FixedQuat::from_axis_angle(
            FixedVec3::new(Fixed::ZERO, Fixed::ONE, Fixed::ZERO),
            Fixed::FRAC_PI_2,
        );
        let f = q.to_f32x4();
        assert!((f[0] - 0.0).abs() < 1e-4);
        assert!((f[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        assert!((f[2] - 0.0).abs() < 1e-4);
        assert!((f[3] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    }

    #[test]
    fn zero_norm_fallbacks_are_safe() {
        let q_zero = FixedQuat::from_xyzw(Fixed::ZERO, Fixed::ZERO, Fixed::ZERO, Fixed::ZERO);
        assert_eq!(q_zero.normalize(), FixedQuat::IDENTITY);
        assert_eq!(q_zero.inverse(), FixedQuat::IDENTITY);
    }

    #[test]
    fn quaternion_rotation_preserves_vector_length() {
        let q = FixedQuat::from_yaw_pitch_roll(
            Fixed::from_ratio(1, 3),
            Fixed::from_ratio(1, 4),
            Fixed::from_ratio(1, 5),
        );
        let v = FixedVec3::new(Fixed::from_int(3), Fixed::from_int(-4), Fixed::from_int(12)); // Length = 13
        assert_eq!(v.length(), Fixed::from_int(13));

        let rotated = q.rotate_vec3(v);
        assert!(rotated.length().raw().abs_diff(Fixed::from_int(13).raw()) <= 16);
    }

    #[test]
    fn quaternion_multiplication_is_associative() {
        let q1 = FixedQuat::from_axis_angle(
            FixedVec3::new(Fixed::ONE, Fixed::ZERO, Fixed::ZERO),
            Fixed::from_ratio(1, 4),
        );
        let q2 = FixedQuat::from_axis_angle(
            FixedVec3::new(Fixed::ZERO, Fixed::ONE, Fixed::ZERO),
            Fixed::from_ratio(1, 3),
        );
        let q3 = FixedQuat::from_axis_angle(
            FixedVec3::new(Fixed::ZERO, Fixed::ZERO, Fixed::ONE),
            Fixed::from_ratio(1, 2),
        );

        let q_a = (q1 * q2) * q3;
        let q_b = q1 * (q2 * q3);

        assert!(q_a.x.raw().abs_diff(q_b.x.raw()) <= 16);
        assert!(q_a.y.raw().abs_diff(q_b.y.raw()) <= 16);
        assert!(q_a.z.raw().abs_diff(q_b.z.raw()) <= 16);
        assert!(q_a.w.raw().abs_diff(q_b.w.raw()) <= 16);
    }
}
