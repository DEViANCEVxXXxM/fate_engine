mod app;
mod demo_input;
mod demo_simulation;
mod performance;

fn main() {
    env_logger::init();
    pollster::block_on(app::run());
}
