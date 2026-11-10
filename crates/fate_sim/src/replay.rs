//! Replay serialization and bounded rollback built on the deterministic simulation core.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use crate::simulation::{EngineCore, SimulationInput, StateDecodeError};

const REPLAY_VERSION: u16 = 1;
const MAX_REPLAY_TICKS: usize = 10_000_000;
const FRAME_BYTES: usize = 11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReplayFrame {
    input: SimulationInput,
    expected_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    BadMagic,
    Truncated,
    UnsupportedVersion(u16),
    InvalidInput,
    TooManyTicks,
    TrailingBytes,
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
            Self::BadMagic => formatter.write_str("invalid replay signature"),
            Self::Truncated => formatter.write_str("truncated replay data"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported replay version {version}")
            }
            Self::InvalidInput => formatter.write_str("invalid input in replay data"),
            Self::TooManyTicks => formatter.write_str("replay exceeds the tick limit"),
            Self::TrailingBytes => formatter.write_str("unexpected trailing bytes in replay"),
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayTape {
    initial_state: Vec<u8>,
    frames: Vec<ReplayFrame>,
}

impl ReplayTape {
    pub fn record(initial: &EngineCore, inputs: &[SimulationInput]) -> Result<Self, ReplayError> {
        if inputs.len() > MAX_REPLAY_TICKS {
            return Err(ReplayError::TooManyTicks);
        }

        let initial_state = initial.state_bytes();
        let mut core =
            EngineCore::from_state_bytes(&initial_state).map_err(ReplayError::InvalidSnapshot)?;
        let mut frames = Vec::with_capacity(inputs.len());
        for input in inputs {
            validate_input(*input)?;
            core.fixed_update(*input);
            frames.push(ReplayFrame {
                input: *input,
                expected_hash: core.state_hash(),
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

    pub fn playback(&self) -> Result<EngineCore, ReplayError> {
        let mut core = EngineCore::from_state_bytes(&self.initial_state)
            .map_err(ReplayError::InvalidSnapshot)?;
        for frame in &self.frames {
            core.fixed_update(frame.input);
            let actual = core.state_hash();
            if actual != frame.expected_hash {
                return Err(ReplayError::Diverged {
                    tick: core.tick(),
                    expected: frame.expected_hash,
                    actual,
                });
            }
        }
        Ok(core)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, ReplayError> {
        if self.frames.len() > MAX_REPLAY_TICKS {
            return Err(ReplayError::TooManyTicks);
        }
        let state_len =
            u32::try_from(self.initial_state.len()).map_err(|_| ReplayError::Truncated)?;
        let frame_count =
            u32::try_from(self.frames.len()).map_err(|_| ReplayError::TooManyTicks)?;

        let mut bytes = Vec::with_capacity(
            14 + self.initial_state.len() + self.frames.len().saturating_mul(FRAME_BYTES),
        );
        bytes.extend_from_slice(b"FREP");
        bytes.extend_from_slice(&REPLAY_VERSION.to_le_bytes());
        bytes.extend_from_slice(&state_len.to_le_bytes());
        bytes.extend_from_slice(&self.initial_state);
        bytes.extend_from_slice(&frame_count.to_le_bytes());
        for frame in &self.frames {
            validate_input(frame.input)?;
            bytes.push(frame.input.move_x as u8);
            bytes.push(frame.input.move_y as u8);
            bytes.push(frame.input.boost as u8);
            bytes.extend_from_slice(&frame.expected_hash.to_le_bytes());
        }
        Ok(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReplayError> {
        let mut offset = 0;
        if read_bytes::<4>(bytes, &mut offset)? != *b"FREP" {
            return Err(ReplayError::BadMagic);
        }
        let version = read_u16(bytes, &mut offset)?;
        if version != REPLAY_VERSION {
            return Err(ReplayError::UnsupportedVersion(version));
        }
        let state_len = read_u32(bytes, &mut offset)? as usize;
        let state_end = offset
            .checked_add(state_len)
            .ok_or(ReplayError::Truncated)?;
        let initial_state = bytes
            .get(offset..state_end)
            .ok_or(ReplayError::Truncated)?
            .to_vec();
        EngineCore::from_state_bytes(&initial_state).map_err(ReplayError::InvalidSnapshot)?;
        offset = state_end;

        let frame_count = read_u32(bytes, &mut offset)? as usize;
        if frame_count > MAX_REPLAY_TICKS {
            return Err(ReplayError::TooManyTicks);
        }
        if offset > bytes.len() || bytes.len() - offset != frame_count.saturating_mul(FRAME_BYTES) {
            return Err(
                if offset > bytes.len()
                    || bytes.len() - offset < frame_count.saturating_mul(FRAME_BYTES)
                {
                    ReplayError::Truncated
                } else {
                    ReplayError::TrailingBytes
                },
            );
        }

        let mut frames = Vec::with_capacity(frame_count);
        for _ in 0..frame_count {
            let move_x = read_u8(bytes, &mut offset)? as i8;
            let move_y = read_u8(bytes, &mut offset)? as i8;
            let boost = match read_u8(bytes, &mut offset)? {
                0 => false,
                1 => true,
                _ => return Err(ReplayError::InvalidInput),
            };
            let input = SimulationInput {
                move_x,
                move_y,
                boost,
            };
            validate_input(input)?;
            let expected_hash = read_u64(bytes, &mut offset)?;
            frames.push(ReplayFrame {
                input,
                expected_hash,
            });
        }

        if offset != bytes.len() {
            return Err(ReplayError::TrailingBytes);
        }
        Ok(Self {
            initial_state,
            frames,
        })
    }
}

fn validate_input(input: SimulationInput) -> Result<(), ReplayError> {
    if !(-1..=1).contains(&input.move_x) || !(-1..=1).contains(&input.move_y) {
        return Err(ReplayError::InvalidInput);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RollbackError {
    ZeroHistoryCapacity,
    InvalidInput,
    TickOverflow,
    FutureSnapshot {
        snapshot_tick: u64,
        current_tick: u64,
    },
    HistoryUnavailable {
        requested_tick: u64,
        oldest_reconstructable_tick: u64,
    },
    CorrectionTickOutOfRange {
        tick: u64,
        authoritative_tick: u64,
        current_tick: u64,
    },
    DuplicateCorrectionTick(u64),
    InvalidSnapshot(StateDecodeError),
}

impl fmt::Display for RollbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroHistoryCapacity => {
                formatter.write_str("rollback history capacity must be non-zero")
            }
            Self::InvalidInput => formatter.write_str("invalid rollback input"),
            Self::TickOverflow => formatter.write_str("simulation tick counter is exhausted"),
            Self::FutureSnapshot {
                snapshot_tick,
                current_tick,
            } => write!(
                formatter,
                "authoritative tick {snapshot_tick} is ahead of local tick {current_tick}"
            ),
            Self::HistoryUnavailable {
                requested_tick,
                oldest_reconstructable_tick,
            } => write!(
                formatter,
                "cannot restore tick {requested_tick}; oldest reconstructable tick is {oldest_reconstructable_tick}"
            ),
            Self::CorrectionTickOutOfRange {
                tick,
                authoritative_tick,
                current_tick,
            } => write!(
                formatter,
                "corrected tick {tick} is outside ({authoritative_tick}, {current_tick}]"
            ),
            Self::DuplicateCorrectionTick(tick) => {
                write!(
                    formatter,
                    "multiple input corrections supplied for tick {tick}"
                )
            }
            Self::InvalidSnapshot(error) => {
                write!(formatter, "invalid authoritative state: {error}")
            }
        }
    }
}

impl std::error::Error for RollbackError {}

struct InputAtTick {
    tick: u64,
    input: SimulationInput,
}

pub struct RollbackSession {
    core: EngineCore,
    history: VecDeque<InputAtTick>,
    history_capacity: usize,
}

impl RollbackSession {
    pub fn new(core: EngineCore, history_capacity: usize) -> Result<Self, RollbackError> {
        if history_capacity == 0 {
            return Err(RollbackError::ZeroHistoryCapacity);
        }
        Ok(Self {
            core,
            history: VecDeque::with_capacity(history_capacity),
            history_capacity,
        })
    }

    pub fn core(&self) -> &EngineCore {
        &self.core
    }

    pub fn advance(&mut self, input: SimulationInput) -> Result<(), RollbackError> {
        if validate_input(input).is_err() {
            return Err(RollbackError::InvalidInput);
        }
        let tick = self
            .core
            .tick()
            .checked_add(1)
            .ok_or(RollbackError::TickOverflow)?;

        self.core.fixed_update(input);
        self.history.push_back(InputAtTick { tick, input });
        if self.history.len() > self.history_capacity {
            self.history.pop_front();
        }
        Ok(())
    }

    /// Restore the authoritative snapshot and deterministically resimulate local
    /// inputs through the current tick. The session is unchanged on failure.
    pub fn reconcile(&mut self, authoritative: &[u8]) -> Result<(), RollbackError> {
        self.reconcile_with_corrections(authoritative, &[])
    }

    /// Reconcile from an authoritative state and replace buffered predicted
    /// inputs at the specified simulation ticks before resimulation.
    pub fn reconcile_with_corrections(
        &mut self,
        authoritative: &[u8],
        corrections: &[(u64, SimulationInput)],
    ) -> Result<(), RollbackError> {
        let authoritative =
            EngineCore::from_state_bytes(authoritative).map_err(RollbackError::InvalidSnapshot)?;
        let authoritative_tick = authoritative.tick();
        let current_tick = self.core.tick();
        if authoritative_tick > current_tick {
            return Err(RollbackError::FutureSnapshot {
                snapshot_tick: authoritative_tick,
                current_tick,
            });
        }

        let mut corrected_inputs = BTreeMap::new();
        for (tick, input) in corrections {
            if validate_input(*input).is_err() {
                return Err(RollbackError::InvalidInput);
            }
            if *tick <= authoritative_tick || *tick > current_tick {
                return Err(RollbackError::CorrectionTickOutOfRange {
                    tick: *tick,
                    authoritative_tick,
                    current_tick,
                });
            }
            if corrected_inputs.insert(*tick, *input).is_some() {
                return Err(RollbackError::DuplicateCorrectionTick(*tick));
            }
        }

        let mut replayed_core = authoritative;
        if authoritative_tick < current_tick {
            let first_needed_tick = authoritative_tick
                .checked_add(1)
                .ok_or(RollbackError::TickOverflow)?;
            let oldest_reconstructable_tick = self
                .history
                .front()
                .map_or(current_tick, |entry| entry.tick.saturating_sub(1));
            let mut expected_tick = first_needed_tick;
            for entry in self
                .history
                .iter()
                .filter(|entry| entry.tick > authoritative_tick)
            {
                if entry.tick != expected_tick {
                    return Err(RollbackError::HistoryUnavailable {
                        requested_tick: authoritative_tick,
                        oldest_reconstructable_tick,
                    });
                }
                let input = corrected_inputs
                    .get(&entry.tick)
                    .copied()
                    .unwrap_or(entry.input);
                replayed_core.fixed_update(input);
                expected_tick = expected_tick
                    .checked_add(1)
                    .ok_or(RollbackError::TickOverflow)?;
            }
            if expected_tick != current_tick.saturating_add(1) {
                return Err(RollbackError::HistoryUnavailable {
                    requested_tick: authoritative_tick,
                    oldest_reconstructable_tick,
                });
            }
        }

        for entry in &mut self.history {
            if let Some(input) = corrected_inputs.get(&entry.tick) {
                entry.input = *input;
            }
        }
        self.core = replayed_core;
        Ok(())
    }
}

fn read_bytes<const N: usize>(bytes: &[u8], offset: &mut usize) -> Result<[u8; N], ReplayError> {
    let end = offset.checked_add(N).ok_or(ReplayError::Truncated)?;
    let value = bytes
        .get(*offset..end)
        .ok_or(ReplayError::Truncated)?
        .try_into()
        .expect("fixed-size slice");
    *offset = end;
    Ok(value)
}

fn read_u8(bytes: &[u8], offset: &mut usize) -> Result<u8, ReplayError> {
    Ok(read_bytes::<1>(bytes, offset)?[0])
}

fn read_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, ReplayError> {
    Ok(u16::from_le_bytes(read_bytes(bytes, offset)?))
}

fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, ReplayError> {
    Ok(u32::from_le_bytes(read_bytes(bytes, offset)?))
}

fn read_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, ReplayError> {
    Ok(u64::from_le_bytes(read_bytes(bytes, offset)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_inputs(count: usize) -> Vec<SimulationInput> {
        (0..count)
            .map(|tick| SimulationInput {
                move_x: match tick % 3 {
                    0 => 1,
                    1 => 0,
                    _ => -1,
                },
                move_y: if tick % 2 == 0 { 1 } else { 0 },
                boost: tick % 5 == 0,
            })
            .collect()
    }

    #[test]
    fn serialized_replay_round_trips_and_replays_to_the_expected_state() {
        let initial = EngineCore::new();
        let tape = ReplayTape::record(&initial, &sample_inputs(240)).unwrap();
        let restored_tape = ReplayTape::from_bytes(&tape.to_bytes().unwrap()).unwrap();

        assert_eq!(restored_tape.tick_count(), 240);
        assert_eq!(restored_tape, tape);
        let replayed = restored_tape.playback().unwrap();
        assert_eq!(replayed.tick(), 240);
    }

    #[test]
    fn corrupted_input_or_checkpoint_is_reported_at_the_diverging_tick() {
        let initial = EngineCore::new();
        let tape = ReplayTape::record(&initial, &sample_inputs(4)).unwrap();
        let mut bytes = tape.to_bytes().unwrap();
        let frame_start = 10 + tape.initial_state.len() + 4;
        bytes[frame_start + 3] ^= 1;

        let corrupted = ReplayTape::from_bytes(&bytes).unwrap();
        assert!(matches!(
            corrupted.playback(),
            Err(ReplayError::Diverged { tick: 1, .. })
        ));
    }

    #[test]
    fn replay_decoder_rejects_truncation_invalid_input_and_unknown_versions() {
        let tape = ReplayTape::record(&EngineCore::new(), &sample_inputs(2)).unwrap();
        let bytes = tape.to_bytes().unwrap();

        for length in 0..bytes.len() {
            assert!(ReplayTape::from_bytes(&bytes[..length]).is_err());
        }

        let mut bad_version = bytes.clone();
        bad_version[4..6].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(matches!(
            ReplayTape::from_bytes(&bad_version),
            Err(ReplayError::UnsupportedVersion(u16::MAX))
        ));

        let mut bad_input = bytes;
        let frame_start = 10 + tape.initial_state.len() + 4;
        bad_input[frame_start] = 2;
        assert!(matches!(
            ReplayTape::from_bytes(&bad_input),
            Err(ReplayError::InvalidInput)
        ));
    }

    #[test]
    fn recorder_rejects_out_of_range_command_axes() {
        let invalid = [SimulationInput {
            move_x: 2,
            move_y: 0,
            boost: false,
        }];
        assert_eq!(
            ReplayTape::record(&EngineCore::new(), &invalid),
            Err(ReplayError::InvalidInput)
        );
    }

    #[test]
    fn rollback_reconciles_authoritative_state_and_replays_to_current_tick() {
        let inputs = sample_inputs(24);
        let mut authoritative = EngineCore::new();
        let mut session = RollbackSession::new(EngineCore::new(), inputs.len()).unwrap();

        for input in inputs.iter().take(10) {
            authoritative.fixed_update(*input);
        }
        for input in &inputs {
            session.advance(*input).unwrap();
        }

        session.reconcile(&authoritative.state_bytes()).unwrap();
        let mut expected = authoritative;
        for input in inputs.iter().skip(10) {
            expected.fixed_update(*input);
        }
        assert_eq!(session.core().state_bytes(), expected.state_bytes());
    }

    #[test]
    fn rollback_rejects_unavailable_history_without_changing_live_state() {
        let initial = EngineCore::new();
        let initial_state = initial.state_bytes();
        let mut session = RollbackSession::new(initial, 3).unwrap();
        for input in sample_inputs(8) {
            session.advance(input).unwrap();
        }
        let before = session.core().state_bytes();

        assert!(matches!(
            session.reconcile(&initial_state),
            Err(RollbackError::HistoryUnavailable { .. })
        ));
        assert_eq!(session.core().state_bytes(), before);
    }

    #[test]
    fn rollback_rejects_future_snapshots_and_invalid_commands() {
        assert_eq!(
            RollbackSession::new(EngineCore::new(), 0).err(),
            Some(RollbackError::ZeroHistoryCapacity)
        );

        let mut future = EngineCore::new();
        future.fixed_update(SimulationInput::default());
        let mut session = RollbackSession::new(EngineCore::new(), 8).unwrap();
        assert!(matches!(
            session.reconcile(&future.state_bytes()),
            Err(RollbackError::FutureSnapshot { .. })
        ));

        let before = session.core().state_bytes();
        assert_eq!(
            session.advance(SimulationInput {
                move_x: 2,
                move_y: 0,
                boost: false,
            }),
            Err(RollbackError::InvalidInput)
        );
        assert_eq!(session.core().state_bytes(), before);
    }

    #[test]
    fn rollback_applies_server_input_corrections_before_resimulation() {
        let predicted = [SimulationInput::default(); 16];
        let mut session = RollbackSession::new(EngineCore::new(), 16).unwrap();
        for input in predicted.iter().copied() {
            session.advance(input).unwrap();
        }

        let mut authoritative = EngineCore::new();
        for input in predicted.iter().take(6) {
            authoritative.fixed_update(*input);
        }
        let corrections = [
            (
                8,
                SimulationInput {
                    move_x: 1,
                    move_y: 0,
                    boost: false,
                },
            ),
            (
                12,
                SimulationInput {
                    move_x: 0,
                    move_y: -1,
                    boost: true,
                },
            ),
        ];
        session
            .reconcile_with_corrections(&authoritative.state_bytes(), &corrections)
            .unwrap();

        let mut expected = authoritative;
        for tick in 7..=16 {
            let input = corrections
                .iter()
                .find_map(|(corrected_tick, input)| (*corrected_tick == tick).then_some(*input))
                .unwrap_or_default();
            expected.fixed_update(input);
        }
        assert_eq!(session.core().state_bytes(), expected.state_bytes());

        session
            .reconcile_with_corrections(&session.core().state_bytes(), &[])
            .unwrap();
        let mut future = expected;
        future.fixed_update(SimulationInput {
            move_x: -1,
            move_y: 0,
            boost: false,
        });
        session
            .advance(SimulationInput {
                move_x: -1,
                move_y: 0,
                boost: false,
            })
            .unwrap();
        assert_eq!(session.core().state_bytes(), future.state_bytes());
    }

    #[test]
    fn invalid_or_duplicate_input_corrections_are_transactional() {
        let mut session = RollbackSession::new(EngineCore::new(), 8).unwrap();
        for input in sample_inputs(4) {
            session.advance(input).unwrap();
        }
        let authoritative = {
            let mut core = EngineCore::new();
            core.fixed_update(SimulationInput::default());
            core.state_bytes()
        };
        let before_state = session.core().state_bytes();

        assert_eq!(
            session.reconcile_with_corrections(
                &authoritative,
                &[
                    (2, SimulationInput::default()),
                    (2, SimulationInput::default()),
                ],
            ),
            Err(RollbackError::DuplicateCorrectionTick(2))
        );
        assert_eq!(session.core().state_bytes(), before_state);

        assert!(matches!(
            session.reconcile_with_corrections(
                &authoritative,
                &[(
                    1,
                    SimulationInput {
                        move_x: 1,
                        move_y: 0,
                        boost: false,
                    },
                )],
            ),
            Err(RollbackError::CorrectionTickOutOfRange { tick: 1, .. })
        ));
        assert_eq!(session.core().state_bytes(), before_state);
    }
}
