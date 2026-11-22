//! Scripted fixed-tick point actor against static boxes. No device initialization.

use fate_sim::math::{Fixed as F, FixedVec3 as V, segment_aabb_contact, segment_aabb_hit};
use fate_sim::{StateHasher, StateWriter};

const fn v(x: i64, y: i64, z: i64) -> V {
    V::new(F::from_int(x), F::from_int(y), F::from_int(z))
}

const BOXES: [(V, V); 2] = [(v(2, -2, -1), v(3, 2, 1)), (v(5, -2, 2), v(6, 2, 4))];
const COMMANDS: [V; 9] = [
    v(4, 0, 0),
    v(4, 0, 0),
    v(0, 0, 3),
    v(7, 0, 0),
    v(0, 3, 0),
    v(7, 0, -4),
    v(0, -4, 0),
    v(-6, 0, 0),
    v(1, 0, 0),
];
const SLIDE_COMMANDS: [V; 9] = [
    v(4, 0, 2),
    v(0, 0, -2),
    v(4, 0, 0),
    v(0, 0, 5),
    v(5, 0, -4),
    v(1, 0, 0),
    v(0, 3, 0),
    v(-6, 0, 0),
    v(0, -6, 0),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Actor {
    tick: u64,
    contacts: u64,
    position: V,
}

impl Actor {
    fn step(&mut self, delta: V) -> Option<F> {
        let end = self.position + delta;
        let contact = BOXES
            .into_iter()
            .filter_map(|(min, max)| segment_aabb_contact(self.position, end, min, max))
            .min();
        // This consumer deliberately rejects a whole blocked command. The
        // query does not choose a response, invent a tangent, or move the actor.
        if contact.is_some() {
            self.contacts += 1;
        } else {
            self.position = end;
        }
        self.tick += 1;
        contact
    }

    fn step_slide(&mut self, delta: V) -> Option<V> {
        let mut remaining = delta;
        let mut first = None;
        // Two contacts per tick are enough for this fixture. Any leftover
        // motion is dropped rather than looping indefinitely in a corner.
        for _ in 0..2 {
            let end = self.position + remaining;
            let hit = BOXES
                .into_iter()
                .filter_map(|(min, max)| segment_aabb_hit(self.position, end, min, max))
                .min_by_key(|hit| hit.fraction);
            let Some(hit) = hit else {
                self.position = end;
                break;
            };
            first.get_or_insert(hit.normal);
            if hit.fraction == F::ZERO {
                // Already touching a face: do not push through it.
                break;
            }
            // Stop just outside the face, then spend the rest of the tick on
            // the tangent plane. The one-raw-unit gap keeps the next tick from
            // starting exactly on the surface.
            let contact = self.position + remaining * hit.fraction;
            self.position = contact + hit.normal * F::from_raw(1);
            let mut rest = remaining * (F::ONE - hit.fraction);
            let into = rest.dot(hit.normal);
            if into < F::ZERO {
                rest = rest - hit.normal * into;
            }
            remaining = rest;
        }
        if first.is_some() {
            self.contacts += 1;
        }
        self.tick += 1;
        first
    }

    fn checkpoint(self) -> [u8; 40] {
        let mut bytes = [0; 40];
        for (slot, value) in bytes.as_chunks_mut::<8>().0.iter_mut().zip([
            self.tick,
            self.contacts,
            self.position.x.raw() as u64,
            self.position.y.raw() as u64,
            self.position.z.raw() as u64,
        ]) {
            *slot = value.to_le_bytes();
        }
        bytes
    }

    fn restore(bytes: [u8; 40]) -> Self {
        let values: [u64; 5] =
            std::array::from_fn(|i| u64::from_le_bytes(bytes.as_chunks::<8>().0[i]));
        let [tick, contacts, x, y, z] = values;
        Self {
            tick,
            contacts,
            position: V::new(
                F::from_raw(x as i64),
                F::from_raw(y as i64),
                F::from_raw(z as i64),
            ),
        }
    }

    fn checksum(self) -> u64 {
        let mut hash = StateHasher::new();
        hash.write_bytes(&self.checkpoint());
        hash.finish()
    }
}

fn run(mut actor: Actor) -> Vec<(Actor, Option<F>)> {
    COMMANDS[actor.tick as usize..]
        .iter()
        .map(|&delta| {
            let hit = actor.step(delta);
            (actor, hit)
        })
        .collect()
}

fn verify() -> Actor {
    let baseline = run(Actor::default());
    assert_eq!(baseline, run(Actor::default()));
    for (i, &(actor, _)) in baseline.iter().enumerate() {
        assert_eq!(run(Actor::restore(actor.checkpoint())), baseline[i + 1..]);
    }
    assert_eq!(
        baseline
            .iter()
            .filter_map(|(actor, hit)| hit.map(|t| (actor.tick, t)))
            .collect::<Vec<_>>(),
        [
            (1, F::from_ratio(1, 2)),
            (2, F::from_ratio(1, 2)),
            (4, F::from_ratio(5, 7)),
            (8, F::from_ratio(2, 3))
        ],
    );
    let final_actor = baseline.last().unwrap().0;
    assert_eq!(
        final_actor,
        Actor {
            tick: 9,
            contacts: 4,
            position: v(8, -1, -1)
        }
    );
    assert_eq!(final_actor.checksum(), 0x012d_ff0f_522a_9e58);
    final_actor
}

fn run_slide(mut actor: Actor) -> Vec<(Actor, Option<V>)> {
    SLIDE_COMMANDS[actor.tick as usize..]
        .iter()
        .map(|&delta| {
            let normal = actor.step_slide(delta);
            (actor, normal)
        })
        .collect()
}

fn verify_slide() -> Actor {
    let baseline = run_slide(Actor::default());
    assert_eq!(baseline, run_slide(Actor::default()));
    for (i, &(actor, _)) in baseline.iter().enumerate() {
        assert_eq!(
            run_slide(Actor::restore(actor.checkpoint())),
            baseline[i + 1..]
        );
    }
    for &(actor, _) in &baseline {
        for (min, max) in BOXES {
            let inside = |value: F, min: F, max: F| min < value && value < max;
            assert!(
                !(inside(actor.position.x, min.x, max.x)
                    && inside(actor.position.y, min.y, max.y)
                    && inside(actor.position.z, min.z, max.z)),
                "tick {} ended inside {min:?}..{max:?}",
                actor.tick
            );
        }
    }
    assert_eq!(
        baseline
            .iter()
            .filter_map(|(actor, normal)| normal.map(|normal| (actor.tick, normal)))
            .collect::<Vec<_>>(),
        [(1, v(-1, 0, 0)), (3, v(-1, 0, 0)), (5, v(-1, 0, 0)),],
    );
    let final_actor = baseline.last().unwrap().0;
    assert_eq!(final_actor.tick, 9);
    assert_eq!(final_actor.contacts, 3);
    assert_eq!(final_actor.position.y, F::from_int(-3));
    assert_eq!(final_actor.position.z, F::from_int(1));
    // The actor stays just outside the last face it slid along. The exact gap
    // accumulates a few Q32.32 units from contact rounding and backoff.
    assert!(final_actor.position.x.raw() < 0);
    assert!(final_actor.position.x.raw() > -(1 << 20));
    assert_eq!(final_actor.checksum(), 0xe996_c878_c537_580c);
    final_actor
}

fn main() {
    let actor = verify();
    let slid = verify_slide();
    println!(
        "3D contact smoke: {} ticks, {} contacts, final checksum {:016x}",
        actor.tick,
        actor.contacts,
        actor.checksum()
    );
    println!(
        "3D slide smoke: {} ticks, {} contacts over 3 faces, final checksum {:016x}",
        slid.tick,
        slid.contacts,
        slid.checksum()
    );
}

#[test]
fn scripted_3d_contacts_replay_and_restore() {
    verify();
    verify_slide();
}
