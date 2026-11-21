//! App-owned deterministic workloads. These are not engine solver or AI APIs.

use std::time::{Duration, Instant};

use fate_sim::math::{Fixed as F, FixedVec2 as V};
use fate_sim::rng::DeterministicRng;
use fate_sim::{StateHasher, StateWriter};

use crate::{
    contact_grid::ContactGrid,
    ecs_lab::{EcsLab, EcsWork},
    navigation::{self, Navigation, RouteWork},
    settings::Settings,
};

const WALL_X: F = F::from_int(19);
const WALL_Y: F = F::from_int(11);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Particle {
    pub p: V,
    v: V,
}

#[derive(Default)]
pub struct Work {
    pub pairs: u64,
    pub contacts: u64,
    pub decisions: u64,
    pub neighbors: u64,
    pub route: RouteWork,
    pub physics: Duration,
    pub ai: Duration,
    pub ecs: EcsWork,
}

#[derive(Default)]
pub struct Lab {
    pub tick: u64,
    pub bodies: Vec<Particle>,
    pub agents: Vec<Particle>,
    desired: Vec<V>,
    contact_grid: ContactGrid,
    navigation: Navigation,
    pub ecs: Option<EcsLab>,
}

fn spawn(seed: u32, index: usize, domain: u64) -> Particle {
    let mut rng = DeterministicRng::new(u64::from(seed) ^ domain ^ ((index as u64) << 32));
    let mut sample =
        |extent: i32| F::from_ratio(i64::from(rng.range_i32(-extent, extent + 1)), 100);
    Particle {
        p: V::new(sample(1650), sample(900)),
        v: V::new(sample(600), sample(600)),
    }
}

fn resize(items: &mut Vec<Particle>, count: usize, seed: u32, domain: u64) {
    items.truncate(count);
    items.extend((items.len()..count).map(|i| spawn(seed, i, domain)));
}

impl Lab {
    /// Diagnostic hash of future-relevant lab state, not timing or configuration.
    pub fn state_hash(&self) -> u64 {
        let mut hash = StateHasher::new();
        hash.write_u64(self.tick);
        let vector = |hash: &mut StateHasher, v: V| {
            hash.write_i64(v.x.raw());
            hash.write_i64(v.y.raw());
        };
        for particles in [&self.bodies, &self.agents] {
            hash.write_u64(particles.len() as u64);
            for item in particles {
                vector(&mut hash, item.p);
                vector(&mut hash, item.v);
            }
        }
        hash.write_u64(self.desired.len() as u64);
        for &desired in &self.desired {
            vector(&mut hash, desired);
        }
        if let Some(ecs) = &self.ecs {
            hash.write_u64(ecs.state_hash());
        }
        hash.finish()
    }

    pub fn sync_counts(&mut self, s: Settings) {
        resize(&mut self.bodies, s.bodies as usize, s.seed, 0x7a77);
        resize(&mut self.agents, s.agents as usize, s.seed, 0xa6e7);
        self.desired.resize(self.agents.len(), V::ZERO);
        if s.ecs {
            self.ecs
                .get_or_insert_with(|| EcsLab::new(s.seed))
                .resize(s.entities as usize);
        } else {
            self.ecs = None;
        }
    }

    pub fn step(&mut self, s: Settings, work: &mut Work) {
        let dt = F::from_ratio(1, i64::from(s.tick_hz));
        if s.physics {
            let start = Instant::now();
            self.physics(s, dt, work);
            work.physics += start.elapsed();
        }
        if s.ai {
            let start = Instant::now();
            if s.nav_mode == 0 {
                self.steering(s, dt, work);
            } else {
                self.navigate(s, work);
            }
            work.ai += start.elapsed();
        }
        if s.ecs {
            self.ecs
                .as_mut()
                .expect("configured ECS workload")
                .step(s, dt, &mut work.ecs);
        }
        self.tick = self.tick.wrapping_add(1);
    }

