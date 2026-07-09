//! Generational entity handles and their allocator.

use serde::{Deserialize, Serialize};

use crate::error::EcsError;

/// A handle to an entity: a slot index plus the generation that was live
/// when the handle was issued.
///
/// Invariants:
/// - A handle is valid iff its generation equals the allocator's current
///   generation for that index and the slot is alive; stale handles are
///   detected, never silently re-targeted.
/// - Ordering is by `(index, generation)`; iteration over entities in
///   index order is the canonical deterministic order (SPEC §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Entity {
    index: u32,
    generation: u32,
}

impl Entity {
    /// The slot index. Stable for the lifetime of this entity.
    pub const fn index(self) -> u32 {
        self.index
    }

    /// The generation at which this handle was issued.
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

/// Allocator of generational entity slots.
///
/// Invariants:
/// - Freed indices are recycled LIFO from an explicit stack, so allocation
///   order is deterministic and survives serialization verbatim (ADR 0002 §7).
/// - Generations bump on despawn (wrapping; a u32 wrap of one slot requires
///   2^32 despawns of that slot — the resulting ABA window is accepted and
///   documented in ADR 0002 §7).
/// - `alive.len() == generations.len()` always.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityAllocator {
    generations: Vec<u32>,
    alive: Vec<bool>,
    free: Vec<u32>,
}

impl EntityAllocator {
    /// Creates an empty allocator.
    pub fn new() -> Self {
        EntityAllocator::default()
    }

    /// Allocates a new live entity, recycling the most recently freed slot
    /// if one exists (LIFO).
    pub fn spawn(&mut self) -> Entity {
        if let Some(index) = self.free.pop() {
            let slot = index as usize;
            // Invariant: indices on the free list are always in range.
            if let (Some(alive), Some(generation)) =
                (self.alive.get_mut(slot), self.generations.get(slot))
            {
                *alive = true;
                return Entity {
                    index,
                    generation: *generation,
                };
            }
        }
        let index = self.generations.len() as u32;
        self.generations.push(0);
        self.alive.push(true);
        Entity {
            index,
            generation: 0,
        }
    }

    /// Kills a live entity, bumping its slot generation so stale handles are
    /// invalidated, and returns its slot to the free stack.
    pub fn despawn(&mut self, entity: Entity) -> Result<(), EcsError> {
        if !self.is_alive(entity) {
            return Err(EcsError::DeadEntity(entity));
        }
        let slot = entity.index as usize;
        if let (Some(alive), Some(generation)) =
            (self.alive.get_mut(slot), self.generations.get_mut(slot))
        {
            *alive = false;
            *generation = generation.wrapping_add(1);
            self.free.push(entity.index);
            Ok(())
        } else {
            Err(EcsError::InternalCorruption(
                "alive slot index out of allocator range",
            ))
        }
    }

    /// True iff the handle refers to a currently-live entity (index in
    /// range, slot alive, generation current).
    pub fn is_alive(&self, entity: Entity) -> bool {
        let slot = entity.index as usize;
        self.alive.get(slot).copied().unwrap_or(false)
            && self.generations.get(slot).copied() == Some(entity.generation)
    }

    /// Returns the live entity occupying `index`, if any. Used to rebuild
    /// full handles when iterating component stores (which key by index).
    pub fn entity_at(&self, index: u32) -> Option<Entity> {
        let slot = index as usize;
        if self.alive.get(slot).copied().unwrap_or(false) {
            self.generations
                .get(slot)
                .map(|&generation| Entity { index, generation })
        } else {
            None
        }
    }

    /// Iterates all live entities in ascending index order (the canonical
    /// deterministic order, SPEC §6).
    pub fn iter_alive(&self) -> impl Iterator<Item = Entity> + '_ {
        (0..self.generations.len() as u32).filter_map(|index| self.entity_at(index))
    }

    /// Number of currently live entities.
    pub fn alive_count(&self) -> usize {
        self.alive.iter().filter(|&&a| a).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_despawn_respawn_recycles_lifo_with_new_generation() {
        let mut alloc = EntityAllocator::new();
        let a = alloc.spawn();
        let b = alloc.spawn();
        assert_eq!(a.index(), 0);
        assert_eq!(b.index(), 1);

        alloc.despawn(a).unwrap();
        alloc.despawn(b).unwrap();
        // LIFO: b's slot comes back first, at generation 1.
        let c = alloc.spawn();
        assert_eq!(c.index(), 1);
        assert_eq!(c.generation(), 1);

        // Stale handles are dead even though the slot is live again.
        assert!(!alloc.is_alive(b));
        assert!(alloc.is_alive(c));
    }

    #[test]
    fn despawning_a_dead_entity_is_a_typed_error() {
        let mut alloc = EntityAllocator::new();
        let a = alloc.spawn();
        alloc.despawn(a).unwrap();
        assert!(matches!(alloc.despawn(a), Err(EcsError::DeadEntity(_))));
    }

    #[test]
    fn iter_alive_is_in_index_order() {
        let mut alloc = EntityAllocator::new();
        let entities: Vec<Entity> = (0..5).map(|_| alloc.spawn()).collect();
        alloc.despawn(entities[2]).unwrap();
        let indices: Vec<u32> = alloc.iter_alive().map(Entity::index).collect();
        assert_eq!(indices, vec![0, 1, 3, 4]);
        assert_eq!(alloc.alive_count(), 4);
    }

    #[test]
    fn serde_round_trip_preserves_free_list_order() {
        let mut alloc = EntityAllocator::new();
        let entities: Vec<Entity> = (0..4).map(|_| alloc.spawn()).collect();
        alloc.despawn(entities[1]).unwrap();
        alloc.despawn(entities[3]).unwrap();

        let bytes = core_types::codec::to_bytes(&alloc).unwrap();
        let mut restored: EntityAllocator = core_types::codec::from_bytes(&bytes).unwrap();

        // Both must recycle in the same (LIFO) order.
        assert_eq!(alloc.spawn(), restored.spawn());
        assert_eq!(alloc.spawn(), restored.spawn());
        assert_eq!(alloc, restored);
    }
}
