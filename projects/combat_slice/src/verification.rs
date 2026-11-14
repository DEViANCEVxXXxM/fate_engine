//! Replay and snapshot verification for the live slice.
//!
//! Verification runs beside the playable simulation. It is deliberately
//! consumer-owned: the engine exposes deterministic state and hashing, while
//! this project decides when and how to check them.

use crate::replay::ReplayTape;
use crate::sim::{Sim, SimInput};

/// Number of ticks in one replay verification window.
pub const REPLAY_WINDOW: usize = 240;
/// Ticks between automatic snapshot round-trip checks.
pub const SNAPSHOT_INTERVAL: u64 = 240;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyResult {
    Pending,
    Ok { ticks: u64, hash: u64 },
    Failed(String),
}

impl VerifyResult {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ok { .. } => "ok",
            Self::Failed(_) => "failed",
        }
    }
}

fn store_result(label: &'static str, tick: u64, slot: &mut VerifyResult, result: VerifyResult) {
    if let VerifyResult::Failed(message) = &result {
        log::error!(
            target: "fate.combat",
            "{label} verification failed at tick {tick}: {message}"
        );
    }
    *slot = result;
}

/// Replay window plus periodic snapshot checks.
pub struct Verification {
    pub replay_enabled: bool,
    pub replay_result: VerifyResult,
    pub snapshot_result: VerifyResult,
    replay_initial: Option<Vec<u8>>,
    replay_inputs: Vec<SimInput>,
    next_snapshot_tick: u64,
}

impl Verification {
    pub fn new(sim: &Sim) -> Self {
        Self {
            replay_enabled: true,
            replay_result: VerifyResult::Pending,
            snapshot_result: VerifyResult::Pending,
            replay_initial: Some(sim.state_bytes()),
            replay_inputs: Vec::with_capacity(REPLAY_WINDOW),
            next_snapshot_tick: SNAPSHOT_INTERVAL,
        }
    }

    /// Record the input that is about to be applied to the live simulation.
    pub fn begin_tick(&mut self, sim: &Sim, input: SimInput) {
        if !self.replay_enabled {
            return;
        }
        if self.replay_initial.is_none() {
            self.replay_initial = Some(sim.state_bytes());
        }
        self.replay_inputs.push(input);
    }

    /// Run any checks that are due after the live step.
    pub fn end_tick(&mut self, sim: &Sim) {
        if self.replay_enabled && self.replay_inputs.len() >= REPLAY_WINDOW {
            self.verify_replay(sim);
        }
        if sim.tick() >= self.next_snapshot_tick {
            self.verify_snapshot(sim);
            self.next_snapshot_tick = sim.tick().saturating_add(SNAPSHOT_INTERVAL);
        }
    }

    pub fn set_replay_enabled(&mut self, enabled: bool, sim: &Sim) {
        if self.replay_enabled == enabled {
            return;
        }
        self.replay_enabled = enabled;
        self.replay_inputs.clear();
        self.replay_result = VerifyResult::Pending;
        self.replay_initial = enabled.then(|| sim.state_bytes());
    }

    pub fn force_snapshot_check(&mut self, sim: &Sim) {
        self.verify_snapshot(sim);
    }

    fn verify_replay(&mut self, sim: &Sim) {
        let initial = self.replay_initial.take();
        let inputs = std::mem::take(&mut self.replay_inputs);
        let result = match initial {
            Some(bytes) => match Sim::from_state_bytes(&bytes) {
                Ok(initial) => match ReplayTape::record(&initial, &inputs) {
                    Ok(tape) => {
                        let ticks = tape.tick_count() as u64;
                        match tape.playback() {
                            Ok(replayed) => {
                                let hash = replayed.state_hash();
                                if hash == sim.state_hash() {
                                    VerifyResult::Ok { ticks, hash }
                                } else {
                                    VerifyResult::Failed(format!(
                                        "hash mismatch: replay {hash:016x}, live {:016x}",
                                        sim.state_hash()
                                    ))
                                }
                            }
                            Err(error) => VerifyResult::Failed(error.to_string()),
                        }
                    }
                    Err(error) => VerifyResult::Failed(error.to_string()),
                },
                Err(error) => VerifyResult::Failed(error.to_string()),
            },
            None => VerifyResult::Failed("replay window has no initial state".to_string()),
        };
        store_result("replay", sim.tick(), &mut self.replay_result, result);
        self.replay_initial = Some(sim.state_bytes());
    }

    fn verify_snapshot(&mut self, sim: &Sim) {
        let bytes = sim.state_bytes();
        let result = match Sim::from_state_bytes(&bytes) {
            Ok(restored) => {
                let hash = restored.state_hash();
                if restored.state_bytes() == bytes && hash == sim.state_hash() {
                    VerifyResult::Ok {
                        ticks: sim.tick(),
                        hash,
                    }
                } else {
                    VerifyResult::Failed("snapshot round-trip mismatch".to_string())
                }
            }
            Err(error) => VerifyResult::Failed(error.to_string()),
        };
        store_result("snapshot", sim.tick(), &mut self.snapshot_result, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scripted_input(tick: u64) -> SimInput {
        SimInput {
            move_x: match (tick / 20) % 3 {
                0 => 1,
                1 => 0,
                _ => -1,
            },
            move_y: if tick % 60 < 30 { 1 } else { -1 },
            look_yaw: (tick % 30) as i16 - 15,
            look_pitch: (tick % 10) as i16 - 5,
            sprint: tick.is_multiple_of(50),
            fire: tick.is_multiple_of(25),
            reload: tick.is_multiple_of(100),
        }
    }

    fn run_ticks(sim: &mut Sim, verification: &mut Verification, ticks: usize) {
        for _ in 0..ticks {
            let input = scripted_input(sim.tick());
            verification.begin_tick(sim, input);
            sim.step(input);
            verification.end_tick(sim);
        }
    }

    #[test]
    fn clean_run_passes_replay_and_snapshot_checks() {
        let mut sim = Sim::new();
        let mut verification = Verification::new(&sim);
        run_ticks(&mut sim, &mut verification, REPLAY_WINDOW);

        assert!(matches!(
            &verification.replay_result,
            VerifyResult::Ok { ticks, .. } if *ticks == REPLAY_WINDOW as u64
        ));
        assert!(matches!(
            &verification.snapshot_result,
            VerifyResult::Ok { .. }
        ));
    }

    #[test]
    fn replay_toggle_starts_a_fresh_window() {
        let mut sim = Sim::new();
        let mut verification = Verification::new(&sim);

        verification.set_replay_enabled(false, &sim);
        run_ticks(&mut sim, &mut verification, REPLAY_WINDOW + 60);
        assert_eq!(verification.replay_result, VerifyResult::Pending);

        verification.set_replay_enabled(true, &sim);
        run_ticks(&mut sim, &mut verification, REPLAY_WINDOW);
        assert!(matches!(
            &verification.replay_result,
            VerifyResult::Ok { .. }
        ));
    }

    #[test]
    fn forced_snapshot_check_reports_ok() {
        let mut sim = Sim::new();
        let mut verification = Verification::new(&sim);
        run_ticks(&mut sim, &mut verification, 30);

        verification.force_snapshot_check(&sim);

        assert!(matches!(
            &verification.snapshot_result,
            VerifyResult::Ok { .. }
        ));
    }
}