    fn physics(&mut self, s: Settings, dt: F, work: &mut Work) {
        let gravity = F::from_int(i64::from(s.gravity));
        let radius = F::from_ratio(i64::from(s.radius), 100);
        let bounce = F::from_ratio(i64::from(s.bounce), 100);
        let diameter = radius + radius;
        let contact = DiskContact {
            diameter,
            diameter2: diameter * diameter,
            bounce,
        };
        for body in &mut self.bodies {
            body.v.y -= gravity * dt;
            body.p += body.v * dt;
            walls(body, radius, bounce);
        }
        if self.bodies.len() < 2 {
            return;
        }
        let mut grid = (s.broad_phase == 1).then_some(&mut self.contact_grid);
        if let Some(grid) = &mut grid {
            grid.rebuild(self.bodies.iter().map(|body| body.p));
        }
        for _ in 0..s.passes {
            for i in 1..self.bodies.len() {
                let (left, right) = self.bodies.split_at_mut(i);
                let b = &mut right[0];
                if let Some(grid) = &mut grid {
                    let mut neighbors = grid.preceding_neighbors(i, 0);
                    while let Some(j) = neighbors.next() {
                        let a = &mut left[j];
                        if contact.resolve(a, b, work) {
                            grid.update(j, a.p);
                            if grid.update(i, b.p) {
                                // Corrections can create new candidates within
                                // this pass. Requery without revisiting pairs.
                                neighbors = grid.preceding_neighbors(i, j + 1);
                            }
                        }
                    }
                    walls(b, radius, bounce);
                    grid.update(i, b.p);
                } else {
                    for a in left {
                        contact.resolve(a, b, work);
                    }
                    walls(b, radius, bounce);
                }
            }
            if let Some(first) = self.bodies.first_mut() {
                walls(first, radius, bounce);
                if let Some(grid) = &mut grid {
                    grid.update(0, first.p);
                }
            }
        }
        // Later pairs can move earlier bodies again.
        for body in &mut self.bodies {
            walls(body, radius, bounce);
        }
    }

    fn steering(&mut self, s: Settings, dt: F, work: &mut Work) {
        let len = self.agents.len();
        let speed = F::from_int(i64::from(s.agent_speed));
        let every = u64::from(s.think_every);
        let anchor = if (self.tick / u64::from(s.tick_hz)).is_multiple_of(4) {
            V::new(F::from_int(10), F::from_int(5))
        } else {
            V::new(F::from_int(-10), F::from_int(-5))
        };
        for (i, agent) in self.agents.iter().enumerate() {
            if !(self.tick % every + i as u64 % every).is_multiple_of(every) {
                continue;
            }
            work.decisions += 1;
            let mut nearest = None;
            let samples = (s.samples as usize).min(len - 1);
            let start = (self.tick as usize).wrapping_mul(17) % len;
            for sample in 0..samples {
                let j = (start + i + sample) % (len - 1);
                let j = j + usize::from(j >= i);
                let away = agent.p - self.agents[j].p;
                let dist = away.length_squared();
                work.neighbors += 1;
                if nearest.is_none_or(|(best, _)| dist < best) {
                    nearest = Some((dist, away));
                }
            }
            let separation = nearest
                .filter(|(dist, _)| *dist < F::from_int(4))
                .map_or(V::ZERO, |(_, away)| away.normalize());
            self.desired[i] = ((anchor - agent.p).normalize() + separation).normalize() * speed;
        }
        for (agent, &desired) in self.agents.iter_mut().zip(&self.desired) {
            agent.v += (desired - agent.v) * (dt * F::from_int(6));
            agent.p += agent.v * dt;
            walls(agent, F::ZERO, F::ONE);
        }
    }

    fn navigate(&mut self, s: Settings, work: &mut Work) {
        let target = navigation::target(self.tick, s.nav_goal_ticks);
        let every = u64::from(s.think_every);
        for (i, (agent, desired)) in self.agents.iter_mut().zip(&mut self.desired).enumerate() {
            let mut cell = self.navigation.start_cell(agent.p);
            if (self.tick % every + i as u64 % every).is_multiple_of(every) {
                work.decisions += 1;
                cell = self
                    .navigation
                    .route(cell, target, s.nav_mode == 2, &mut work.route);
            }
            agent.p = navigation::position(cell);
            agent.v = V::ZERO;
            *desired = V::ZERO;
        }
    }
}

struct DiskContact {
    diameter: F,
    diameter2: F,
    bounce: F,
}

impl DiskContact {
    /// True if the pair overlapped and may have moved.
    fn resolve(&self, a: &mut Particle, b: &mut Particle, work: &mut Work) -> bool {
        work.pairs += 1;
        let delta = b.p - a.p;
        let dist2 = delta.length_squared();
        if dist2 >= self.diameter2 {
            return false;
        }
        work.contacts += 1;
        let distance = dist2.sqrt();
        let normal = if distance == F::ZERO {
            V::new(F::ONE, F::ZERO)
        } else {
            delta / distance
        };
        let push = normal * ((self.diameter - distance) * F::from_ratio(1, 2));
        a.p = a.p - push;
        b.p += push;
        let relative = b.v - a.v;
        let approach = relative.x * normal.x + relative.y * normal.y;
        if approach < F::ZERO {
            let impulse = normal * (approach * (F::ONE + self.bounce) * F::from_ratio(1, 2));
            a.v += impulse;
            b.v = b.v - impulse;
        }
        true
    }
}

