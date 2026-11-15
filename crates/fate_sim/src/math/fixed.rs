use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

const FRACTIONAL_BITS: u32 = 32;
const SCALE: i64 = 1_i64 << FRACTIONAL_BITS;

/// Signed Q32.32 value with truncating division and saturating arithmetic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fixed(i64);

impl Fixed {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(SCALE);
    pub const FRAC_PI_4: Self = Self(3_373_259_426);
    pub const FRAC_PI_2: Self = Self(6_746_518_852);
    pub const PI: Self = Self(13_493_037_704);
    pub const TAU: Self = Self(26_986_075_408);

    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> i64 {
        self.0
    }

    pub const fn from_int(value: i64) -> Self {
        Self(value.saturating_mul(SCALE))
    }

    pub const fn from_ratio(numerator: i64, denominator: i64) -> Self {
        assert!(
            denominator != 0,
            "fixed-point ratio denominator must not be zero"
        );
        let value = numerator as i128 * SCALE as i128 / denominator as i128;
        if value > i64::MAX as i128 {
            Self(i64::MAX)
        } else if value < i64::MIN as i128 {
            Self(i64::MIN)
        } else {
            Self(value as i64)
        }
    }

    /// Convert to a renderer-facing float. Do not feed this value back into simulation.
    pub fn to_f32(self) -> f32 {
        self.raw() as f32 / SCALE as f32
    }

    pub fn sqrt(self) -> Self {
        if self.0 <= 0 {
            return Self::ZERO;
        }
        let scaled = (self.0 as u128) << FRACTIONAL_BITS;
        Self::from_raw(integer_sqrt(scaled) as i64)
    }

    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self(self.0.clamp(min.0, max.0))
    }

    pub fn rem_euclid(self, rhs: Self) -> Self {
        assert!(rhs.0 > 0, "fixed-point modulus must be positive");
        Self(self.0.rem_euclid(rhs.0))
    }

    /// Compute deterministic sine and cosine using fixed-point polynomial approximation.
    pub fn sin_cos(self) -> (Self, Self) {
        let raw = self.rem_euclid(Self::TAU).0;
        let oct = (raw / Self::FRAC_PI_4.0).min(7) as usize;
        let rem = raw - oct as i64 * Self::FRAC_PI_4.0;
        let t = if oct % 2 == 1 {
            Self::from_raw(Self::FRAC_PI_4.0 - rem)
        } else {
            Self::from_raw(rem)
        };

        let s = sin_octant(t);
        let c = cos_octant(t);

        match oct {
            0 => (s, c),
            1 => (c, s),
            2 => (c, -s),
            3 => (s, -c),
            4 => (-s, -c),
            5 => (-c, -s),
            6 => (-c, s),
            7 => (-s, c),
            _ => unreachable!(),
        }
    }

    /// Compute deterministic sine in fixed-point.
    pub fn sin(self) -> Self {
        self.sin_cos().0
    }

    /// Compute deterministic cosine in fixed-point.
    pub fn cos(self) -> Self {
        self.sin_cos().1
    }
}

impl Add for Fixed {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }
}

impl AddAssign for Fixed {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for Fixed {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl SubAssign for Fixed {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl Mul for Fixed {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        let product = self.0 as i128 * rhs.0 as i128;
        Self((product / SCALE as i128).clamp(i64::MIN as i128, i64::MAX as i128) as i64)
    }
}

impl Div for Fixed {
    type Output = Self;

    fn div(self, rhs: Self) -> Self {
        assert_ne!(rhs.0, 0, "fixed-point division by zero");
        let quotient = self.0 as i128 * SCALE as i128 / rhs.0 as i128;
        Self(quotient.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
    }
}

impl Neg for Fixed {
    type Output = Self;

