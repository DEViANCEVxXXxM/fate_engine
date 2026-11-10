use super::world::World;
use crate::math::Vec2;

/// Integrate position from velocity: `p += v * dt`.
///
/// Positions and velocities live in separate columns, so destructuring the
/// `World` gives two disjoint borrows and the loop stays a plain `for`.
pub fn movement_system(world: &mut World, dt: f32) {
    let World {
        positions,
        velocities,
        ..
    } = world;

    for (entity, position) in positions.iter_mut() {
        if let Some(velocity) = velocities.get(entity) {
            position.0 += velocity.0 * dt;
        }
    }
}

/// Integrate rotation from angular velocity: `angle += spin * dt`.
pub fn spin_system(world: &mut World, dt: f32) {
    let World {
        rotations, spins, ..
    } = world;

    for (entity, rotation) in rotations.iter_mut() {
        if let Some(spin) = spins.get(entity) {
            rotation.0 += spin.0 * dt;
            // Keep the angle bounded so it stays precise over long sessions.
            let tau = std::f32::consts::TAU;
            rotation.0 = rotation.0.rem_euclid(tau);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ecs::world::Position, math::Vec2};

    #[test]
    fn movement_integrates_velocity() {
        let mut world = World::new();
        let e = world.spawn();
        world.positions.insert(e, Position(Vec2::new(0.0, 0.0)));
        world.velocities.insert(e, crate::ecs::world::Velocity(Vec2::new(2.0, 0.0)));

        movement_system(&mut world, 0.5);

        assert_eq!(world.positions.get(e).unwrap().0, Vec2::new(1.0, 0.0));
    }

    #[test]
    fn static_entities_do_not_move() {
        let mut world = World::new();
        let e = world.spawn();
        world.positions.insert(e, Position(Vec2::new(5.0, 5.0)));

        movement_system(&mut world, 1.0);

        assert_eq!(world.positions.get(e).unwrap().0, Vec2::new(5.0, 5.0));
    }
}

/// Reflect entities off the playfield walls.
pub fn bounds_system(world: &mut World, half: Vec2) {
    let World {
        positions,
        velocities,
        ..
    } = world;

    for (entity, position) in positions.iter_mut() {
        let Some(velocity) = velocities.get_mut(entity) else {
            continue;
        };

        if position.0.x > half.x || position.0.x < -half.x {
            velocity.0.x = -velocity.0.x;
            position.0.x = position.0.x.clamp(-half.x, half.x);
        }
        if position.0.y > half.y || position.0.y < -half.y {
            velocity.0.y = -velocity.0.y;
            position.0.y = position.0.y.clamp(-half.y, half.y);
        }
    }
}

