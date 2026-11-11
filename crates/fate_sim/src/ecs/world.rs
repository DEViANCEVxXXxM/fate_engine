use crate::math::{Fixed, FixedVec2};

const MAX_SNAPSHOT_ENTITIES: u32 = 1_000_000;

/// Generational entity identifier preventing the ABA problem and stale handle reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entity {
    id: u32,
    generation: u32,
}

impl Entity {
    pub const DANGLING: Self = Self {
        id: u32::MAX,
        generation: 0,
    };

    pub const fn new(id: u32, generation: u32) -> Self {
        Self { id, generation }
    }

    pub const fn id(self) -> u32 {
        self.id
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }

    pub const fn to_bits(self) -> u64 {
        (self.id as u64) | ((self.generation as u64) << 32)
    }

    pub const fn from_bits(bits: u64) -> Self {
        Self {
            id: bits as u32,
            generation: (bits >> 32) as u32,
        }
    }
}

impl Default for Entity {
    fn default() -> Self {
        Self::DANGLING
    }
}

/// Errors returned by ECS lifecycle and component operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EcsError {
    EntityNotAlive(Entity),
    InvalidEntity(Entity),
    EntityIdExhausted,
}

impl std::fmt::Display for EcsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EntityNotAlive(entity) => {
                write!(
                    formatter,
                    "entity (id={}, gen={}) is not alive",
                    entity.id, entity.generation
                )
            }
            Self::InvalidEntity(entity) => {
                write!(
                    formatter,
                    "entity (id={}, gen={}) is invalid or out of range",
                    entity.id, entity.generation
                )
            }
            Self::EntityIdExhausted => {
                formatter.write_str("entity ID allocation space is exhausted")
            }
        }
    }
}

impl std::error::Error for EcsError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ComponentEntry<T> {
    generation: u32,
    value: T,
}

