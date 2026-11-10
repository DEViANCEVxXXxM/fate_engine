mod app;
mod demo_input;
pub mod demo_replay;
mod demo_simulation;
mod performance;
mod prototype_simulation;

fn main() {
    env_logger::init();
    pollster::block_on(app::run());
}
