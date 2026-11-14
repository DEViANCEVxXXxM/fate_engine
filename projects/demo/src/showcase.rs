//! Deterministic showcase simulation plus replay, rollback, and snapshot checks.
//!
//! The showcase is deliberately not a game: it advances a scripted input pattern
//! and renders the resulting fixed-point world. Verification runs alongside the
//! simulation so the demo can show engine seams without adding presentation
//! policy to the engine.

use std::collections::VecDeque;

use fate_engine::math::Vec3;
use fate_sim::ecs::{Entity, Position, Rotation, Spin, Velocity, WorldDiagnostics};
use fate_sim::math::{Fixed, FixedVec2};

use crate::demo_replay::{ReplayTape, RollbackSession};
use crate::prototype_simulation::{EngineCore, SimulationInput};
use crate::scene::CubeInstance;

const GRID_COLUMNS: usize = 16;
const GRID_ROWS: usize = 10;
const GRID_SPEED_NUMERATOR: i64 = 3;
const GRID_SPEED_DENOMINATOR: i64 = 2;
const SPIN_PER_TICK: i64 = 35;
pub const REPLAY_WINDOW: usize = 240;
const ROLLBACK_WINDOW: u64 = 120;
const ROLLBACK_INTERVAL: u64 = 240;
const SNAPSHOT_INTERVAL: u64 = 240;
const ROLLBACK_HISTORY: usize = 256;
const SNAPSHOT_HISTORY: usize = 256;

const PALETTE: [[f32; 3]; 8] = [
    [0.95, 0.32, 0.28],
    [0.98, 0.62, 0.22],
    [0.92, 0.84, 0.30],
    [0.42, 0.82, 0.40],
    [0.24, 0.76, 0.80],
    [0.30, 0.52, 0.96],
    [0.60, 0.40, 0.92],
    [0.92, 0.36, 0.66],
];

/// Deterministic, non-interactive input pattern used by the showcase.
pub fn scripted_input(tick: u64) -> SimulationInput {
    let phase = (tick / 90) % 8;
    let (move_x, move_y, boost) = match phase {
        0 => (1, 0, false),
        1 => (1, 1, false),
        2 => (0, 1, true),
        3 => (-1, 1, false),
        4 => (-1, 0, false),
        5 => (-1, -1, true),
        6 => (0, -1, false),
        _ => (1, -1, false),
    };
    SimulationInput {
        move_x,
        move_y,
        boost,
    }
}

/// Deterministic cube-grid simulation with interpolated render positions.
pub struct ShowcaseSim {
    core: EngineCore,
    tracked: Vec<Entity>,
    previous_positions: Vec<FixedVec2>,
    current_positions: Vec<FixedVec2>,
}

impl ShowcaseSim {
    pub fn new() -> Self {
        let mut core = EngineCore::new();

        for row in 0..GRID_ROWS {
            for column in 0..GRID_COLUMNS {
                let index = row * GRID_COLUMNS + column;
                let x = Fixed::from_ratio(column as i64 * 2 - (GRID_COLUMNS as i64 - 1), 2);
                let y = Fixed::from_ratio(row as i64 * 2 - (GRID_ROWS as i64 - 1), 2);
                let direction: (i64, i64) = match index % 8 {
                    0 => (1, 0),
                    1 => (1, 1),
                    2 => (0, 1),
                    3 => (-1, 1),
                    4 => (-1, 0),
                    5 => (-1, -1),
                    6 => (0, -1),
                    _ => (1, -1),
                };
                let velocity =
                    FixedVec2::new(Fixed::from_int(direction.0), Fixed::from_int(direction.1))
                        .normalize()
                        * Fixed::from_ratio(GRID_SPEED_NUMERATOR, GRID_SPEED_DENOMINATOR);
                let spin = Fixed::from_ratio(
                    if index.is_multiple_of(2) {
                        SPIN_PER_TICK
                    } else {
                        -SPIN_PER_TICK
                    },
                    100,
                );

                let entity = core.world.spawn();
                core.world
                    .set_position(entity, Position(FixedVec2::new(x, y)))
                    .unwrap();
                core.world.set_velocity(entity, Velocity(velocity)).unwrap();
                core.world
                    .set_rotation(entity, Rotation(Fixed::ZERO))
                    .unwrap();
                core.world.set_spin(entity, Spin(spin)).unwrap();
            }
        }

        let tracked: Vec<Entity> = core
            .world
            .positions()
            .iter()
            .map(|(entity, _)| entity)
            .collect();
        let positions: Vec<FixedVec2> = tracked
            .iter()
            .map(|entity| {
                core.world
                    .positions()
                    .get(*entity)
                    .map_or(FixedVec2::ZERO, |position| position.0)
            })
            .collect();

        Self {
            core,
            tracked,
            previous_positions: positions.clone(),
            current_positions: positions,
        }
    }

