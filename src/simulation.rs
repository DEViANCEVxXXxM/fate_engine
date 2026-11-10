use crate::ecs::{self, Position, Rotation, Spin, Velocity, World};
use crate::math::{Fixed, FixedVec2};
use std::collections::HashSet;
use std::fmt;

pub const SIMULATION_HZ: u32 = 120;
const STATE_FORMAT_VERSION: u16 = 2;
const MAX_SNAPSHOT_ENTITIES: u32 = 1_000_000;
const FIXED_DELTA: Fixed = Fixed::from_ratio(1, SIMULATION_HZ as i64);
const PLAYFIELD_HALF: FixedVec2 = FixedVec2::new(Fixed::from_int(9), Fixed::from_ratio(17, 2));

pub struct EngineCore {
    pub world: World,
    pub player: ecs::Entity,
    tick: u64,
    demo_entities: Vec<ecs::Entity>,
    demo_stress: bool,
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
    InvalidBoolean,
    InvalidPlayer,
    InvalidDemoEntities,
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
            Self::InvalidBoolean => formatter.write_str("invalid boolean in simulation snapshot"),
            Self::InvalidPlayer => formatter.write_str("invalid player entity in snapshot"),
            Self::InvalidDemoEntities => {
                formatter.write_str("invalid demo entity list in simulation snapshot")
            }
            Self::InvalidWorldState(reason) => {
                write!(formatter, "invalid world state in snapshot: {reason}")
            }
            Self::TrailingBytes => formatter.write_str("unexpected trailing bytes in snapshot"),
        }
    }
}

impl std::error::Error for StateDecodeError {}

#[allow(dead_code)]
pub type SimulationKernel = EngineCore;

#[allow(dead_code)]
impl EngineCore {
    pub fn new() -> Self {
        let mut world = World::new();
        let player = world.spawn();
        world.positions.insert(player, Position(FixedVec2::ZERO));
        world.velocities.insert(
            player,
            Velocity(FixedVec2::new(Fixed::from_int(4), Fixed::from_ratio(5, 2))),
        );
        world.rotations.insert(player, Rotation(Fixed::ZERO));
        world.spins.insert(player, Spin(Fixed::from_ratio(9, 10)));

        Self {
            world,
            player,
            tick: 0,
            demo_entities: Vec::new(),
            demo_stress: false,
        }
    }

    pub fn new_demo() -> Self {
        let mut core = Self::new();
        core.set_demo_stress(false);
        core
    }

    pub fn demo_stress_enabled(&self) -> bool {
        self.demo_stress
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Versioned canonical snapshot; all integers use little-endian encoding.
    pub fn state_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FATE");
        bytes.extend_from_slice(&STATE_FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.tick.to_le_bytes());
        bytes.extend_from_slice(&self.player.to_le_bytes());
        bytes.push(self.demo_stress as u8);
        bytes.extend_from_slice(&(self.demo_entities.len() as u32).to_le_bytes());
        for entity in &self.demo_entities {
            bytes.extend_from_slice(&entity.to_le_bytes());
        }
        self.world.encode_state(&mut bytes);
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
        let player = Self::read_u32(bytes, &mut offset)?;
        let demo_stress = match Self::read_u8(bytes, &mut offset)? {
            0 => false,
            1 => true,
            _ => return Err(StateDecodeError::InvalidBoolean),
        };
        let demo_count = Self::read_u32(bytes, &mut offset)?;
        if demo_count > MAX_SNAPSHOT_ENTITIES || demo_count as usize > bytes.len() / 4 {
            return Err(StateDecodeError::InvalidDemoEntities);
        }
        let mut demo_entities = Vec::with_capacity(demo_count as usize);
        for _ in 0..demo_count {
            demo_entities.push(Self::read_u32(bytes, &mut offset)?);
        }

        let world =
            World::decode_state(bytes, &mut offset).map_err(StateDecodeError::InvalidWorldState)?;
        if offset != bytes.len() {
            return Err(StateDecodeError::TrailingBytes);
        }
        if !world.is_alive(player) {
            return Err(StateDecodeError::InvalidPlayer);
        }

        let mut unique_demo_entities = HashSet::with_capacity(demo_entities.len());
        if demo_entities.iter().any(|entity| {
            *entity == player
                || !world.was_allocated(*entity)
                || !unique_demo_entities.insert(*entity)
        }) {
            return Err(StateDecodeError::InvalidDemoEntities);
        }

        Ok(Self {
            world,
            player,
            tick,
            demo_entities,
            demo_stress,
        })
    }

