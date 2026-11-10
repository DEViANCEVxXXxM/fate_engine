//! Minimal 2D engine skeleton: winit for the window, wgpu for the GPU,
//! everything else (loop, ECS, math, renderer) written here.

mod app;
mod ecs;
mod input;
mod math;
mod performance;
mod renderer;
mod simulation;
#[allow(dead_code)]
mod simulation_bench;
mod time;

fn main() {
    env_logger::init();
    pollster::block_on(app::run());
}