    /// Advance one fixed step and keep the previous positions for interpolation.
    pub fn advance(&mut self, input: SimulationInput) {
        std::mem::swap(&mut self.previous_positions, &mut self.current_positions);
        self.core.fixed_update(input);
        for (index, entity) in self.tracked.iter().enumerate() {
            self.current_positions[index] = self
                .core
                .world
                .positions()
                .get(*entity)
                .map_or(FixedVec2::ZERO, |position| position.0);
        }
    }

    /// Build interpolated cube instances for the renderer.
    pub fn render_instances(&self, alpha: f32) -> Vec<CubeInstance> {
        let alpha = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let tick = self.tick() as f32;
        let mut instances = Vec::with_capacity(self.tracked.len());

        for (index, entity) in self.tracked.iter().enumerate() {
            let previous = self.previous_positions[index];
            let current = self.current_positions[index];
            let x = previous.x.to_f32() + (current.x.to_f32() - previous.x.to_f32()) * alpha;
            let y = previous.y.to_f32() + (current.y.to_f32() - previous.y.to_f32()) * alpha;
            let rotation = self
                .core
                .world
                .rotations()
                .get(*entity)
                .map_or(0.0, |rotation| {
                    rotation.0.to_f32().rem_euclid(std::f32::consts::TAU)
                });
            let is_player = *entity == self.core.player;
            let bob = if is_player {
                0.0
            } else {
                (tick * 0.08 + index as f32 * 0.7).sin() * 0.25
            };
            let scale = if is_player {
                Vec3::new(1.6, 1.6, 1.6)
            } else {
                Vec3::new(0.9, 1.2, 0.9)
            };
            let tint = if is_player {
                [0.95, 0.95, 1.0]
            } else {
                PALETTE[index % PALETTE.len()]
            };

            instances.push(CubeInstance {
                position: Vec3::new(x, 0.9 + bob, -y),
                scale,
                rotation,
                tint,
            });
        }

        instances
    }

    pub fn tick(&self) -> u64 {
        self.core.tick()
    }

    pub fn state_bytes(&self) -> Vec<u8> {
        self.core.state_bytes()
    }

    pub fn state_hash(&self) -> u64 {
        self.core.state_hash()
    }

    pub fn tracked_count(&self) -> usize {
        self.tracked.len()
    }

    pub fn world_diagnostics(&self) -> WorldDiagnostics {
        self.core.world.diagnostics()
    }
}

impl Default for ShowcaseSim {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of one demonstration verification.
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

/// Store a verification result, logging the details whenever it fails.
///
/// Failures are exceptional, so each one is logged rather than only the first.
fn store_result(label: &'static str, tick: u64, slot: &mut VerifyResult, result: VerifyResult) {
    if let VerifyResult::Failed(message) = &result {
        log::error!(
            target: "fate.showcase",
            "{label} verification failed at tick {tick}: {message}"
        );
    }
    *slot = result;
}

/// Runs replay, rollback, and snapshot checks against the live simulation.
pub struct VerificationState {
    pub replay_enabled: bool,
    pub replay_result: VerifyResult,
    pub rollback_result: VerifyResult,
    pub snapshot_result: VerifyResult,
    replay_initial: Option<Vec<u8>>,
    replay_inputs: Vec<SimulationInput>,
    rollback: Option<RollbackSession>,
    snapshots: VecDeque<(u64, Vec<u8>)>,
    next_rollback_tick: u64,
    next_snapshot_tick: u64,
}

impl VerificationState {
    pub fn new(sim: &ShowcaseSim) -> Self {
        let rollback = EngineCore::from_state_bytes(&sim.state_bytes())
            .ok()
            .and_then(|core| RollbackSession::new(core, ROLLBACK_HISTORY).ok());
        let mut snapshots = VecDeque::with_capacity(SNAPSHOT_HISTORY);
        snapshots.push_back((sim.tick(), sim.state_bytes()));

        Self {
            replay_enabled: true,
            replay_result: VerifyResult::Pending,
            rollback_result: VerifyResult::Pending,
            snapshot_result: VerifyResult::Pending,
            replay_initial: Some(sim.state_bytes()),
            replay_inputs: Vec::with_capacity(REPLAY_WINDOW),
            rollback,
            snapshots,
            next_rollback_tick: ROLLBACK_INTERVAL,
            next_snapshot_tick: SNAPSHOT_INTERVAL,
        }
    }

