//! In-memory replay recording and deterministic playback.
//!
//! The slice uses this to prove that a recorded input sequence reproduces the
//! live state exactly. Replay data is project-owned and is not part of a
//! reusable engine crate.

use std::fmt;

use crate::sim::{MAX_LOOK_INPUT, Sim, SimInput, StateDecodeError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReplayFrame {
    input: SimInput,
    expected_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    InvalidInput,
    InvalidSnapshot(StateDecodeError),
    Diverged {
        tick: u64,
        expected: u64,
        actual: u64,
    },
}

impl fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput => formatter.write_str("invalid input in replay data"),
            Self::InvalidSnapshot(error) => write!(formatter, "invalid initial state: {error}"),
            Self::Diverged {
                tick,
                expected,
                actual,
            } => write!(
                formatter,
                "replay diverged at tick {tick}: expected {expected:016x}, got {actual:016x}"
            ),
        }
    }
}

impl std::error::Error for ReplayError {}

/// A recorded initial state plus one input per simulated tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayTape {
    initial_state: Vec<u8>,
    frames: Vec<ReplayFrame>,
}

impl ReplayTape {
    /// Simulate `inputs` from `initial` and record the resulting state hash for
    /// every tick.
    pub fn record(initial: &Sim, inputs: &[SimInput]) -> Result<Self, ReplayError> {
        let initial_state = initial.state_bytes();
        let mut sim =
            Sim::from_state_bytes(&initial_state).map_err(ReplayError::InvalidSnapshot)?;
        let mut frames = Vec::with_capacity(inputs.len());
        for input in inputs {
            validate_input(*input)?;
            sim.step(*input);
            frames.push(ReplayFrame {
                input: *input,
                expected_hash: sim.state_hash(),
            });
        }

        Ok(Self {
            initial_state,
            frames,
        })
    }

    pub fn tick_count(&self) -> usize {
        self.frames.len()
    }

    /// Replay every frame and verify each recorded hash.
    pub fn playback(&self) -> Result<Sim, ReplayError> {
        let mut sim =
            Sim::from_state_bytes(&self.initial_state).map_err(ReplayError::InvalidSnapshot)?;
        for frame in &self.frames {
            sim.step(frame.input);
            let actual = sim.state_hash();
            if actual != frame.expected_hash {
                return Err(ReplayError::Diverged {
                    tick: sim.tick(),
                    expected: frame.expected_hash,
                    actual,
                });
            }
        }
        Ok(sim)
    }
}

fn validate_input(input: SimInput) -> Result<(), ReplayError> {
    if !(-1..=1).contains(&input.move_x)
        || !(-1..=1).contains(&input.move_y)
        || !(-MAX_LOOK_INPUT..=MAX_LOOK_INPUT).contains(&input.look_yaw)
        || !(-MAX_LOOK_INPUT..=MAX_LOOK_INPUT).contains(&input.look_pitch)
    {
        return Err(ReplayError::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_inputs(count: usize) -> Vec<SimInput> {
        (0..count)
            .map(|tick| SimInput {
                move_x: match tick % 3 {
                    0 => 1,
                    1 => 0,
                    _ => -1,
                },
                move_y: if tick % 2 == 0 { 1 } else { -1 },
                look_yaw: (tick % 25) as i16 * 3 - 36,
                look_pitch: (tick % 15) as i16 - 7,
                sprint: tick % 40 == 0,
                fire: tick % 20 == 0,
                reload: tick % 80 == 0,
            })
            .collect()
    }

    #[test]
    fn replay_reproduces_the_final_state() {
        let initial = Sim::new();
        let inputs = sample_inputs(240);
        let tape = ReplayTape::record(&initial, &inputs).unwrap();

        let mut live = Sim::new();
        for input in &inputs {
            live.step(*input);
        }

        assert_eq!(tape.tick_count(), 240);
        let replayed = tape.playback().unwrap();
        assert_eq!(replayed.tick(), 240);
        assert_eq!(replayed.state_bytes(), live.state_bytes());
    }

    #[test]
    fn replay_reports_the_first_diverging_tick() {
        let initial = Sim::new();
        let inputs = sample_inputs(4);
        let mut tape = ReplayTape::record(&initial, &inputs).unwrap();
        tape.frames[0].input.move_x = -tape.frames[0].input.move_x;

        assert!(matches!(
            tape.playback(),
            Err(ReplayError::Diverged { tick: 1, .. })
        ));
    }

    #[test]
    fn recorder_rejects_out_of_range_axes() {
        let invalid = [SimInput {
            move_x: 2,
            ..SimInput::default()
        }];
        assert_eq!(
            ReplayTape::record(&Sim::new(), &invalid),
            Err(ReplayError::InvalidInput)
        );
    }
}
