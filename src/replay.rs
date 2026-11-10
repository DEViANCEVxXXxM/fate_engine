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
        let initial = EngineCore::new_demo();
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
}
