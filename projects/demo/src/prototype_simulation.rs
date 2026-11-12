//! Fixed-tick gameplay simulation with no platform, wall-clock, or renderer dependency.

use fate_sim::ecs::{self, Entity, Position, Rotation, Spin, Velocity, World};
use fate_sim::math::{Fixed, FixedVec2};
use fate_sim::rng::DeterministicRng;
use fate_sim::{StateHasher, StateWriter};
use std::fmt;

pub const SIMULATION_HZ: u32 = 120;
const STATE_FORMAT_VERSION: u16 = 5;
const FIXED_DELTA: Fixed = Fixed::from_ratio(1, SIMULATION_HZ as i64);
const PLAYFIELD_HALF: FixedVec2 = FixedVec2::new(Fixed::from_int(9), Fixed::from_ratio(17, 2));

pub struct EngineCore {
    pub world: World,
    pub player: ecs::Entity,
    tick: u64,
    rng: DeterministicRng,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimulationInput {
    pub move_x: i8,
    pub move_y: i8,
    pub boost: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateDecodeError {
    BadMagic,
    Truncated,
    UnsupportedVersion(u16),
    InvalidPlayer,
    InvalidWorldState(&'static str),
    TrailingBytes,
}

impl fmt::Display for StateDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => formatter.write_str("invalid simulation snapshot signature"),
            Self::Truncated => formatter.write_str("truncated simulation snapshot"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported simulation snapshot version {version}"
                )
            }
            Self::InvalidPlayer => formatter.write_str("invalid player entity in snapshot"),
            Self::InvalidWorldState(reason) => {
                write!(formatter, "invalid world state in snapshot: {reason}")
            }
            Self::TrailingBytes => formatter.write_str("unexpected trailing bytes in snapshot"),
        }
    }
}

impl std::error::Error for StateDecodeError {}

impl Default for EngineCore {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
pub type SimulationKernel = EngineCore;

#[allow(dead_code)]
impl EngineCore {
    pub fn new() -> Self {
        Self::with_seed(0x4641_5445_5349_4d31)
    }

    pub fn with_seed(seed: u64) -> Self {
        let mut world = World::new();
        let player = world.spawn();
        world
            .set_position(player, Position(FixedVec2::ZERO))
            .unwrap();
        world
            .set_velocity(
                player,
                Velocity(FixedVec2::new(Fixed::from_int(4), Fixed::from_ratio(5, 2))),
            )
            .unwrap();
        world.set_rotation(player, Rotation(Fixed::ZERO)).unwrap();
        world
            .set_spin(player, Spin(Fixed::from_ratio(9, 10)))
            .unwrap();

        Self {
            world,
            player,
            tick: 0,
            rng: DeterministicRng::new(seed),
        }
    }

    pub fn tick(&self) -> u64 {
        self.tick
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
        writer.write_bytes(b"FATE");
        writer.write_u16(STATE_FORMAT_VERSION);
        writer.write_u64(self.tick);
        writer.write_u64(self.player.to_bits());
        writer.write_u64(self.rng.state());
        self.world.write_state(writer);
    }

    /// Versioned canonical snapshot; all integers use little-endian encoding.
    pub fn state_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        self.write_state(&mut bytes);
        bytes
    }

    pub fn from_state_bytes(bytes: &[u8]) -> Result<Self, StateDecodeError> {
        let mut offset = 0;
        if Self::read_bytes::<4>(bytes, &mut offset)? != *b"FATE" {
            return Err(StateDecodeError::BadMagic);
        }
        let version = Self::read_u16(bytes, &mut offset)?;
        if version != STATE_FORMAT_VERSION {
            return Err(StateDecodeError::UnsupportedVersion(version));
        }

        let tick = Self::read_u64(bytes, &mut offset)?;
        let player = Entity::from_bits(Self::read_u64(bytes, &mut offset)?);
        let rng = DeterministicRng::from_state(Self::read_u64(bytes, &mut offset)?);

        let world =
            World::decode_state(bytes, &mut offset).map_err(StateDecodeError::InvalidWorldState)?;
        if offset != bytes.len() {
            return Err(StateDecodeError::TrailingBytes);
        }
        if !world.is_alive(player) {
            return Err(StateDecodeError::InvalidPlayer);
        }

        Ok(Self {
            world,
            player,
            tick,
            rng,
        })
    }