    /// FNV-1a 64-bit diagnostic hash over `state_bytes`; not a security primitive.
    pub fn state_hash(&self) -> u64 {
        self.state_bytes()
            .into_iter()
            .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
                (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3)
            })
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

    fn read_u8(bytes: &[u8], offset: &mut usize) -> Result<u8, StateDecodeError> {
        Ok(Self::read_bytes::<1>(bytes, offset)?[0])
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

    pub fn set_demo_stress(&mut self, enabled: bool) {
        if self.demo_stress == enabled && !self.demo_entities.is_empty() {
            return;
        }

        for entity in self.demo_entities.drain(..) {
            self.world.despawn(entity);
        }

        let visible_total = if enabled { 192 } else { 32 };
        for index in 0..visible_total - 1 {
            let column = index % 16;
            let row = index / 16;
            let angle = Fixed::from_ratio((index as i64 * 618_034) % 6_283_185, 1_000_000);
            let direction = match index % 8 {
                0 => FixedVec2::new(Fixed::ONE, Fixed::ZERO),
                1 => FixedVec2::new(Fixed::ONE, Fixed::ONE).normalize(),
                2 => FixedVec2::new(Fixed::ZERO, Fixed::ONE),
                3 => FixedVec2::new(-Fixed::ONE, Fixed::ONE).normalize(),
                4 => FixedVec2::new(-Fixed::ONE, Fixed::ZERO),
                5 => FixedVec2::new(-Fixed::ONE, -Fixed::ONE).normalize(),
                6 => FixedVec2::new(Fixed::ZERO, -Fixed::ONE),
                _ => FixedVec2::new(Fixed::ONE, -Fixed::ONE).normalize(),
            };
            let entity = self.world.spawn();
            self.world.positions.insert(
                entity,
                Position(FixedVec2::new(
                    Fixed::from_ratio((column as i64 * 2 - 15) * 11, 20),
                    Fixed::from_ratio((row as i64 * 2 - 11) * 11, 20),
                )),
            );
            self.world
                .velocities
                .insert(entity, Velocity(direction * Fixed::from_int(2)));
            self.world.rotations.insert(entity, Rotation(angle));
            self.world.spins.insert(
                entity,
                Spin(if index % 2 == 0 {
                    Fixed::from_ratio(45, 100)
                } else {
                    Fixed::from_ratio(-45, 100)
                }),
            );
            self.demo_entities.push(entity);
        }

        for index in 0..8 {
            let entity = self.world.spawn();
            let offset = Fixed::from_int(index as i64 * 2);
            let x = if index % 2 == 0 {
                Fixed::from_int(24) + offset
            } else {
                Fixed::from_int(-24) - offset
            };
            self.world
                .positions
                .insert(entity, Position(FixedVec2::new(x, Fixed::ZERO)));
            self.world.rotations.insert(entity, Rotation(Fixed::ZERO));
            self.demo_entities.push(entity);
        }

        self.demo_stress = enabled;
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
        if let Some(velocity) = self.world.velocities.get_mut(self.player) {
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
        assert!(sim.world.positions.get(sim.player).unwrap().0.x > Fixed::ZERO);
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

        let position = sim.world.positions.get(sim.player).unwrap().0;
        let velocity = sim.world.velocities.get(sim.player).unwrap().0;
        let rotation = sim.world.rotations.get(sim.player).unwrap().0;
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
        assert_eq!(sim.state_hash(), 0x1147_5aac_821f_a31c);
    }

    #[test]
    fn canonical_snapshot_is_versioned_and_hash_tracks_gameplay_state() {
        let mut first = EngineCore::new();
        let second = EngineCore::new();

        assert_eq!(first.tick(), 0);
        assert_eq!(&first.state_bytes()[..6], b"FATE\x02\x00");
        assert_eq!(first.state_hash(), second.state_hash());

        first.fixed_update(SimulationInput::default());
        assert_ne!(first.state_hash(), second.state_hash());
        assert_eq!(first.tick(), 1);
        assert_eq!(first.state_bytes(), first.state_bytes());

        let mut changed_component = EngineCore::new();
        changed_component.world.velocities.insert(
            changed_component.player,
            Velocity(FixedVec2::new(Fixed::from_int(5), Fixed::from_ratio(5, 2))),
        );
        assert_ne!(changed_component.state_hash(), second.state_hash());
    }

    #[test]
    fn snapshot_round_trip_preserves_world_and_demo_lifecycle() {
        let mut original = EngineCore::new_demo();
        original.set_demo_stress(true);
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
        assert!(restored.demo_stress_enabled());

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

        original.set_demo_stress(false);
        restored.set_demo_stress(false);
        assert_eq!(restored.state_bytes(), original.state_bytes());
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

        let mut bad_boolean = snapshot.clone();
        bad_boolean[18] = 2;
        assert!(matches!(
            EngineCore::from_state_bytes(&bad_boolean),
            Err(StateDecodeError::InvalidBoolean)
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

        let normal_velocity = normal.world.velocities.get(normal.player).unwrap().0;
        let boosted_velocity = boosted.world.velocities.get(boosted.player).unwrap().0;
        assert!(normal_velocity.length() <= Fixed::from_int(18));
        assert!(boosted_velocity.length() <= Fixed::from_int(36));
    }
}
