use crate::ecs::World;
use crate::math::Fixed;
use crate::rng::DeterministicRng;

const STATE_FORMAT_VERSION: u16 = 2;

/// Sink trait for writing canonical Little-Endian simulation state bytes.
pub trait StateWriter {
    fn write_byte(&mut self, byte: u8);
    fn write_bytes(&mut self, bytes: &[u8]);

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.write_byte(value);
    }

    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.write_bytes(&value.to_le_bytes());
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.write_bytes(&value.to_le_bytes());
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    #[inline]
    fn write_i64(&mut self, value: i64) {
        self.write_bytes(&value.to_le_bytes());
    }
}

impl StateWriter for Vec<u8> {
    #[inline]
    fn write_byte(&mut self, byte: u8) {
        self.push(byte);
    }

    #[inline]
    fn write_bytes(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}

/// Zero-allocation FNV-1a 64-bit streaming diagnostic state hasher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateHasher {
    hash: u64,
}

impl StateHasher {
    pub const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    pub const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub const fn new() -> Self {
        Self {
            hash: Self::OFFSET_BASIS,
        }
    }

    #[inline]
    pub fn finish(self) -> u64 {
        self.hash
    }
}

impl Default for StateHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StateWriter for StateHasher {
    #[inline]
    fn write_byte(&mut self, byte: u8) {
        self.hash = (self.hash ^ (byte as u64)).wrapping_mul(Self::PRIME);
    }

    #[inline]
    fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_byte(byte);
        }
    }
}

/// Reusable deterministic state container without any built-in game rules.
pub struct SimulationState {
    pub world: World,
    tick: u64,
    rng: DeterministicRng,
}

/// Read-only snapshot of simulation state for tooling and diagnostics.
///
/// `state_hash` is the same diagnostic checksum as
/// [`SimulationState::state_hash`]; it is not authentication.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimulationDiagnostics {
    pub tick: u64,
    pub world: crate::ecs::WorldDiagnostics,
    pub state_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimulationStateError {
    BadMagic,
    Truncated,
    UnsupportedVersion(u16),
    TickOverflow,
    InvalidWorldState(&'static str),
    TrailingBytes,
}

impl std::fmt::Display for SimulationStateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => formatter.write_str("invalid simulation state signature"),
            Self::Truncated => formatter.write_str("truncated simulation state"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported simulation state version {version}")
            }
            Self::TickOverflow => formatter.write_str("simulation tick counter is exhausted"),
            Self::InvalidWorldState(reason) => {
                write!(formatter, "invalid world state: {reason}")
            }
            Self::TrailingBytes => formatter.write_str("unexpected trailing state bytes"),
        }
    }
}

impl std::error::Error for SimulationStateError {}

impl Default for SimulationState {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulationState {
    pub fn new() -> Self {
        Self::with_seed(0x4641_5445_5349_4d31)
    }

    pub fn with_seed(seed: u64) -> Self {
        Self {
            world: World::new(),
            tick: 0,
            rng: DeterministicRng::new(seed),
        }
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn advance_tick(&mut self) -> Result<(), SimulationStateError> {
        self.tick = self
            .tick
            .checked_add(1)
            .ok_or(SimulationStateError::TickOverflow)?;
        Ok(())
    }

    pub fn random_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    pub fn random_range_u32(&mut self, upper_exclusive: u32) -> u32 {
        self.rng.range_u32(upper_exclusive)
    }

    pub fn random_range_i32(&mut self, min_inclusive: i32, max_exclusive: i32) -> i32 {
        self.rng.range_i32(min_inclusive, max_exclusive)
    }

    pub fn random_range_fixed(&mut self, min_inclusive: Fixed, max_exclusive: Fixed) -> Fixed {
        self.rng.range_fixed(min_inclusive, max_exclusive)
    }

    pub fn shuffle<T>(&mut self, values: &mut [T]) {
        self.rng.shuffle(values);
    }

    /// Stream canonical Little-Endian state bytes into any `StateWriter` sink without heap buffering.
    pub fn write_state<W: StateWriter>(&self, writer: &mut W) {
        writer.write_bytes(b"FSTS");
        writer.write_u16(STATE_FORMAT_VERSION);
        writer.write_u64(self.tick);
        writer.write_u64(self.rng.state());
        self.world.write_state(writer);
    }

    /// Serialize state into an owned vector.
    pub fn state_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        self.write_state(&mut bytes);
        bytes
    }

