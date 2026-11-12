use std::ops::{Deref, DerefMut};

use crate::prototype_simulation::EngineCore;
use fate_sim::ecs::{Entity, Position, Rotation, Spin, Velocity};
use fate_sim::math::{Fixed, FixedVec2};

const DEFAULT_SEED: u64 = 0x4641_5445_5349_4d31;

pub struct DemoSimulation {
    core: EngineCore,
    demo_entities: Vec<Entity>,
    stress_enabled: bool,
}

impl DemoSimulation {
    pub fn new() -> Self {
        Self::with_seed(DEFAULT_SEED)
    }

    pub fn with_seed(seed: u64) -> Self {
        let mut simulation = Self {
            core: EngineCore::with_seed(seed),
            demo_entities: Vec::new(),
            stress_enabled: false,
        };
        simulation.set_demo_stress(false);
        simulation
    }

    pub fn demo_stress_enabled(&self) -> bool {
        self.stress_enabled
    }

    pub fn set_demo_stress(&mut self, enabled: bool) {
        if self.stress_enabled == enabled && !self.demo_entities.is_empty() {
            return;
        }

        for entity in self.demo_entities.drain(..) {
            self.core.world.despawn(entity);
        }

        let visible_total = if enabled { 192 } else { 32 };
        for index in 0..visible_total - 1 {
            let column = index % 16;
            let row = index / 16;
            let angle = Fixed::from_ratio(self.core.random_range_u32(6_283_185) as i64, 1_000_000);
            let direction = match self.core.random_range_u32(8) {
                0 => FixedVec2::new(Fixed::ONE, Fixed::ZERO),
                1 => FixedVec2::new(Fixed::ONE, Fixed::ONE).normalize(),
                2 => FixedVec2::new(Fixed::ZERO, Fixed::ONE),
                3 => FixedVec2::new(-Fixed::ONE, Fixed::ONE).normalize(),
                4 => FixedVec2::new(-Fixed::ONE, Fixed::ZERO),
                5 => FixedVec2::new(-Fixed::ONE, -Fixed::ONE).normalize(),
                6 => FixedVec2::new(Fixed::ZERO, -Fixed::ONE),
                _ => FixedVec2::new(Fixed::ONE, -Fixed::ONE).normalize(),
            };
            let entity = self.core.world.spawn();
            self.core
                .world
                .set_position(
                    entity,
                    Position(FixedVec2::new(
                        Fixed::from_ratio((column as i64 * 2 - 15) * 11, 20),
                        Fixed::from_ratio((row as i64 * 2 - 11) * 11, 20),
                    )),
                )
                .unwrap();
            self.core
                .world
                .set_velocity(entity, Velocity(direction * Fixed::from_int(2)))
                .unwrap();
            self.core
                .world
                .set_rotation(entity, Rotation(angle))
                .unwrap();
            let spin = if self.core.random_range_u32(2) == 0 {
                Fixed::from_ratio(45, 100)
            } else {
                Fixed::from_ratio(-45, 100)
            };
            self.core.world.set_spin(entity, Spin(spin)).unwrap();
            self.demo_entities.push(entity);
        }

        for index in 0..8 {
            let entity = self.core.world.spawn();
            let offset = Fixed::from_int(index as i64 * 2);
            let x = if index % 2 == 0 {
                Fixed::from_int(24) + offset
            } else {
                Fixed::from_int(-24) - offset
            };
            self.core
                .world
                .set_position(entity, Position(FixedVec2::new(x, Fixed::ZERO)))
                .unwrap();
            self.core
                .world
                .set_rotation(entity, Rotation(Fixed::ZERO))
                .unwrap();
            self.demo_entities.push(entity);
        }

        self.stress_enabled = enabled;
    }
}

impl Deref for DemoSimulation {
    type Target = EngineCore;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl DerefMut for DemoSimulation {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl Default for DemoSimulation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_demo_worlds_are_reproducible_and_seed_sensitive() {
        let first = DemoSimulation::with_seed(0x1234);
        let replay = DemoSimulation::with_seed(0x1234);
        let different = DemoSimulation::with_seed(0x5678);

        assert_eq!(first.state_bytes(), replay.state_bytes());
        assert_ne!(first.state_hash(), different.state_hash());
        assert_eq!(first.world.entity_count(), 40);
    }

    #[test]
    fn demo_stress_toggle_replaces_entities_without_affecting_engine_core() {
        let mut first = DemoSimulation::new();
        let mut replay = DemoSimulation::new();
        assert_eq!(first.world.entity_count(), 40);

        first.set_demo_stress(true);
        replay.set_demo_stress(true);
        assert!(first.demo_stress_enabled());
        assert_eq!(first.world.entity_count(), 200);
        assert_eq!(first.state_bytes(), replay.state_bytes());

        first.set_demo_stress(false);
        replay.set_demo_stress(false);
        assert!(!first.demo_stress_enabled());
        assert_eq!(first.world.entity_count(), 40);
        assert_eq!(first.state_bytes(), replay.state_bytes());
    }
}
