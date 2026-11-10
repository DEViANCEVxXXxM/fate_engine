use super::world::World;
use crate::math::{Fixed, FixedVec2};

/// Integrate fixed-point position from velocity: `p += v * dt`.
///
/// Positions and velocities live in separate columns, so destructuring the
/// `World` gives two disjoint borrows and the loop stays a plain `for`.
pub fn movement_system(world: &mut World, dt: Fixed) {
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
pub fn spin_system(world: &mut World, dt: Fixed) {
    let World {
        rotations, spins, ..
    } = world;

    for (entity, rotation) in rotations.iter_mut() {
        if let Some(spin) = spins.get(entity) {
            rotation.0 += spin.0 * dt;
            // Keep the angle bounded so it stays precise over long sessions.
            let tau = Fixed::from_ratio(6_283_185, 1_000_000);
            rotation.0 = rotation.0.rem_euclid(tau);
        }
    }
}

/// Reflect entities off the playfield walls.
pub fn bounds_system(world: &mut World, half: FixedVec2) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::world::{Position, Velocity};
    use crate::math::{Fixed, FixedVec2};

    fn f(value: i64) -> Fixed {
        Fixed::from_int(value)
    }

    #[test]
    fn movement_integrates_velocity() {
        let mut world = World::new();
        let e = world.spawn();
        world.positions.insert(e, Position(FixedVec2::ZERO));
        world
            .velocities
            .insert(e, Velocity(FixedVec2::new(f(2), Fixed::ZERO)));

        movement_system(&mut world, Fixed::from_ratio(1, 2));

        assert_eq!(
            world.positions.get(e).unwrap().0,
            FixedVec2::new(f(1), Fixed::ZERO)
        );
    }

    #[test]
    fn static_entities_do_not_move() {
        let mut world = World::new();
        let e = world.spawn();
        world
            .positions
            .insert(e, Position(FixedVec2::new(f(5), f(5))));

        movement_system(&mut world, Fixed::ONE);

        assert_eq!(
            world.positions.get(e).unwrap().0,
            FixedVec2::new(f(5), f(5))
        );
    }

    #[test]
    fn spin_wraps_negative_angles_into_a_positive_turn() {
        let mut world = World::new();
        let e = world.spawn();
        world
            .rotations
            .insert(e, crate::ecs::world::Rotation(Fixed::from_ratio(1, 10)));
        world
            .spins
            .insert(e, crate::ecs::world::Spin(Fixed::from_ratio(-1, 2)));

        spin_system(&mut world, Fixed::ONE);

        let tau = Fixed::from_ratio(6_283_185, 1_000_000);
        let expected = tau - Fixed::from_ratio(2, 5);
        assert!(
            world
                .rotations
                .get(e)
                .unwrap()
                .0
                .raw()
                .abs_diff(expected.raw())
                <= 1
        );
    }

    #[test]
    fn bounds_reflect_and_clamp_fixed_point_entities() {
        let mut world = World::new();
        let e = world.spawn();
        world
            .positions
            .insert(e, Position(FixedVec2::new(f(6), Fixed::from_ratio(-7, 2))));
        world
            .velocities
            .insert(e, Velocity(FixedVec2::new(f(3), f(-2))));

        bounds_system(&mut world, FixedVec2::new(f(5), f(4)));

        assert_eq!(
            world.positions.get(e).unwrap().0,
            FixedVec2::new(f(5), Fixed::from_ratio(-7, 2))
        );
        assert_eq!(
            world.velocities.get(e).unwrap().0,
            FixedVec2::new(f(-3), f(-2))
        );
    }
}
