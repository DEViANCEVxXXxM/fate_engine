//! Hand-rolled math. Fixed-point types are for simulation; `Vec2` and matrices
//! use `f32` at the renderer boundary.

mod fixed;
mod mat4;
mod vec2;
mod vec3;

pub use fixed::{Fixed, FixedVec2};
pub use mat4::Mat4;
pub use vec2::Vec2;
pub use vec3::Vec3;

pub const EPSILON: f32 = 1.0e-6;
