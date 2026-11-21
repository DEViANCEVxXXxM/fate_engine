mod app;
mod demo_input;
pub mod demo_replay;
mod diagnostics_panel;
mod prototype_simulation;
mod replay_cli;
mod scene;
mod showcase;

fn main() {
    let mut args = std::env::args_os().skip(1).peekable();
    if let Some("--record-replay" | "--inspect-replay") = args.peek().and_then(|arg| arg.to_str()) {
        let record = args.next().is_some_and(|arg| arg == "--record-replay");
        let Some(path) = args
            .next()
            .filter(|arg| !arg.is_empty() && args.next().is_none())
        else {
            eprintln!("Replay commands take exactly one file path. Use --help.");
            std::process::exit(2);
        };
        std::process::exit(replay_cli::run(record, std::path::Path::new(&path)));
    }
    let mut debug = false;
    let mut smoke = false;
    for arg in args {
        match arg.to_str() {
            Some("--dbg") => debug = true,
            Some("--dbg-smoke") => {
                debug = true;
                smoke = true;
            }
            Some("--help" | "-h") => {
                println!(
                    "fate_demo [--dbg] [--dbg-smoke]\n  --dbg                  Open the native diagnostics companion (F3 reopens it)\n  --dbg-smoke            Exercise diagnostics and exit automatically\n  --record-replay PATH   Record a scripted showcase replay without opening a window\n  --inspect-replay PATH  Check a replay without opening a window"
                );
                return;
            }
            _ => {
                eprintln!("Unknown argument: {}. Use --help.", arg.to_string_lossy());
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
