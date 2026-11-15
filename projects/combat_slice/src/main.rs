#![forbid(unsafe_code)]

mod app;
mod combat_input;
mod gamepad;
mod interpolation;
mod replay;
mod scene;
mod sim;
#[cfg(target_os = "windows")]
mod sony_hid;
mod verification;

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("fate.combat=info,warn"),
    )
    .init();
    pollster::block_on(app::run());
}
