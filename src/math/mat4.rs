//! 4x4 matrix, column-major to match WGSL/GLSL memory layout.
//!
//! `m[col * 4 + row]`, so `to_cols_array()` can go straight into a uniform buffer.

use super::{Vec2, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    pub m: [f32; 16],
}

impl Default for Mat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        m: [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, //
        ],
    };

    pub const fn from_cols_array(m: [f32; 16]) -> Self {
        Self { m }
    }

    pub const fn to_cols_array(self) -> [f32; 16] {
        self.m
    }

    fn index(&self, col: usize, row: usize) -> f32 {
        self.m[col * 4 + row]
    }

    /// `self * rhs`, i.e. `rhs` is applied first.
    #[allow(clippy::should_implement_trait)]
    pub fn mul(self, rhs: Self) -> Self {
        let mut out = [0.0; 16];
        for col in 0..4 {
            for row in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += self.index(k, row) * rhs.index(col, k);
                }
                out[col * 4 + row] = sum;
            }
        }
        Self { m: out }
    }

    pub fn translation(t: Vec3) -> Self {
        let mut m = Self::IDENTITY;
        m.m[12] = t.x;
        m.m[13] = t.y;
        m.m[14] = t.z;
        m
    }

    pub fn scale(s: Vec3) -> Self {
        let mut m = Self::IDENTITY;
        m.m[0] = s.x;
        m.m[5] = s.y;
        m.m[10] = s.z;
        m
    }

    pub fn rotation_z(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Self::from_cols_array([
            c, s, 0.0, 0.0, //
            -s, c, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, //
        ])
    }

    pub fn rotation_x(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Self::from_cols_array([
            1.0, 0.0, 0.0, 0.0, //
            0.0, c, s, 0.0, //
            0.0, -s, c, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    pub fn rotation_y(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Self::from_cols_array([
            c, 0.0, -s, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            s, 0.0, c, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// Orthographic projection for 2D. `size` is the visible world size,
    /// `aspect` the window aspect ratio; the larger dimension is kept fixed.
    pub fn orthographic_2d(size: Vec2, aspect: f32) -> Self {
        let aspect = aspect.abs().max(super::EPSILON);
        let size = Vec2::new(
            size.x.abs().max(super::EPSILON),
            size.y.abs().max(super::EPSILON),
        );

        let (half_w, half_h) = if aspect >= 1.0 {
            (size.x * 0.5, size.x * 0.5 / aspect)
        } else {
            (size.y * 0.5 * aspect, size.y * 0.5)
        };

        let left = -half_w;
        let right = half_w;
        let bottom = -half_h;
        let top = half_h;

        // Maps (left, right) -> (-1, 1) and (bottom, top) -> (-1, 1), so +Y is up.
        Self::from_cols_array([
            2.0 / (right - left),
            0.0,
            0.0,
            0.0,
            0.0,
            2.0 / (top - bottom),
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            -(right + left) / (right - left),
            -(top + bottom) / (top - bottom),
            0.0,
            1.0,
        ])
    }

    /// Right-handed perspective projection, depth in `0..1` (wgpu convention).
    /// Kept for the 3D milestone; the 2D demo does not call it yet.
    #[allow(dead_code)]
    pub fn perspective(fov_y_radians: f32, aspect: f32, near: f32, far: f32) -> Self {
        let f = 1.0 / (fov_y_radians * 0.5).tan();
        let nf = near - far;
        Self::from_cols_array([
            f / aspect,
            0.0,
            0.0,
            0.0,
            0.0,
            f,
            0.0,
            0.0,
            0.0,
            0.0,
            far / nf,
            -1.0,
            0.0,
            0.0,
            (near * far) / nf,
            0.0,
        ])
    }

    #[allow(dead_code)]
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let f = (target - eye).normalize();
        let s = f.cross(up).normalize();
        let u = s.cross(f);

        Self::from_cols_array([
            s.x,
            u.x,
            -f.x,
            0.0, //
            s.y,
            u.y,
            -f.y,
            0.0, //
            s.z,
            u.z,
            -f.z,
            0.0, //
            -s.dot(eye),
            -u.dot(eye),
            f.dot(eye),
            1.0,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_neutral() {
        let t = Mat4::translation(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(Mat4::IDENTITY.mul(t), t);
        assert_eq!(t.mul(Mat4::IDENTITY), t);
    }

    #[test]
    fn rotation_z_90_degrees() {
        let r = Mat4::rotation_z(std::f32::consts::FRAC_PI_2);
        // Column 0 is the transformed X axis: it should become +Y.
        assert!((r.m[0] - 0.0).abs() < 1e-5);
        assert!((r.m[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn rotations_x_and_y_use_right_handed_axes() {
        let rotation_x = Mat4::rotation_x(std::f32::consts::FRAC_PI_2);
        assert!((rotation_x.m[6] - 1.0).abs() < 1e-5);

        let rotation_y = Mat4::rotation_y(std::f32::consts::FRAC_PI_2);
        assert!((rotation_y.m[2] + 1.0).abs() < 1e-5);
    }

    #[test]
    fn ortho_center_maps_to_origin() {
        let p = Mat4::orthographic_2d(Vec2::new(20.0, 20.0), 1.0);
        assert!((p.m[12]).abs() < 1e-5);
        assert!((p.m[13]).abs() < 1e-5);
        // Top of the screen maps to +1 in clip space.
        assert!((p.m[5] - 0.1).abs() < 1e-5);
    }

    #[test]
    fn perspective_projection_is_well_defined() {
        let p = Mat4::perspective(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        let expected_z = 100.0 / (0.1 - 100.0);
        let expected_w = (0.1 * 100.0) / (0.1 - 100.0);
        assert!((p.m[0] - 1.0).abs() < 1e-5);
        assert!((p.m[5] - 1.0).abs() < 1e-5);
        assert!((p.m[10] - expected_z).abs() < 1e-5);
        assert!((p.m[14] - expected_w).abs() < 1e-5);
    }

    #[test]
    fn orthographic_zero_size_and_zero_aspect_stay_finite() {
        let p = Mat4::orthographic_2d(Vec2::ZERO, 0.0);
        assert!(p.m.iter().all(|value| value.is_finite()));
        assert!(p.m[0].is_finite());
    }
}
