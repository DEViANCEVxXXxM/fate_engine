mod app;
mod demo_input;
pub mod demo_replay;
mod diagnostics_panel;
mod prototype_simulation;
mod scene;
mod showcase;

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("fate.showcase=info,warn"),
    )
    .init();
    pollster::block_on(app::run());
}
