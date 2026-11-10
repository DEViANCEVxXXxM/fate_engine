use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use super::Vec2;

const FRACTIONAL_BITS: u32 = 32;
const SCALE: i64 = 1_i64 << FRACTIONAL_BITS;

/// Signed Q32.32 value with truncating division and saturating arithmetic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fixed(i64);

impl Fixed {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(SCALE);

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

    pub fn to_vec2(self) -> Vec2 {
        Vec2::new(self.x.to_f32(), self.y.to_f32())
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
    fn arithmetic_saturates_instead_of_wrapping() {
        assert_eq!((Fixed::from_raw(i64::MAX) + Fixed::ONE).raw(), i64::MAX);
        assert_eq!(
            (Fixed::from_raw(i64::MAX) * Fixed::from_int(2)).raw(),
            i64::MAX
        );
        assert_eq!(Fixed::from_ratio(i64::MAX, 1).raw(), i64::MAX);
    }
}
