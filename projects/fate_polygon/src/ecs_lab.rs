//! Stress the real ECS and snapshot APIs with a bounded deterministic workload.

use std::time::{Duration, Instant};

use fate_sim::ecs::{
    Entity, Position, Rotation, Spin, Velocity, bounds_system, movement_system, spin_system,
};
use fate_sim::math::{Fixed as F, FixedVec2 as V};
use fate_sim::{SimulationState, StateHasher, StateWriter};

use crate::settings::Settings;

#[derive(Default)]
pub struct EcsWork {
    pub spawned: u64,
    pub despawned: u64,
    pub updates: u64,
    pub snapshots: u64,
    pub bytes: u64,
    pub update_time: Duration,
    pub snapshot_time: Duration,
}

pub struct EcsLab {
    state: SimulationState,
    live: Vec<Entity>,
    encoded: Vec<u8>,
    round_trip: Vec<u8>,
}

impl EcsLab {
    pub fn new(seed: u32) -> Self {
        Self {
            state: SimulationState::with_seed(u64::from(seed) ^ 0xec5a_5310),
            live: Vec::new(),
            encoded: Vec::new(),
            round_trip: Vec::new(),
        }
    }

    pub fn resize(&mut self, count: usize) {
        while self.live.len() > count {
            let entity = self.live.pop().expect("extra entity");
            assert!(self.state.world.despawn(entity));
        }
        while self.live.len() < count {
            let entity = spawn(&mut self.state);
            self.live.push(entity);
        }
    }

    pub fn step(&mut self, settings: Settings, dt: F, work: &mut EcsWork) {
        let start = Instant::now();
        let count = self.live.len();
        let churn = (settings.churn as usize).min(count);
        if churn != 0 {
            let first = (self.state.tick() % count as u64) as usize * churn % count;
            for offset in 0..churn {
                let slot = (first + offset) % count;
                assert!(self.state.world.despawn(self.live[slot]));
                self.live[slot] = spawn(&mut self.state);
            }
            work.spawned += churn as u64;
            work.despawned += churn as u64;
        }
        movement_system(&mut self.state.world, dt);
        spin_system(&mut self.state.world, dt);
        bounds_system(
            &mut self.state.world,
            V::new(F::from_int(18), F::from_int(10)),
        );
        work.updates += count as u64;
        self.state.advance_tick().expect("ECS tick overflow");
        work.update_time += start.elapsed();

        let cadence = u64::from(settings.snapshot_every);
        if cadence != 0 && self.state.tick().is_multiple_of(cadence) {
            let start = Instant::now();
            self.encoded.clear();
            self.state.write_state(&mut self.encoded);
            let restored = SimulationState::from_state_bytes(&self.encoded)
                .expect("engine must decode its own ECS snapshot");
            self.round_trip.clear();
            restored.write_state(&mut self.round_trip);
            assert!(
                self.round_trip == self.encoded,
                "ECS snapshot round trip changed state"
            );
            self.state = restored;
            work.snapshots += 1;
            work.bytes += self.encoded.len() as u64;
            work.snapshot_time += start.elapsed();
        }
    }

    pub fn state_hash(&self) -> u64 {
        let mut hash = StateHasher::new();
        self.state.write_state(&mut hash);
        hash.write_u64(self.live.len() as u64);
        for &entity in &self.live {
            hash.write_u64(entity.to_bits());
        }
        hash.finish()
    }

    pub fn live_and_capacity(&self) -> (usize, usize) {
        (self.live.len(), self.state.world.capacity())
    }

    pub fn positions(&self) -> impl Iterator<Item = V> + '_ {
        self.state
            .world
            .positions()
            .iter()
            .map(|(_, position)| position.0)
    }
}

fn spawn(state: &mut SimulationState) -> Entity {
    let mut axis = |max: i32| F::from_ratio(i64::from(state.random_range_i32(-max, max + 1)), 100);
    let position = Position(V::new(axis(1750), axis(950)));
    let velocity = Velocity(V::new(axis(600), axis(600)));
    let rotation = Rotation(axis(314));
    let spin = Spin(axis(450));
    let world = &mut state.world;
    let entity = world.spawn();
    world.set_position(entity, position).expect("live entity");
    world.set_velocity(entity, velocity).expect("live entity");
    world.set_rotation(entity, rotation).expect("live entity");
    world.set_spin(entity, spin).expect("live entity");
    entity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn churn_reuses_slots_and_snapshot_restore_does_not_change_state() {
        let s = Settings {
            entities: 80,
            churn: 11,
            snapshot_every: 3,
            ..Settings::default()
        };
        let mut a = EcsLab::new(s.seed);
        let mut b = EcsLab::new(s.seed);
        a.resize(s.entities as usize);
        b.resize(s.entities as usize);
        let original = a.live[0];
        let mut work = EcsWork::default();
        for _ in 0..12 {
            a.step(s, F::from_ratio(1, 120), &mut work);
            b.step(
                Settings {
                    snapshot_every: 0,
                    ..s
                },
                F::from_ratio(1, 120),
                &mut EcsWork::default(),
            );
            assert_eq!(a.state_hash(), b.state_hash());
        }
        assert!(!a.state.world.is_alive(original));
        assert_eq!(a.live_and_capacity(), (80, 80));
        assert_eq!(a.state.world.diagnostics().positions, 80);
        assert_eq!(
            (work.spawned, work.despawned, work.updates, work.snapshots),
            (132, 132, 960, 4)
        );
        assert!(work.bytes > 0);
    }

    #[test]
    fn churn_is_bounded_by_live_count_after_resize() {
        let mut lab = EcsLab::new(71);
        lab.resize(20);
        lab.resize(3);
        let s = Settings {
            churn: 1024,
            snapshot_every: 1,
            ..Settings::default()
        };
        let mut work = EcsWork::default();
        lab.step(s, F::ONE, &mut work);
        assert_eq!((work.spawned, work.despawned, work.updates), (3, 3, 3));
        assert_eq!(lab.live_and_capacity(), (3, 20));
        lab.resize(0);
        lab.step(s, F::ONE, &mut work);
        assert_eq!(work.spawned, 3);
        assert_eq!(work.snapshots, 2);
    }
}
