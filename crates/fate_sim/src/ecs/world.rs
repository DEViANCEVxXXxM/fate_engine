use crate::math::{Fixed, FixedVec2};

pub type Entity = u32;
const MAX_SNAPSHOT_ENTITIES: u32 = 1_000_000;

/// A dense column of optional simulation components, indexed by `Entity`.
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
pub struct Position(pub FixedVec2);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Velocity(pub FixedVec2);

/// 2D orientation in radians.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rotation(pub Fixed);

/// Angular velocity in radians per second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spin(pub Fixed);

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

    pub fn was_allocated(&self, entity: Entity) -> bool {
        entity < self.next_entity
    }

    pub fn entity_count(&self) -> usize {
        self.alive.iter().filter(|alive| **alive).count()
    }

    pub fn encode_state(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(&self.next_entity.to_le_bytes());
        let slot_count = self
            .alive
            .len()
            .max(self.positions.len())
            .max(self.velocities.len())
            .max(self.rotations.len())
            .max(self.spins.len());
        output.extend_from_slice(&(slot_count as u32).to_le_bytes());

        for index in 0..slot_count {
            let entity = index as Entity;
            let mut component_mask = 0_u8;
            if self.positions.contains(entity) {
                component_mask |= 1 << 0;
            }
            if self.velocities.contains(entity) {
                component_mask |= 1 << 1;
            }
            if self.rotations.contains(entity) {
                component_mask |= 1 << 2;
            }
            if self.spins.contains(entity) {
                component_mask |= 1 << 3;
            }

            output.extend_from_slice(&entity.to_le_bytes());
            output.push(self.is_alive(entity) as u8);
            output.push(component_mask);

            if let Some(position) = self.positions.get(entity) {
                output.extend_from_slice(&position.0.x.raw().to_le_bytes());
                output.extend_from_slice(&position.0.y.raw().to_le_bytes());
            }
            if let Some(velocity) = self.velocities.get(entity) {
                output.extend_from_slice(&velocity.0.x.raw().to_le_bytes());
                output.extend_from_slice(&velocity.0.y.raw().to_le_bytes());
            }
            if let Some(rotation) = self.rotations.get(entity) {
                output.extend_from_slice(&rotation.0.raw().to_le_bytes());
            }
            if let Some(spin) = self.spins.get(entity) {
                output.extend_from_slice(&spin.0.raw().to_le_bytes());
            }
        }
    }

    pub fn decode_state(input: &[u8], offset: &mut usize) -> Result<Self, &'static str> {
        let next_entity = Self::read_u32(input, offset)?;
        let slot_count = Self::read_u32(input, offset)?;
        if slot_count != next_entity || slot_count > MAX_SNAPSHOT_ENTITIES {
            return Err("invalid world entity count");
        }
        if *offset > input.len() || (input.len() - *offset) / 6 < slot_count as usize {
            return Err("truncated world entity records");
        }

        let mut world = Self::new();
        world.next_entity = next_entity;
        world.alive.resize(slot_count as usize, false);

        for expected_entity in 0..slot_count {
            let entity = Self::read_u32(input, offset)?;
            let alive = Self::read_u8(input, offset)?;
            let component_mask = Self::read_u8(input, offset)?;
            if entity != expected_entity || alive > 1 || component_mask & !0x0f != 0 {
                return Err("invalid world entity record");
            }
            if alive == 0 && component_mask != 0 {
                return Err("despawned entity has components");
            }
            world.alive[entity as usize] = alive == 1;

            if component_mask & (1 << 0) != 0 {
                let x = Fixed::from_raw(Self::read_i64(input, offset)?);
                let y = Fixed::from_raw(Self::read_i64(input, offset)?);
                world
                    .positions
                    .insert(entity, Position(FixedVec2::new(x, y)));
            }
            if component_mask & (1 << 1) != 0 {
                let x = Fixed::from_raw(Self::read_i64(input, offset)?);
                let y = Fixed::from_raw(Self::read_i64(input, offset)?);
                world
                    .velocities
                    .insert(entity, Velocity(FixedVec2::new(x, y)));
            }
            if component_mask & (1 << 2) != 0 {
                world.rotations.insert(
                    entity,
                    Rotation(Fixed::from_raw(Self::read_i64(input, offset)?)),
                );
            }
            if component_mask & (1 << 3) != 0 {
                world.spins.insert(
                    entity,
                    Spin(Fixed::from_raw(Self::read_i64(input, offset)?)),
                );
            }
        }

        Ok(world)
    }

    fn read_u8(input: &[u8], offset: &mut usize) -> Result<u8, &'static str> {
        let value = *input.get(*offset).ok_or("truncated world state")?;
        *offset += 1;
        Ok(value)
    }

    fn read_u32(input: &[u8], offset: &mut usize) -> Result<u32, &'static str> {
        let end = offset.checked_add(4).ok_or("truncated world state")?;
        let bytes = input.get(*offset..end).ok_or("truncated world state")?;
        *offset = end;
        Ok(u32::from_le_bytes(
            bytes.try_into().expect("fixed-size slice"),
        ))
    }

    fn read_i64(input: &[u8], offset: &mut usize) -> Result<i64, &'static str> {
        let end = offset.checked_add(8).ok_or("truncated world state")?;
        let bytes = input.get(*offset..end).ok_or("truncated world state")?;
        *offset = end;
        Ok(i64::from_le_bytes(
            bytes.try_into().expect("fixed-size slice"),
        ))
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
        world.positions.insert(
            e,
            Position(FixedVec2::new(Fixed::from_int(1), Fixed::from_int(2))),
        );
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
        world.spins.insert(b, Spin(Fixed::ONE));
        assert!(world.spins.get(a).is_none());
        assert_eq!(world.spins.get(b), Some(&Spin(Fixed::ONE)));
    }

    #[test]
    fn despawn_clears_all_component_columns() {
        let mut world = World::new();
        let e = world.spawn();

        world.positions.insert(
            e,
            Position(FixedVec2::new(Fixed::from_int(1), Fixed::from_int(2))),
        );
        world.velocities.insert(
            e,
            Velocity(FixedVec2::new(Fixed::from_int(3), Fixed::from_int(4))),
        );
        world.rotations.insert(e, Rotation(Fixed::from_ratio(1, 2)));
        world.spins.insert(e, Spin(Fixed::from_ratio(5, 4)));

        world.despawn(e);

        assert!(!world.is_alive(e));
        assert!(!world.positions.contains(e));
        assert!(!world.velocities.contains(e));
        assert!(!world.rotations.contains(e));
        assert!(!world.spins.contains(e));
        assert_eq!(world.entity_count(), 0);
    }
}