    /// FNV-1a 64-bit diagnostic hash via zero-allocation streaming.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = StateHasher::new();
        self.write_state(&mut hasher);
        hasher.finish()
    }

    fn read_bytes<const N: usize>(
        bytes: &[u8],
        offset: &mut usize,
    ) -> Result<[u8; N], StateDecodeError> {
        let end = offset.checked_add(N).ok_or(StateDecodeError::Truncated)?;
        let value = bytes
            .get(*offset..end)
            .ok_or(StateDecodeError::Truncated)?
            .try_into()
            .expect("fixed-size slice");
        *offset = end;
        Ok(value)
    }

    fn read_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, StateDecodeError> {
        Ok(u16::from_le_bytes(Self::read_bytes(bytes, offset)?))
    }

    fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, StateDecodeError> {
        Ok(u32::from_le_bytes(Self::read_bytes(bytes, offset)?))
    }

    fn read_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, StateDecodeError> {
        Ok(u64::from_le_bytes(Self::read_bytes(bytes, offset)?))
    }

    pub fn fixed_update(&mut self, input: SimulationInput) {
        self.tick = self.tick.saturating_add(1);
        let mut direction = FixedVec2::new(
            Fixed::from_int(input.move_x as i64),
            Fixed::from_int(input.move_y as i64),
        );
        if direction.length() > Fixed::ONE {
            direction = direction.normalize();
        }
        let boost = if input.boost {
            Fixed::from_int(2)
        } else {
            Fixed::ONE
        };
        if let Some(velocity) = self.world.velocity_mut(self.player) {
            velocity.0 += direction * (Fixed::from_int(40) * boost * FIXED_DELTA);
            let max_speed = Fixed::from_int(18) * boost;
            if velocity.0.length() > max_speed {
                velocity.0 = velocity.0.normalize() * max_speed;
            }
        }

        ecs::movement_system(&mut self.world, FIXED_DELTA);
        ecs::spin_system(&mut self.world, FIXED_DELTA);
        ecs::bounds_system(&mut self.world, PLAYFIELD_HALF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulation_core_runs_fixed_ticks_without_platform_or_wall_clock() {
        let mut sim = SimulationKernel::new();
        sim.fixed_update(SimulationInput {
            move_x: 1,
            move_y: 0,
            boost: false,
        });

        assert_eq!(sim.world.entity_count(), 1);
        assert!(sim.world.positions().get(sim.player).unwrap().0.x > Fixed::ZERO);
    }

    #[test]
    fn replaying_identical_tick_inputs_produces_identical_world_state() {
        let mut first = EngineCore::new();
        let mut replay = EngineCore::new();

        for tick in 0..600 {
            let input = match (tick / 120) % 4 {
                0 => SimulationInput {
                    move_x: 1,
                    move_y: 0,
                    boost: false,
                },
                1 => SimulationInput {
                    move_x: 0,
                    move_y: 1,
                    boost: true,
                },
                2 => SimulationInput {
                    move_x: -1,
                    move_y: -1,
                    boost: false,
                },
                _ => SimulationInput::default(),
            };
            first.fixed_update(input);
            replay.fixed_update(input);
            assert_eq!(first.state_hash(), replay.state_hash(), "tick {tick}");
        }

        assert_eq!(first.state_bytes(), replay.state_bytes());
    }

    #[test]
    fn replay_has_stable_fixed_point_golden_state() {
        let mut sim = EngineCore::new();
        for tick in 0..600 {
            let input = match (tick / 120) % 4 {
                0 => SimulationInput {
                    move_x: 1,
                    move_y: 0,
                    boost: false,
                },
                1 => SimulationInput {
                    move_x: 0,
                    move_y: 1,
                    boost: true,
                },
                2 => SimulationInput {
                    move_x: -1,
                    move_y: -1,
                    boost: false,
                },
                _ => SimulationInput::default(),
            };
            sim.fixed_update(input);
        }

        let position = sim.world.positions().get(sim.player).unwrap().0;
        let velocity = sim.world.velocities().get(sim.player).unwrap().0;
        let rotation = sim.world.rotations().get(sim.player).unwrap().0;
        assert_eq!(
            (
                position.x.raw(),
                position.y.raw(),
                velocity.x.raw(),
                velocity.y.raw(),
                rotation.raw(),
            ),
            (
                34_778_264_232,
                -14_421_543_408,
                57_766_190_812,
                33_547_866_426,
                19_327_352_400,
            )
        );
        assert_eq!(sim.state_hash(), 0x4de0_f2f5_b340_fc18);
    }

    #[test]
    fn canonical_snapshot_is_versioned_and_hash_tracks_gameplay_state() {
        let mut first = EngineCore::new();
        let second = EngineCore::new();

        assert_eq!(first.tick(), 0);
        assert_eq!(&first.state_bytes()[..6], b"FATE\x05\x00");
        assert_eq!(first.state_hash(), second.state_hash());

        first.fixed_update(SimulationInput::default());
        assert_ne!(first.state_hash(), second.state_hash());
        assert_eq!(first.tick(), 1);
        assert_eq!(first.state_bytes(), first.state_bytes());

        let mut changed_component = EngineCore::new();
        changed_component
            .world
            .set_velocity(
                changed_component.player,
                Velocity(FixedVec2::new(Fixed::from_int(5), Fixed::from_ratio(5, 2))),
            )
            .unwrap();
        assert_ne!(changed_component.state_hash(), second.state_hash());
    }

    #[test]
    fn snapshot_round_trip_preserves_world_and_tick() {
        let mut original = EngineCore::new();
        let extra_entity = original.world.spawn();
        original
            .world
            .set_position(
                extra_entity,
                Position(FixedVec2::new(Fixed::from_int(3), Fixed::from_int(2))),
            )
            .unwrap();
        for _ in 0..240 {
            original.fixed_update(SimulationInput {
                move_x: 1,
                move_y: -1,
                boost: true,
            });
        }

        let restored = EngineCore::from_state_bytes(&original.state_bytes()).unwrap();
        assert_eq!(restored.state_bytes(), original.state_bytes());
        assert_eq!(restored.tick(), 240);

        let mut restored = restored;
        for tick in 0..120 {
            let input = SimulationInput {
                move_x: if tick % 2 == 0 { 1 } else { -1 },
                move_y: if tick % 3 == 0 { 1 } else { 0 },
                boost: tick % 4 == 0,
            };
            original.fixed_update(input);
            restored.fixed_update(input);
            assert_eq!(restored.state_hash(), original.state_hash(), "tick {tick}");
        }

        assert_eq!(restored.state_bytes(), original.state_bytes());
    }

    #[test]
    fn snapshot_restore_continues_the_seeded_random_stream() {
        let mut original = EngineCore::with_seed(0xfeed_beef);
        for _ in 0..11 {
            original.random_u64();
        }
        let snapshot = original.state_bytes();
        let mut restored = EngineCore::from_state_bytes(&snapshot).unwrap();
        assert_eq!(restored.state_bytes(), snapshot);

        for _ in 0..100 {
            assert_eq!(original.random_u64(), restored.random_u64());
        }

        assert_eq!(original.random_range_u32(17), restored.random_range_u32(17));
    }

    #[test]
    fn equal_seeds_generate_equal_streams_and_different_seeds_do_not() {
        let mut first = EngineCore::with_seed(1);
        let mut replay = EngineCore::with_seed(1);
        let mut different = EngineCore::with_seed(2);

        for _ in 0..32 {
            assert_eq!(first.random_u64(), replay.random_u64());
        }

        let mut first = EngineCore::with_seed(1);
        assert_ne!(first.random_u64(), different.random_u64());
    }

    #[test]
    fn core_rng_helpers_expose_seeded_signed_ranges_and_shuffle() {
        let mut first = EngineCore::with_seed(99);
        let mut second = EngineCore::with_seed(99);

        for _ in 0..100 {
            assert_eq!(
                first.random_range_i32(-100, 100),
                second.random_range_i32(-100, 100)
            );
        }

        let mut first_values = [1, 2, 3, 4, 5, 6];
        let mut second_values = first_values;
        first.shuffle(&mut first_values);
        second.shuffle(&mut second_values);
        assert_eq!(first_values, second_values);

        assert_eq!(
            first.random_range_fixed(Fixed::from_int(-5), Fixed::from_int(5)),
            second.random_range_fixed(Fixed::from_int(-5), Fixed::from_int(5))
        );
    }

    #[test]
    fn snapshot_decoder_rejects_malformed_and_trailing_data() {
        let snapshot = EngineCore::new().state_bytes();
        for length in 0..snapshot.len() {
            assert!(EngineCore::from_state_bytes(&snapshot[..length]).is_err());
        }

        let mut bad_magic = snapshot.clone();
        bad_magic[0] = b'X';
        assert!(matches!(
            EngineCore::from_state_bytes(&bad_magic),
            Err(StateDecodeError::BadMagic)
        ));

        let mut bad_version = snapshot.clone();
        bad_version[4..6].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(matches!(
            EngineCore::from_state_bytes(&bad_version),
            Err(StateDecodeError::UnsupportedVersion(u16::MAX))
        ));

        let mut trailing = snapshot;
        trailing.push(0);
        assert!(matches!(
            EngineCore::from_state_bytes(&trailing),
            Err(StateDecodeError::TrailingBytes)
        ));
    }

    #[test]
    fn movement_input_is_normalized_and_boost_is_explicit() {
        let mut normal = EngineCore::new();
        let mut boosted = EngineCore::new();
        let diagonal = SimulationInput {
            move_x: 1,
            move_y: 1,
            boost: false,
        };
        let boosted_diagonal = SimulationInput {
            boost: true,
            ..diagonal
        };

        normal.fixed_update(diagonal);
        boosted.fixed_update(boosted_diagonal);

        let normal_velocity = normal.world.velocities().get(normal.player).unwrap().0;
        let boosted_velocity = boosted.world.velocities().get(boosted.player).unwrap().0;
        assert!(normal_velocity.length() <= Fixed::from_int(18));
        assert!(boosted_velocity.length() <= Fixed::from_int(36));
    }
}
