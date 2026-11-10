use crate::math::Vec2;

pub type Entity = u32;

/// A dense column of optional components, indexed by `Entity`.
pub struct Column<T> {
    data: Vec<Option<T>>,
}

/// API surface for later milestones; not every method has a caller yet.
#[allow(dead_code)]
impl<T: Copy> Column<T> {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    fn grow_to(&mut self, index: usize) {
        if self.data.len() <= index {
            self.data.resize(index + 1, None);
        }
    }

    pub fn insert(&mut self, entity: Entity, value: T) {
        self.grow_to(entity as usize);
        self.data[entity as usize] = Some(value);
    }

    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        self.data
            .get_mut(entity as usize)
            .and_then(|slot| slot.take())
    }

    pub fn get(&self, entity: Entity) -> Option<&T> {
        self.data.get(entity as usize).and_then(Option::as_ref)
    }

    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        self.data.get_mut(entity as usize).and_then(Option::as_mut)
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.get(entity).is_some()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Iterate over `(entity, &component)` for every populated slot.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.data
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_ref().map(|v| (i as Entity, v)))
    }

    /// Iterate mutably over `(entity, &mut component)`.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.data
            .iter_mut()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_mut().map(|v| (i as Entity, v)))
    }
}

impl<T: Copy> Default for Column<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Position(pub Vec2);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Velocity(pub Vec2);

/// Orientation in radians, for the 2D demo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rotation(pub f32);

/// Angular velocity in radians per second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spin(pub f32);

pub struct World {
    next_entity: Entity,
    alive: Vec<bool>,
    pub positions: Column<Position>,
    pub velocities: Column<Velocity>,
    pub rotations: Column<Rotation>,
    pub spins: Column<Spin>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl World {
    pub fn new() -> Self {
        Self {
            next_entity: 0,
            alive: Vec::new(),
            positions: Column::new(),
            velocities: Column::new(),
            rotations: Column::new(),
            spins: Column::new(),
        }
    }

    pub fn spawn(&mut self) -> Entity {
        let entity = self.next_entity;
        self.next_entity += 1;
        self.alive.push(true);
        entity
    }

    pub fn despawn(&mut self, entity: Entity) {
        if let Some(alive) = self.alive.get_mut(entity as usize) {
            *alive = false;
            self.positions.remove(entity);
            self.velocities.remove(entity);
            self.rotations.remove(entity);
            self.spins.remove(entity);
        }
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.alive.get(entity as usize).copied().unwrap_or(false)
    }

    pub fn entity_count(&self) -> usize {
        self.alive.iter().filter(|alive| **alive).count()
    }

    pub fn with_position(mut self, entity: Entity, value: Position) -> Self {
        self.positions.insert(entity, value);
        self
    }

    pub fn with_velocity(mut self, entity: Entity, value: Velocity) -> Self {
        self.velocities.insert(entity, value);
        self
    }

    pub fn with_rotation(mut self, entity: Entity, value: Rotation) -> Self {
        self.rotations.insert(entity, value);
        self
    }

    pub fn with_spin(mut self, entity: Entity, value: Spin) -> Self {
        self.spins.insert(entity, value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_insert_remove() {
        let mut world = World::new();
        let e = world.spawn();
        world.positions.insert(e, Position(Vec2::new(1.0, 2.0)));
        assert!(world.positions.contains(e));

        world.despawn(e);
        assert!(!world.is_alive(e));
        assert!(!world.positions.contains(e));
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn columns_grow_with_entities() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.spins.insert(b, Spin(1.0));
        assert!(world.spins.get(a).is_none());
        assert_eq!(world.spins.get(b), Some(&Spin(1.0)));
    }

    #[test]
    fn despawn_clears_all_component_columns() {
        let mut world = World::new();
        let e = world.spawn();

        world.positions.insert(e, Position(Vec2::new(1.0, 2.0)));
        world.velocities.insert(e, Velocity(Vec2::new(3.0, 4.0)));
        world.rotations.insert(e, Rotation(0.5));
        world.spins.insert(e, Spin(1.25));

        world.despawn(e);

        assert!(!world.is_alive(e));
        assert!(!world.positions.contains(e));
        assert!(!world.velocities.contains(e));
        assert!(!world.rotations.contains(e));
        assert!(!world.spins.contains(e));
        assert_eq!(world.entity_count(), 0);
    }
}