fn walls(body: &mut Particle, radius: F, bounce: F) {
    for (p, v, edge) in [
        (&mut body.p.x, &mut body.v.x, WALL_X - radius),
        (&mut body.p.y, &mut body.v.y, WALL_Y - radius),
    ] {
        if *p > edge {
            *p = edge;
            if *v > F::ZERO {
                *v = -*v * bounce;
            }
        } else if *p < -edge {
            *p = -edge;
            if *v < F::ZERO {
                *v = -*v * bounce;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_navigation_preserves_agent_state_across_goal_changes() {
        let s = Settings {
            agents: 80,
            nav_goal_ticks: 16,
            think_every: 3,
            physics: false,
            ..Settings::preset(crate::settings::Preset::Navigation)
        };
        let mut individual = Lab::default();
        let mut shared = Lab::default();
        individual.sync_counts(s);
        shared.sync_counts(s);
        let mut a = Work::default();
        let mut b = Work::default();
        for _ in 0..100 {
            individual.step(Settings { nav_mode: 1, ..s }, &mut a);
            shared.step(Settings { nav_mode: 2, ..s }, &mut b);
            assert_eq!(individual.agents, shared.agents);
        }
        assert_eq!(individual.state_hash(), shared.state_hash());
        assert_eq!((a.decisions, a.route.moves), (b.decisions, b.route.moves));
        assert!(a.route.moves > 0);
        assert_eq!(b.route.searches, 7);
        assert!(b.route.visits < a.route.visits / 10);
    }

    #[test]
    fn grid_requeries_after_contact_pushes_a_body_into_a_new_neighborhood() {
        let s = Settings {
            bodies: 3,
            radius: 80,
            gravity: 0,
            ai: false,
            passes: 1,
            ..Settings::default()
        };
        let starting: Vec<_> = [190, 401, 195]
            .into_iter()
            .map(|x| Particle {
                p: V::new(F::from_ratio(x, 100), F::ZERO),
                v: V::ZERO,
            })
            .collect();
        let mut direct = Lab {
            bodies: starting.clone(),
            ..Lab::default()
        };
        let mut indexed = Lab {
            bodies: starting,
            ..Lab::default()
        };
        let mut a = Work::default();
        let mut b = Work::default();
        direct.step(s, &mut a);
        indexed.step(
            Settings {
                broad_phase: 1,
                ..s
            },
            &mut b,
        );
        assert_eq!(a.contacts, 2);
        assert_eq!(a.contacts, b.contacts);
        assert_eq!(direct.bodies, indexed.bodies);
        assert!(b.pairs < a.pairs);
    }

    #[test]
    fn grid_matches_each_seeded_solver_step_at_small_and_large_radii() {
        for (seed, radius, passes) in [(71, 5, 1), (17, 22, 2), (9, 80, 4)] {
            let settings = Settings {
                seed,
                radius,
                passes,
                bodies: 96,
                agents: 0,
                ai: false,
                ..Settings::default()
            };
            let mut direct = Lab::default();
            let mut indexed = Lab::default();
            direct.sync_counts(settings);
            indexed.sync_counts(settings);
            let mut a = Work::default();
            let mut b = Work::default();
            for tick in 0..180 {
                direct.step(settings, &mut a);
                indexed.step(
                    Settings {
                        broad_phase: 1,
                        ..settings
                    },
                    &mut b,
                );
                assert_eq!(direct.bodies, indexed.bodies, "seed {seed}, tick {tick}");
                assert_eq!(a.contacts, b.contacts, "seed {seed}, tick {tick}");
            }
            assert!(b.pairs < a.pairs / 2);
            assert_eq!(direct.state_hash(), indexed.state_hash());
        }
    }

    #[test]
    fn repeats_seeded_work_and_respects_workload_switches() {
        let s = Settings {
            bodies: 24,
            agents: 16,
            samples: 5,
            think_every: 1,
            passes: 2,
            ..Settings::default()
        };
        let run = || {
            let mut lab = Lab::default();
            lab.sync_counts(s);
            let mut work = Work::default();
            for _ in 0..20 {
                lab.step(s, &mut work);
            }
            assert_eq!(work.pairs, 20 * 2 * 24 * 23 / 2);
            assert_eq!(work.neighbors, 20 * 16 * 5);
            (lab.bodies, lab.agents)
        };
        assert_eq!(run(), run());
        let mut lab = Lab::default();
        lab.sync_counts(s);
        let initial = (lab.bodies.clone(), lab.agents.clone());
        lab.step(
            Settings {
                physics: false,
                ai: false,
                ..s
            },
            &mut Work::default(),
        );
        assert_eq!((lab.bodies, lab.agents), initial);
    }

    #[test]
    fn zero_and_singleton_workloads_are_valid() {
        let mut lab = Lab::default();
        for agents in [0, 1] {
            let s = Settings {
                bodies: 0,
                agents,
                ..Settings::default()
            };
            lab.sync_counts(s);
            let mut work = Work::default();
            lab.step(s, &mut work);
            assert_eq!((work.pairs, work.neighbors), (0, 0));
        }
    }
}
