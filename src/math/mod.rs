//! Hand-rolled math. `f32` everywhere: it matches the GPU and is plenty for
//! game transforms.

mod mat4;
mod vec2;
mod vec3;

pub use mat4::Mat4;
pub use vec2::Vec2;
pub use vec3::Vec3;

pub const EPSILON: f32 = 1.0e-6;