    /// Record the input and advance the rollback mirror before the live step.
    pub fn begin_tick(&mut self, sim: &ShowcaseSim, input: SimulationInput) {
        if self.replay_enabled {
            if self.replay_initial.is_none() {
                self.replay_initial = Some(sim.state_bytes());
            }
            self.replay_inputs.push(input);
        }

        let advance_result = self.rollback.as_mut().map(|session| session.advance(input));
        if let Some(Err(error)) = advance_result {
            store_result(
                "rollback",
                sim.tick(),
                &mut self.rollback_result,
                VerifyResult::Failed(error.to_string()),
            );
            self.rollback = None;
        }
    }

    /// Store a snapshot and run any checks that are due after the live step.
    pub fn end_tick(&mut self, sim: &ShowcaseSim) {
        self.snapshots.push_back((sim.tick(), sim.state_bytes()));
        while self.snapshots.len() > SNAPSHOT_HISTORY {
            self.snapshots.pop_front();
        }

        if self.replay_enabled && self.replay_inputs.len() >= REPLAY_WINDOW {
            self.verify_replay(sim);
        }
        if sim.tick() >= self.next_rollback_tick {
            self.verify_rollback(sim);
            self.next_rollback_tick = sim.tick().saturating_add(ROLLBACK_INTERVAL);
        }
        if sim.tick() >= self.next_snapshot_tick {
            self.verify_snapshot(sim);
            self.next_snapshot_tick = sim.tick().saturating_add(SNAPSHOT_INTERVAL);
        }
    }

    pub fn set_replay_enabled(&mut self, enabled: bool, sim: &ShowcaseSim) {
        if self.replay_enabled == enabled {
            return;
        }
        self.replay_enabled = enabled;
        self.replay_inputs.clear();
        self.replay_result = VerifyResult::Pending;
        self.replay_initial = if enabled {
            Some(sim.state_bytes())
        } else {
            None
        };
    }

    pub fn force_snapshot_check(&mut self, sim: &ShowcaseSim) {
        self.verify_snapshot(sim);
    }

    fn verify_replay(&mut self, sim: &ShowcaseSim) {
        let initial = self.replay_initial.take();
        let inputs = std::mem::take(&mut self.replay_inputs);
        let result = match initial {
            Some(initial) => verify_replay_window(&initial, &inputs, sim),
            None => VerifyResult::Failed("replay window has no initial state".to_string()),
        };
        store_result("replay", sim.tick(), &mut self.replay_result, result);
        self.replay_initial = Some(sim.state_bytes());
    }

    fn verify_rollback(&mut self, sim: &ShowcaseSim) {
        let target_tick = sim.tick().saturating_sub(ROLLBACK_WINDOW);
        let snapshot = self
            .snapshots
            .iter()
            .find(|(tick, _)| *tick == target_tick)
            .map(|(_, bytes)| bytes.clone());
        let Some(snapshot) = snapshot else {
            self.rollback_result = VerifyResult::Pending;
            return;
        };
        let Some(session) = self.rollback.as_mut() else {
            return;
        };

        let result = match session.reconcile(&snapshot) {
            Ok(()) => {
                let hash = session.core().state_hash();
                if hash == sim.state_hash() {
                    VerifyResult::Ok {
                        ticks: ROLLBACK_WINDOW,
                        hash,
                    }
                } else {
                    VerifyResult::Failed(format!(
                        "hash mismatch after reconcile: rollback {hash:016x}, live {:016x}",
                        sim.state_hash()
                    ))
                }
            }
            Err(error) => VerifyResult::Failed(error.to_string()),
        };
        store_result("rollback", sim.tick(), &mut self.rollback_result, result);
    }

