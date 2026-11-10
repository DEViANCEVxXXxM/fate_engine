use crate::ecs::{self, Position, Rotation, Spin, Velocity, World};
use crate::math::{Fixed, FixedVec2};

pub const SIMULATION_HZ: u32 = 120;
const FIXED_DELTA: Fixed = Fixed::from_ratio(1, SIMULATION_HZ as i64);
const PLAYFIELD_HALF: FixedVec2 = FixedVec2::new(Fixed::from_int(9), Fixed::from_ratio(17, 2));

pub struct EngineCore {
    pub world: World,
    pub player: ecs::Entity,
    demo_entities: Vec<ecs::Entity>,
    demo_stress: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimulationInput {
    pub move_x: i8,
    pub move_y: i8,
    pub boost: bool,
}

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
        }

        assert_eq!(
            first.world.positions.get(first.player),
            replay.world.positions.get(replay.player)
        );
        assert_eq!(
            first.world.velocities.get(first.player),
            replay.world.velocities.get(replay.player)
        );
        assert_eq!(
            first.world.rotations.get(first.player),
            replay.world.rotations.get(replay.player)
        );
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
