#[cfg(feature = "windowed")]
mod gamepad;
#[cfg(feature = "windowed")]
mod models;
mod replay;
mod sim;
mod storage;
#[cfg(feature = "windowed")]
mod window;

use std::path::Path;

use replay::Replay;
use sim::{Command, Duel, FULL_AXIS, Outcome, Phase};

const CHECKPOINT: u32 = 169;
const LUNGE_CHECKPOINT: u32 = 239;
const SCRIPT_END: u32 = 315;

fn command_at(tick: u32) -> Command {
    Command {
        x: (i8::from(matches!(tick, 45..=50 | 225..=236)) - i8::from((56..=65).contains(&tick)))
            * FULL_AXIS,
        z: i8::from((207..=214).contains(&tick)) * FULL_AXIS,
        guard: matches!(
            tick,
            294..=297 | 303..=306 | 312..=315
        ),
        attack: matches!(tick, 40 | 104 | 168 | 244 | 315),
        aim_x: if tick <= 40 { 0 } else { -103 },
        aim_z: -FULL_AXIS,
        boost: tick == 109,
        dash: (50..=56).contains(&tick),
    }
}

fn checked_script() -> (Replay, u64) {
    let mut reference = Duel::new();
    let mut tape = Replay::new(&reference);
    let mut branches: [Option<Replay>; 2] = [None, None];

    for tick in 1..=SCRIPT_END {
        let command = command_at(tick);
        reference.step(command);
        tape.record(command, &reference);
        for branch in branches.iter_mut().flatten() {
            branch.record(command, &reference);
        }
        if tick == CHECKPOINT {
            assert_eq!(reference.outcome, Outcome::Active);
            assert_eq!(reference.phase, Phase::Waves);
            assert_eq!(reference.pressure, 1);
            assert!(!reference.bolts.is_empty());
            assert!(!reference.shots.is_empty());
            branches[0] = Some(Replay::new(&reference));
        }
        if tick == LUNGE_CHECKPOINT {
            assert_eq!(reference.phase, Phase::Close);
            assert_ne!(reference.opponent_position, fate_sim::math::FixedVec2::ZERO);
            branches[1] = Some(Replay::new(&reference));
        }
    }
    assert_eq!(reference.outcome, Outcome::Won);

    let expected = reference.snapshot();
    let checkpoints = branches
        .iter()
        .map(|branch| branch.as_ref().expect("recorded replay checkpoint"));
    for replay in std::iter::once(&tape).chain(checkpoints) {
        let decoded = Replay::decode(&replay.encode()).expect("encoded replay is valid");
        assert_eq!(
            decoded.verify().expect("script replay matches").snapshot(),
            expected
        );
    }
    (tape, reference.hash())
}

fn verify_script() -> u64 {
    checked_script().1
}

fn verify_file(path: &Path) -> Result<Duel, String> {
    let bytes =
        storage::read_bounded(path, replay::MAX_BYTES).map_err(|error| error.to_string())?;
    Replay::decode(&bytes)?.verify()
}

fn fail_file(path: &Path, error: impl std::fmt::Display) -> ! {
    eprintln!("{}: {error}", path.display());
    std::process::exit(1);
}

fn report_script() {
    let hash = verify_script();
    println!("duel lab: replay and mid-round restore matched; final hash {hash:016x}");
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next();
    match (
        mode.as_deref().and_then(std::ffi::OsStr::to_str),
        args.next(),
        args.next(),
    ) {
        (Some("--verify"), None, None) => report_script(),
        (Some("--verify-replay"), Some(path), None) => {
            let path = Path::new(&path);
            let duel = verify_file(path).unwrap_or_else(|error| fail_file(path, error));
            println!(
                "replay verified: tick {}, {:?}, hash {:016x}",
                duel.tick,
                duel.outcome,
                duel.hash()
            );
        }
        (Some("--export-script"), Some(path), None) => {
            let path = Path::new(&path);
            let (tape, _) = checked_script();
            storage::write_atomic(path, &tape.encode())
                .unwrap_or_else(|error| fail_file(path, error));
            println!("script replay exported: {}", path.display());
        }
        #[cfg(feature = "windowed")]
        (None, None, None) if mode.is_none() => window::run(false),
        #[cfg(feature = "windowed")]
        (Some("--play"), None, None) => window::run(false),
        #[cfg(feature = "windowed")]
        (Some("--window-smoke"), None, None) => window::run(true),
        #[cfg(not(feature = "windowed"))]
        (None, None, None) if mode.is_none() => {
            report_script();
            println!("to play: cargo run -p fate_duel_lab --features windowed");
        }
        _ => {
            eprintln!(
                "usage: fate_duel_lab [--verify | --verify-replay PATH | --export-script PATH | --play | --window-smoke] (window modes require --features windowed)"
            );
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scripted_round_wins_and_replays_with_mid_round_restore() {
        super::verify_script();
    }

    #[test]
    fn the_same_route_wins_without_spending_charge() {
        let mut duel = super::Duel::new();
        let mut tape = super::Replay::new(&duel);
        for tick in 1..=306 {
            let command = super::Command {
                boost: false,
                attack: matches!(tick, 40 | 104 | 168 | 244 | 306),
                ..super::command_at(tick)
            };
            duel.step(command);
            tape.record(command, &duel);
        }
        assert_eq!(duel.outcome, super::Outcome::Won);
        assert_eq!(duel.pressure, 0);
        assert_eq!(tape.verify().unwrap().snapshot(), duel.snapshot());
    }
}
