//! A small sparse-set-ish ECS in the headless simulation crate.
//!
//! Every entity index is valid in every component column, which keeps lookup
//! O(1) and the code trivial to debug. This is deliberately not archetype
//! based; swap it out later when the game actually needs query caching.

mod system;
mod world;

pub use system::{bounds_system, movement_system, spin_system};
pub use world::{EcsError, Entity, Position, Rotation, Spin, Velocity, World};