    fn verify_snapshot(&mut self, sim: &ShowcaseSim) {
        let bytes = sim.state_bytes();
        let result = match EngineCore::from_state_bytes(&bytes) {
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

fn verify_replay_window(
    initial_bytes: &[u8],
    inputs: &[SimulationInput],
    sim: &ShowcaseSim,
) -> VerifyResult {
    let initial = match EngineCore::from_state_bytes(initial_bytes) {
        Ok(initial) => initial,
        Err(error) => return VerifyResult::Failed(error.to_string()),
    };
    let tape = match ReplayTape::record(&initial, inputs) {
        Ok(tape) => tape,
        Err(error) => return VerifyResult::Failed(error.to_string()),
    };
    let replayed = match tape.playback() {
        Ok(replayed) => replayed,
        Err(error) => return VerifyResult::Failed(error.to_string()),
    };

    let hash = replayed.state_hash();
    if hash != sim.state_hash() {
        return VerifyResult::Failed(format!(
            "hash mismatch: replay {hash:016x}, live {:016x}",
            sim.state_hash()
        ));
    }

    VerifyResult::Ok {
        ticks: inputs.len() as u64,
        hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_ticks(sim: &mut ShowcaseSim, verification: &mut VerificationState, ticks: usize) {
        for _ in 0..ticks {
            let input = scripted_input(sim.tick());
            verification.begin_tick(sim, input);
            sim.advance(input);
            verification.end_tick(sim);
        }
    }

    #[test]
    fn scripted_input_is_deterministic_and_bounded() {
        for tick in 0..1_000 {
            let input = scripted_input(tick);
            assert!((-1..=1).contains(&input.move_x));
            assert!((-1..=1).contains(&input.move_y));
            assert_eq!(input, scripted_input(tick));
        }
    }

    #[test]
    fn showcase_simulation_is_deterministic() {
        let mut first = ShowcaseSim::new();
        let mut second = ShowcaseSim::new();

        for _ in 0..300 {
            let input = scripted_input(first.tick());
            first.advance(input);
            second.advance(input);
        }

        assert_eq!(first.state_hash(), second.state_hash());
        assert_eq!(first.state_bytes(), second.state_bytes());
        let world = first.world_diagnostics();
        assert_eq!(world.live_entities, GRID_COLUMNS * GRID_ROWS + 1);
        assert_eq!(first.tracked_count(), world.live_entities);
    }

    #[test]
    fn alpha_interpolates_between_previous_and_current_positions() {
        let mut sim = ShowcaseSim::new();
        let input = scripted_input(0);
        sim.advance(input);

        let at_zero = sim.render_instances(0.0);
        let at_one = sim.render_instances(1.0);

        assert_eq!(at_zero.len(), sim.tracked_count());
        assert!(
            at_zero
                .iter()
                .zip(&at_one)
                .any(|(start, end)| { (start.position - end.position).length() > 1e-6 })
        );
    }

    #[test]
    fn render_instances_sanitizes_invalid_alpha() {
        let mut sim = ShowcaseSim::new();
        sim.advance(scripted_input(0));

        let nan = sim.render_instances(f32::NAN);
        let clamped_high = sim.render_instances(f32::INFINITY);
        let one = sim.render_instances(1.0);

        assert_eq!(nan.len(), one.len());
        for ((nan, high), one) in nan.iter().zip(&clamped_high).zip(&one) {
            assert_eq!(nan.position, one.position);
            assert_eq!(nan.rotation, one.rotation);
            assert_eq!(high.position, one.position);
            assert_eq!(high.rotation, one.rotation);
        }
    }

    #[test]
    fn render_rotations_stay_bounded_over_long_runs() {
        let mut sim = ShowcaseSim::new();
        for _ in 0..5_000 {
            sim.advance(scripted_input(sim.tick()));
        }

        for instance in sim.render_instances(1.0) {
            assert!(
                (0.0..std::f32::consts::TAU).contains(&instance.rotation),
                "rotation {} is outside [0, TAU)",
                instance.rotation
            );
        }
    }

    #[test]
    fn replay_rollback_and_snapshot_checks_pass_on_a_clean_run() {
        let mut sim = ShowcaseSim::new();
        let mut verification = VerificationState::new(&sim);
        run_ticks(&mut sim, &mut verification, REPLAY_WINDOW);

        assert!(matches!(
            &verification.replay_result,
            VerifyResult::Ok { ticks, .. } if *ticks == REPLAY_WINDOW as u64
        ));
        assert!(matches!(
            &verification.rollback_result,
            VerifyResult::Ok { .. }
        ));
        assert!(matches!(
            &verification.snapshot_result,
            VerifyResult::Ok { .. }
        ));
    }

    #[test]
    fn replay_toggle_starts_a_fresh_window() {
        let mut sim = ShowcaseSim::new();
        let mut verification = VerificationState::new(&sim);

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
        let mut sim = ShowcaseSim::new();
        let mut verification = VerificationState::new(&sim);
        run_ticks(&mut sim, &mut verification, 30);

        verification.force_snapshot_check(&sim);

        assert!(matches!(
            &verification.snapshot_result,
            VerifyResult::Ok { .. }
        ));
    }
}
