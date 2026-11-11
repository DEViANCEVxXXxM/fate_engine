#![forbid(unsafe_code)]

pub mod ecs;
pub mod math;
pub mod rng;
pub mod simulation;

pub use simulation::{SimulationState, SimulationStateError, StateHasher, StateWriter};
