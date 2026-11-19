mod app;
mod demo_input;
pub mod demo_replay;
mod diagnostics_panel;
mod prototype_simulation;
mod scene;
mod showcase;

fn main() {
    let mut debug = false;
    let mut smoke = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--dbg" => debug = true,
            "--dbg-smoke" => {
                debug = true;
                smoke = true;
            }
            "--help" | "-h" => {
                println!(
                    "fate_demo [--dbg] [--dbg-smoke]\n  --dbg        Open the native diagnostics companion (F3 reopens it)\n  --dbg-smoke  Exercise diagnostics and exit automatically"
                );
                return;
            }
            _ => {
                eprintln!("Unknown argument: {arg}. Use --help.");
                std::process::exit(2);
            }
        }
    }
    let logger = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("fate.showcase=info,warn"),
    )
    .build();
    let filter = logger.filter();
    let inbox = if debug {
        let (logger, inbox) = fate_debug::capture_logs(Box::new(logger));
        log::set_boxed_logger(Box::new(logger)).expect("logger already initialized");
        Some(inbox)
    } else {
        log::set_boxed_logger(Box::new(logger)).expect("logger already initialized");
        None
    };
    log::set_max_level(filter);
    pollster::block_on(app::run(debug, smoke, inbox));
}