    fn neg(self) -> Self {
        Self(self.0.saturating_neg())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FixedVec2 {
    pub x: Fixed,
    pub y: Fixed,
}

impl FixedVec2 {
    pub const ZERO: Self = Self {
        x: Fixed::ZERO,
        y: Fixed::ZERO,
    };

    pub const fn new(x: Fixed, y: Fixed) -> Self {
        Self { x, y }
    }

    pub fn length_squared(self) -> Fixed {
        self.x * self.x + self.y * self.y
    }

    pub fn length(self) -> Fixed {
        self.length_squared().sqrt()
    }

    pub fn normalize(self) -> Self {
        let length = self.length();
        if length == Fixed::ZERO {
            Self::ZERO
        } else {
            self / length
        }
    }
}

impl Add for FixedVec2 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl AddAssign for FixedVec2 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for FixedVec2 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<Fixed> for FixedVec2 {
    type Output = Self;

    fn mul(self, rhs: Fixed) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

impl Div<Fixed> for FixedVec2 {
    type Output = Self;

    fn div(self, rhs: Fixed) -> Self {
        Self::new(self.x / rhs, self.y / rhs)
    }
}

impl Neg for FixedVec2 {
    type Output = Self;

    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FixedVec3 {
    pub x: Fixed,
    pub y: Fixed,
    pub z: Fixed,
}

impl FixedVec3 {
    pub const ZERO: Self = Self {
        x: Fixed::ZERO,
        y: Fixed::ZERO,
        z: Fixed::ZERO,
    };

    pub const fn new(x: Fixed, y: Fixed, z: Fixed) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, rhs: Self) -> Fixed {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    pub fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }

    pub fn length_squared(self) -> Fixed {
        self.dot(self)
    }

    pub fn length(self) -> Fixed {
        self.length_squared().sqrt()
    }

    pub fn normalize(self) -> Self {
        let length = self.length();
        if length == Fixed::ZERO {
            Self::ZERO
        } else {
            self / length
        }
    }

    /// Convert to a renderer-facing float triple. Do not feed this back into simulation.
    pub fn to_f32x3(self) -> [f32; 3] {
        [self.x.to_f32(), self.y.to_f32(), self.z.to_f32()]
    }
}

impl Add for FixedVec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl AddAssign for FixedVec3 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for FixedVec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<Fixed> for FixedVec3 {
    type Output = Self;

    fn mul(self, rhs: Fixed) -> Self {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Div<Fixed> for FixedVec3 {
    type Output = Self;

    fn div(self, rhs: Fixed) -> Self {
        Self::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

impl Neg for FixedVec3 {
    type Output = Self;

    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

const SIN_C1: Fixed = Fixed::from_ratio(1, 6);
const SIN_C2: Fixed = Fixed::from_ratio(1, 120);
const SIN_C3: Fixed = Fixed::from_ratio(1, 5_040);
const SIN_C4: Fixed = Fixed::from_ratio(1, 362_880);

const COS_C1: Fixed = Fixed::from_ratio(1, 2);
const COS_C2: Fixed = Fixed::from_ratio(1, 24);
const COS_C3: Fixed = Fixed::from_ratio(1, 720);
const COS_C4: Fixed = Fixed::from_ratio(1, 40_320);
const COS_C5: Fixed = Fixed::from_ratio(1, 3_628_800);

fn sin_octant(x: Fixed) -> Fixed {
    let x2 = x * x;
    let term = SIN_C1 - x2 * (SIN_C2 - x2 * (SIN_C3 - x2 * SIN_C4));
    x * (Fixed::ONE - x2 * term)
}

fn cos_octant(x: Fixed) -> Fixed {
    let x2 = x * x;
    let term = COS_C1 - x2 * (COS_C2 - x2 * (COS_C3 - x2 * (COS_C4 - x2 * COS_C5)));
    Fixed::ONE - x2 * term
}

fn integer_sqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }

    let mut remainder = value;
    let mut result = 0_u128;
    let mut bit = 1_u128 << 126;
    while bit > remainder {
        bit >>= 2;
    }

    while bit != 0 {
        if remainder >= result + bit {
            remainder -= result + bit;
            result = (result >> 1) + bit;
        } else {
            result >>= 1;
        }
        bit >>= 2;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_and_arithmetic_are_deterministic() {
        let third = Fixed::from_ratio(1, 3);
        assert_eq!((third * Fixed::from_int(3)).raw(), SCALE - 1);
        assert_eq!((Fixed::from_int(2) / Fixed::from_int(4)).raw(), SCALE / 2);
    }

    #[test]
    fn square_root_and_vector_normalization_are_fixed_point() {
        assert_eq!(Fixed::from_int(25).sqrt(), Fixed::from_int(5));

        let diagonal = FixedVec2::new(Fixed::ONE, Fixed::ONE).normalize();
        assert_eq!(diagonal.length(), Fixed::ONE);
        assert!(diagonal.x.raw().abs_diff(diagonal.y.raw()) <= 1);
    }

    #[test]
    fn vec3_dot_cross_and_normalize_are_fixed_point() {
        let x = FixedVec3::new(Fixed::ONE, Fixed::ZERO, Fixed::ZERO);
        let y = FixedVec3::new(Fixed::ZERO, Fixed::ONE, Fixed::ZERO);

        assert_eq!(x.dot(y), Fixed::ZERO);
        assert_eq!(x.dot(x), Fixed::ONE);
        assert_eq!(
            x.cross(y),
            FixedVec3::new(Fixed::ZERO, Fixed::ZERO, Fixed::ONE)
        );
        assert_eq!(x.cross(y).dot(x), Fixed::ZERO);
        assert_eq!(x.cross(y).dot(y), Fixed::ZERO);

        let diagonal = FixedVec3::new(Fixed::ONE, Fixed::ONE, Fixed::ONE).normalize();
        // Truncating multiply and sqrt can lose a few raw units on a unit vector.
        assert!(diagonal.length().raw().abs_diff(Fixed::ONE.raw()) <= 4);
        assert_eq!(diagonal.x, diagonal.y);
        assert_eq!(diagonal.y, diagonal.z);
        assert_eq!(FixedVec3::ZERO.normalize(), FixedVec3::ZERO);

        assert_eq!(
            FixedVec3::new(Fixed::ONE, Fixed::from_int(2), Fixed::from_int(3)).to_f32x3(),
            [1.0, 2.0, 3.0]
        );
    }

    #[test]
    fn arithmetic_saturates_instead_of_wrapping() {
        assert_eq!((Fixed::from_raw(i64::MAX) + Fixed::ONE).raw(), i64::MAX);
        assert_eq!(
            (Fixed::from_raw(i64::MAX) * Fixed::from_int(2)).raw(),
            i64::MAX
        );
        assert_eq!(Fixed::from_ratio(i64::MAX, 1).raw(), i64::MAX);
    }

    #[test]
    fn trig_cardinal_angles_and_periodicity() {
        // Zero
        let (s0, c0) = Fixed::ZERO.sin_cos();
        assert_eq!(s0, Fixed::ZERO);
        assert_eq!(c0, Fixed::ONE);

        // pi / 2
        let (s_pi2, c_pi2) = Fixed::FRAC_PI_2.sin_cos();
        assert_eq!(s_pi2, Fixed::ONE);
        assert_eq!(c_pi2, Fixed::ZERO);

        // pi
        let (s_pi, c_pi) = Fixed::PI.sin_cos();
        assert_eq!(s_pi, Fixed::ZERO);
        assert_eq!(c_pi, -Fixed::ONE);

        // 3pi / 2
        let (s_3pi2, c_3pi2) = (Fixed::PI + Fixed::FRAC_PI_2).sin_cos();
        assert_eq!(s_3pi2, -Fixed::ONE);
        assert_eq!(c_3pi2, Fixed::ZERO);

        // 2pi (tau)
        let (s_tau, c_tau) = Fixed::TAU.sin_cos();
        assert_eq!(s_tau, Fixed::ZERO);
        assert_eq!(c_tau, Fixed::ONE);

        // Negative angle (-pi/2)
        let (s_neg_pi2, c_neg_pi2) = (-Fixed::FRAC_PI_2).sin_cos();
        assert_eq!(s_neg_pi2, -Fixed::ONE);
        assert_eq!(c_neg_pi2, Fixed::ZERO);

        // Periodic wrap (tau + pi/2)
        let (s_wrap, c_wrap) = (Fixed::TAU + Fixed::FRAC_PI_2).sin_cos();
        assert_eq!(s_wrap, s_pi2);
        assert_eq!(c_wrap, c_pi2);

        // sin(pi/4) == cos(pi/4) approx sqrt(2)/2 = 0.70710678
        let (s_pi4, c_pi4) = Fixed::FRAC_PI_4.sin_cos();
        assert!(
            s_pi4.raw().abs_diff(c_pi4.raw()) <= 64,
            "s_pi4: {}, c_pi4: {}, diff: {}",
            s_pi4.raw(),
            c_pi4.raw(),
            s_pi4.raw().abs_diff(c_pi4.raw())
        );
        let expected_sqrt2_2 = (Fixed::ONE * Fixed::ONE / Fixed::from_int(2)).sqrt();
        assert!(s_pi4.raw().abs_diff(expected_sqrt2_2.raw()) < 100);

        // Pythagorean identity sin^2 + cos^2 approx 1 across various angles
        let angles = [
            Fixed::from_ratio(1, 10),
            Fixed::from_ratio(1, 3),
            Fixed::from_ratio(7, 5),
            Fixed::from_ratio(22, 7),
            Fixed::from_ratio(-5, 4),
        ];
        for angle in angles {
            let (s, c) = angle.sin_cos();
            let sum_sq = s * s + c * c;
            assert!(
                sum_sq.raw().abs_diff(Fixed::ONE.raw()) <= 16,
                "sin^2 + cos^2 diverged for angle {:?}",
                angle
            );
        }

        // Test standalone sin() and cos() helpers against sin_cos()
        for angle in angles {
            assert_eq!(angle.sin(), angle.sin_cos().0);
            assert_eq!(angle.cos(), angle.sin_cos().1);
        }
    }

    #[test]
    fn trig_odd_even_symmetry() {
        // sin(-x) == -sin(x), cos(-x) == cos(x)
        let sample_steps = 32;
        let step = Fixed::TAU / Fixed::from_int(sample_steps);
        for i in 0..sample_steps {
            let angle = step * Fixed::from_int(i);
            let (s_pos, c_pos) = angle.sin_cos();
            let (s_neg, c_neg) = (-angle).sin_cos();

            assert!(
                s_neg.raw().abs_diff((-s_pos).raw()) <= 1,
                "sin(-x) symmetry failed for angle {:?}",
                angle
            );
            assert!(
                c_neg.raw().abs_diff(c_pos.raw()) <= 1,
                "cos(-x) symmetry failed for angle {:?}",
                angle
            );
        }
    }

    #[test]
    fn trig_continuous_across_octant_boundaries() {
        // Step tightly around boundaries: k * PI / 4
        let eps = Fixed::from_raw(1);
        for k in 0..8 {
            let boundary = Fixed::FRAC_PI_4 * Fixed::from_int(k);
            let (s_before, c_before) = (boundary - eps).sin_cos();
            let (s_at, c_at) = boundary.sin_cos();
            let (s_after, c_after) = (boundary + eps).sin_cos();

            assert!(s_before.raw().abs_diff(s_at.raw()) <= 64);
            assert!(s_after.raw().abs_diff(s_at.raw()) <= 64);
            assert!(c_before.raw().abs_diff(c_at.raw()) <= 64);
            assert!(c_after.raw().abs_diff(c_at.raw()) <= 64);
        }
    }
}