/// A dense column of optional simulation components, indexed by `Entity::id`
/// and validated by `Entity::generation`.
pub struct Column<T> {
    data: Vec<Option<ComponentEntry<T>>>,
}

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
        self.grow_to(entity.id as usize);
        self.data[entity.id as usize] = Some(ComponentEntry {
            generation: entity.generation,
            value,
        });
    }

    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        let slot = self.data.get_mut(entity.id as usize)?;
        if slot
            .as_ref()
            .is_some_and(|entry| entry.generation == entity.generation)
        {
            slot.take().map(|entry| entry.value)
        } else {
            None
        }
    }

    pub fn get(&self, entity: Entity) -> Option<&T> {
        self.data
            .get(entity.id as usize)
            .and_then(Option::as_ref)
            .and_then(|entry| {
                if entry.generation == entity.generation {
                    Some(&entry.value)
                } else {
                    None
                }
            })
    }

    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        self.data
            .get_mut(entity.id as usize)
            .and_then(Option::as_mut)
            .and_then(|entry| {
                if entry.generation == entity.generation {
                    Some(&mut entry.value)
                } else {
                    None
                }
            })
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

    pub fn clear_slot(&mut self, id: u32) {
        if let Some(slot) = self.data.get_mut(id as usize) {
            slot.take();
        }
    }

    /// Iterate over `(entity, &component)` for every populated slot matching its stored generation.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.data.iter().enumerate().filter_map(|(id, slot)| {
            slot.as_ref()
                .map(|entry| (Entity::new(id as u32, entry.generation), &entry.value))
        })
    }

    /// Iterate mutably over `(entity, &mut component)`.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.data.iter_mut().enumerate().filter_map(|(id, slot)| {
            slot.as_mut()
                .map(|entry| (Entity::new(id as u32, entry.generation), &mut entry.value))
        })
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
    next_id: u32,
    generations: Vec<u32>,
    alive: Vec<bool>,
    free_list: Vec<u32>,
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
            next_id: 0,
            generations: Vec::new(),
            alive: Vec::new(),
            free_list: Vec::new(),
            positions: Column::new(),
            velocities: Column::new(),
            rotations: Column::new(),
            spins: Column::new(),
        }
    }

    /// Allocate a new entity handle or recycle a despawned slot.
    pub fn try_spawn(&mut self) -> Result<Entity, EcsError> {
        if let Some(id) = self.free_list.pop() {
            let index = id as usize;
            self.alive[index] = true;
            let generation = self.generations[index];
            Ok(Entity::new(id, generation))
        } else {
            if self.next_id == u32::MAX {
                return Err(EcsError::EntityIdExhausted);
            }
            let id = self.next_id;
            self.next_id += 1;
            self.generations.push(1);
            self.alive.push(true);
            Ok(Entity::new(id, 1))
        }
    }

    /// Allocate a new entity handle, panicking if the 32-bit entity ID space is exhausted.
    pub fn spawn(&mut self) -> Entity {
        self.try_spawn().expect("entity ID space exhausted")
    }

    /// Despawn an entity, removing all its components and recycling its slot.
    ///
    /// Returns `true` if the entity was alive and was despawned; returns `false`
    /// if the entity handle was already dead or invalid (idempotent repeated despawn).
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let index = entity.id as usize;
        self.alive[index] = false;
        let next_gen = self.generations[index].wrapping_add(1);
        self.generations[index] = if next_gen == 0 { 1 } else { next_gen };
        self.free_list.push(entity.id);
        self.positions.remove(entity);
        self.velocities.remove(entity);
        self.rotations.remove(entity);
        self.spins.remove(entity);
        true
    }

    /// Check if the entity handle corresponds to an active, non-despawned entity
    /// with a matching generation.
    pub fn is_alive(&self, entity: Entity) -> bool {
        let index = entity.id as usize;
        self.generations
            .get(index)
            .is_some_and(|&slot_gen| slot_gen == entity.generation)
            && self.alive.get(index).copied().unwrap_or(false)
    }

    /// Check whether this entity ID was ever allocated in the world.
    pub fn was_allocated(&self, entity: Entity) -> bool {
        entity.id < self.next_id
    }

    /// Return the count of currently living entities.
    pub fn entity_count(&self) -> usize {
        self.alive.iter().filter(|alive| **alive).count()
    }

    /// Return the total number of entity slots allocated so far.
    pub fn capacity(&self) -> usize {
        self.next_id as usize
    }

    /// Return the current generation of an entity slot.
    pub fn slot_generation(&self, id: u32) -> Option<u32> {
        self.generations.get(id as usize).copied()
    }

    pub fn set_position(&mut self, entity: Entity, value: Position) -> Result<(), EcsError> {
        if !self.is_alive(entity) {
            return Err(EcsError::EntityNotAlive(entity));
        }
        self.positions.insert(entity, value);
        Ok(())
    }

    pub fn set_velocity(&mut self, entity: Entity, value: Velocity) -> Result<(), EcsError> {
        if !self.is_alive(entity) {
            return Err(EcsError::EntityNotAlive(entity));
        }
        self.velocities.insert(entity, value);
        Ok(())
    }

    pub fn set_rotation(&mut self, entity: Entity, value: Rotation) -> Result<(), EcsError> {
        if !self.is_alive(entity) {
            return Err(EcsError::EntityNotAlive(entity));
        }
        self.rotations.insert(entity, value);
        Ok(())
    }

    pub fn set_spin(&mut self, entity: Entity, value: Spin) -> Result<(), EcsError> {
        if !self.is_alive(entity) {
            return Err(EcsError::EntityNotAlive(entity));
        }
        self.spins.insert(entity, value);
        Ok(())
    }

    pub fn write_state<W: crate::simulation::StateWriter>(&self, writer: &mut W) {
        writer.write_u32(self.next_id);
        writer.write_u32(self.free_list.len() as u32);
        for &free_id in &self.free_list {
            writer.write_u32(free_id);
        }

        for id in 0..self.next_id {
            let index = id as usize;
            let generation = self.generations[index];
            let is_alive = self.alive[index];

            writer.write_u32(generation);
            writer.write_u8(is_alive as u8);

            if is_alive {
                let entity = Entity::new(id, generation);
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

                writer.write_u8(component_mask);

                if let Some(position) = self.positions.get(entity) {
                    writer.write_i64(position.0.x.raw());
                    writer.write_i64(position.0.y.raw());
                }
                if let Some(velocity) = self.velocities.get(entity) {
                    writer.write_i64(velocity.0.x.raw());
                    writer.write_i64(velocity.0.y.raw());
                }
                if let Some(rotation) = self.rotations.get(entity) {
                    writer.write_i64(rotation.0.raw());
                }
                if let Some(spin) = self.spins.get(entity) {
                    writer.write_i64(spin.0.raw());
                }
            }
        }
    }

    pub fn encode_state(&self, output: &mut Vec<u8>) {
        self.write_state(output);
    }

    pub fn decode_state(input: &[u8], offset: &mut usize) -> Result<Self, &'static str> {
        let next_id = Self::read_u32(input, offset)?;
        if next_id > MAX_SNAPSHOT_ENTITIES {
            return Err("invalid world entity slot count");
        }
        let free_list_len = Self::read_u32(input, offset)?;
        if free_list_len > next_id {
            return Err("free list length exceeds entity slot count");
        }

        let mut free_list = Vec::with_capacity(free_list_len as usize);
        for _ in 0..free_list_len {
            let free_id = Self::read_u32(input, offset)?;
            if free_id >= next_id {
                return Err("free list id out of bounds");
            }
            free_list.push(free_id);
        }

        let mut generations = Vec::with_capacity(next_id as usize);
        let mut alive = Vec::with_capacity(next_id as usize);
        let mut positions = Column::new();
        let mut velocities = Column::new();
        let mut rotations = Column::new();
        let mut spins = Column::new();
        let mut live_count = 0_usize;

        for id in 0..next_id {
            let generation = Self::read_u32(input, offset)?;
            let alive_byte = Self::read_u8(input, offset)?;
            if alive_byte > 1 {
                return Err("invalid alive flag");
            }
            let is_alive = alive_byte == 1;
            generations.push(generation);
            alive.push(is_alive);

            if is_alive {
                live_count += 1;
                let entity = Entity::new(id, generation);
                let component_mask = Self::read_u8(input, offset)?;
                if component_mask & !0x0f != 0 {
                    return Err("invalid component mask");
                }

                if component_mask & (1 << 0) != 0 {
                    let x = Fixed::from_raw(Self::read_i64(input, offset)?);
                    let y = Fixed::from_raw(Self::read_i64(input, offset)?);
                    positions.insert(entity, Position(FixedVec2::new(x, y)));
                }
                if component_mask & (1 << 1) != 0 {
                    let x = Fixed::from_raw(Self::read_i64(input, offset)?);
                    let y = Fixed::from_raw(Self::read_i64(input, offset)?);
                    velocities.insert(entity, Velocity(FixedVec2::new(x, y)));
                }
                if component_mask & (1 << 2) != 0 {
                    rotations.insert(
                        entity,
                        Rotation(Fixed::from_raw(Self::read_i64(input, offset)?)),
                    );
                }
                if component_mask & (1 << 3) != 0 {
                    spins.insert(
                        entity,
                        Spin(Fixed::from_raw(Self::read_i64(input, offset)?)),
                    );
                }
            }
        }

        if live_count + free_list.len() != next_id as usize {
            return Err("live entity count and free list size do not match total allocated slots");
        }
        for &free_id in &free_list {
            if alive[free_id as usize] {
                return Err("entity in free list is marked alive");
            }
        }

        Ok(Self {
            next_id,
            generations,
            alive,
            free_list,
            positions,
            velocities,
            rotations,
            spins,
        })
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
        self.set_position(entity, value)
            .expect("attempted to attach position to dead or invalid entity");
        self
    }

    pub fn with_velocity(mut self, entity: Entity, value: Velocity) -> Self {
        self.set_velocity(entity, value)
            .expect("attempted to attach velocity to dead or invalid entity");
        self
    }

    pub fn with_rotation(mut self, entity: Entity, value: Rotation) -> Self {
        self.set_rotation(entity, value)
            .expect("attempted to attach rotation to dead or invalid entity");
        self
    }

    pub fn with_spin(mut self, entity: Entity, value: Spin) -> Self {
        self.set_spin(entity, value)
            .expect("attempted to attach spin to dead or invalid entity");
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
        assert_eq!(e.id(), 0);
        assert_eq!(e.generation(), 1);
        world.positions.insert(
            e,
            Position(FixedVec2::new(Fixed::from_int(1), Fixed::from_int(2))),
        );
        assert!(world.positions.contains(e));

        assert!(world.despawn(e));
        assert!(!world.is_alive(e));
        assert!(!world.positions.contains(e));
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn repeated_despawn_is_safe_and_idempotent() {
        let mut world = World::new();
        let e = world.spawn();
        assert!(world.despawn(e));
        assert!(!world.despawn(e));
        assert!(!world.despawn(e));
        assert_eq!(world.entity_count(), 0);
        assert_eq!(world.free_list.len(), 1);
    }

    #[test]
    fn entity_slot_recycling_increments_generation() {
        let mut world = World::new();
        let e1 = world.spawn();
        assert_eq!(e1.id(), 0);
        assert_eq!(e1.generation(), 1);

        world
            .set_position(e1, Position(FixedVec2::new(Fixed::ONE, Fixed::ZERO)))
            .unwrap();
        assert!(world.despawn(e1));

        let e2 = world.spawn();
        assert_eq!(e2.id(), 0);
        assert_eq!(e2.generation(), 2);
        assert!(world.is_alive(e2));
        assert!(!world.is_alive(e1));

        // Stale handle cannot see components of new generation
        assert!(world.positions.get(e1).is_none());
        assert!(world.positions.get(e2).is_none());
        assert!(!world.positions.contains(e1));
        assert!(!world.positions.contains(e2));
    }

    #[test]
    fn dead_entities_cannot_acquire_components() {
        let mut world = World::new();
        let e = world.spawn();
        world.despawn(e);

        let err = world.set_position(e, Position(FixedVec2::ZERO));
        assert_eq!(err, Err(EcsError::EntityNotAlive(e)));

        let err_vel = world.set_velocity(e, Velocity(FixedVec2::ZERO));
        assert_eq!(err_vel, Err(EcsError::EntityNotAlive(e)));
    }

    #[test]
    fn invalid_and_dangling_entities_are_handled_safely() {
        let mut world = World::new();
        assert!(!world.is_alive(Entity::DANGLING));
        assert!(!world.despawn(Entity::DANGLING));
        assert_eq!(
            world.set_position(Entity::DANGLING, Position(FixedVec2::ZERO)),
            Err(EcsError::EntityNotAlive(Entity::DANGLING))
        );

        let ghost = Entity::new(999, 1);
        assert!(!world.is_alive(ghost));
        assert!(!world.despawn(ghost));
        assert_eq!(
            world.set_position(ghost, Position(FixedVec2::ZERO)),
            Err(EcsError::EntityNotAlive(ghost))
        );
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

        assert!(world.despawn(e));

        assert!(!world.is_alive(e));
        assert!(!world.positions.contains(e));
        assert!(!world.velocities.contains(e));
        assert!(!world.rotations.contains(e));
        assert!(!world.spins.contains(e));
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn snapshot_round_trip_preserves_generations_and_recycled_slots() {
        let mut world = World::new();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();

        world
            .set_position(
                e1,
                Position(FixedVec2::new(Fixed::from_int(10), Fixed::ZERO)),
            )
            .unwrap();
        world
            .set_position(
                e3,
                Position(FixedVec2::new(Fixed::from_int(30), Fixed::ZERO)),
            )
            .unwrap();

        // Despawn e2 so slot 1 is on the free list
        assert!(world.despawn(e2));

        let mut bytes = Vec::new();
        world.encode_state(&mut bytes);

        let mut offset = 0;
        let restored = World::decode_state(&bytes, &mut offset).unwrap();
        assert_eq!(offset, bytes.len());

        assert!(restored.is_alive(e1));
        assert!(!restored.is_alive(e2));
        assert!(restored.is_alive(e3));
        assert_eq!(restored.entity_count(), 2);
        assert_eq!(restored.capacity(), 3);
        assert_eq!(
            restored.positions.get(e1).unwrap().0,
            FixedVec2::new(Fixed::from_int(10), Fixed::ZERO)
        );
        assert_eq!(
            restored.positions.get(e3).unwrap().0,
            FixedVec2::new(Fixed::from_int(30), Fixed::ZERO)
        );

        // Next spawn in restored world should recycle slot 1 with gen 2
        let mut restored = restored;
        let e4 = restored.spawn();
        assert_eq!(e4.id(), 1);
        assert_eq!(e4.generation(), 2);
        assert!(restored.is_alive(e4));
    }
}