    pub fn from_state_bytes(bytes: &[u8]) -> Result<Self, SimulationStateError> {
        let mut offset = 0;
        if Self::read_bytes::<4>(bytes, &mut offset)? != *b"FSTS" {
            return Err(SimulationStateError::BadMagic);
        }
        let version = u16::from_le_bytes(Self::read_bytes(bytes, &mut offset)?);
        if version != STATE_FORMAT_VERSION {
            return Err(SimulationStateError::UnsupportedVersion(version));
        }
        let tick = u64::from_le_bytes(Self::read_bytes(bytes, &mut offset)?);
        let rng =
            DeterministicRng::from_state(u64::from_le_bytes(Self::read_bytes(bytes, &mut offset)?));
        let world = World::decode_state(bytes, &mut offset)
            .map_err(SimulationStateError::InvalidWorldState)?;
        if offset != bytes.len() {
            return Err(SimulationStateError::TrailingBytes);
        }
        Ok(Self { world, tick, rng })
    }

    /// Calculate FNV-1a 64-bit diagnostic hash via zero-allocation streaming.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = StateHasher::new();
        self.write_state(&mut hasher);
        hasher.finish()
    }

    /// Capture a read-only snapshot of simulation and world state.
    ///
    /// The hash is the same diagnostic checksum as [`Self::state_hash`]; it
    /// detects deterministic divergence but does not authenticate data.
    pub fn diagnostics(&self) -> SimulationDiagnostics {
        SimulationDiagnostics {
            tick: self.tick,
            world: self.world.diagnostics(),
            state_hash: self.state_hash(),
        }
    }

    fn read_bytes<const N: usize>(
        bytes: &[u8],
        offset: &mut usize,
    ) -> Result<[u8; N], SimulationStateError> {
        let end = offset
            .checked_add(N)
            .ok_or(SimulationStateError::Truncated)?;
        let value = bytes
            .get(*offset..end)
            .ok_or(SimulationStateError::Truncated)?
            .try_into()
            .expect("fixed-size slice");
        *offset = end;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::Position;
    use crate::math::FixedVec2;

    #[test]
    fn streaming_hash_matches_byte_buffer_fold() {
        let mut state = SimulationState::with_seed(42);
        let e = state.world.spawn();
        state
            .world
            .set_position(
                e,
                Position(FixedVec2::new(Fixed::from_int(7), Fixed::from_int(-9))),
            )
            .unwrap();
        state.advance_tick().unwrap();

        let streamed_hash = state.state_hash();
        let bytes = state.state_bytes();
        let manual_hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ (b as u64)).wrapping_mul(0x0000_0100_0000_01b3)
        });

        assert_eq!(streamed_hash, manual_hash);
    }

    #[test]
    fn diagnostics_report_tick_world_and_state_hash() {
        let mut state = SimulationState::with_seed(7);
        let entity = state.world.spawn();
        state
            .world
            .set_position(entity, Position(FixedVec2::ZERO))
            .unwrap();
        state.advance_tick().unwrap();

        let diagnostics = state.diagnostics();
        assert_eq!(diagnostics.tick, 1);
        assert_eq!(diagnostics.state_hash, state.state_hash());
        assert_eq!(diagnostics.world.live_entities, 1);
        assert_eq!(diagnostics.world.positions, 1);
        assert_eq!(diagnostics.world.velocities, 0);
    }

    #[test]
    fn state_round_trip_preserves_world_tick_and_future_random_stream() {
        let mut state = SimulationState::with_seed(1234);
        let entity = state.world.spawn();
        state
            .world
            .set_position(
                entity,
                Position(FixedVec2::new(Fixed::from_int(2), Fixed::from_int(-3))),
            )
            .unwrap();
        state.advance_tick().unwrap();
        let snapshot = state.state_bytes();
        let mut restored = SimulationState::from_state_bytes(&snapshot).unwrap();

        assert_eq!(restored.state_bytes(), snapshot);
        assert_eq!(restored.tick(), 1);
        assert_eq!(state.random_u64(), restored.random_u64());
        assert_eq!(state.state_hash(), restored.state_hash());
    }

    #[test]
    fn decoder_rejects_bad_versions_and_trailing_data() {
        let snapshot = SimulationState::new().state_bytes();
        let mut bad_version = snapshot.clone();
        bad_version[4..6].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(matches!(
            SimulationState::from_state_bytes(&bad_version),
            Err(SimulationStateError::UnsupportedVersion(u16::MAX))
        ));

        let mut trailing = snapshot;
        trailing.push(0);
        assert!(matches!(
            SimulationState::from_state_bytes(&trailing),
            Err(SimulationStateError::TrailingBytes)
        ));
    }
}
