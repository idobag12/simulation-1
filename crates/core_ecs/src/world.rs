//! The world: entity allocator + registered component stores + the RNG
//! registry (ADR 0002 §4).

use std::any::TypeId;
use std::collections::HashMap;

use core_rng::{Pcg64, RngRegistry};
use core_types::{Seed, StableHasher, codec};

use crate::entity::{Entity, EntityAllocator};
use crate::error::EcsError;
use crate::store::{Component, ErasedStore, StoreCell, StoreIter, StoreIterMut};

/// The complete mutable simulation state: entities, component stores, and
/// named RNG streams.
///
/// Invariants:
/// - Component registration order is fixed per application (one registration
///   function, called identically for fresh worlds and loads) and is part of
///   the save/hash format (ADR 0002 §6).
/// - `by_type` is a lookup-only `HashMap` (TypeId → store slot). It is never
///   iterated; all iteration walks `stores` in registration order or
///   entities in index order (SPEC §3).
/// - A component exists only for a live entity: `despawn` removes the
///   entity's components from every store at despawn time.
/// - All randomness flows through [`World::rng`] named streams (SPEC §2).
pub struct World {
    entities: EntityAllocator,
    stores: Vec<Box<dyn ErasedStore>>,
    by_type: HashMap<TypeId, usize>,
    rng: RngRegistry,
}

impl World {
    /// Creates an empty world with no registered components.
    pub fn new(seed: Seed) -> Self {
        World {
            entities: EntityAllocator::new(),
            stores: Vec::new(),
            by_type: HashMap::new(),
            rng: RngRegistry::new(seed),
        }
    }

    /// The master seed this world was created with.
    pub fn seed(&self) -> Seed {
        self.rng.master_seed()
    }

    /// Registers a component type. Must be called before any use of `T`;
    /// registration order is part of the save/hash format (ADR 0002 §6).
    pub fn register<T: Component>(&mut self) -> Result<(), EcsError> {
        if self.by_type.contains_key(&TypeId::of::<T>()) {
            return Err(EcsError::DuplicateComponent(T::NAME.to_owned()));
        }
        if self.stores.iter().any(|s| s.component_name() == T::NAME) {
            return Err(EcsError::DuplicateComponent(T::NAME.to_owned()));
        }
        self.by_type.insert(TypeId::of::<T>(), self.stores.len());
        self.stores.push(Box::new(StoreCell::<T>::new()));
        Ok(())
    }

    fn store_slot<T: Component>(&self) -> Result<usize, EcsError> {
        self.by_type
            .get(&TypeId::of::<T>())
            .copied()
            .ok_or(EcsError::UnregisteredComponent(T::NAME))
    }

    fn typed_store<T: Component>(&self) -> Result<&StoreCell<T>, EcsError> {
        let slot = self.store_slot::<T>()?;
        self.stores
            .get(slot)
            .and_then(|s| s.as_any().downcast_ref::<StoreCell<T>>())
            .ok_or(EcsError::InternalCorruption(
                "type map points at a store of a different type",
            ))
    }

    fn typed_store_mut<T: Component>(&mut self) -> Result<&mut StoreCell<T>, EcsError> {
        let slot = self.store_slot::<T>()?;
        self.stores
            .get_mut(slot)
            .and_then(|s| s.as_any_mut().downcast_mut::<StoreCell<T>>())
            .ok_or(EcsError::InternalCorruption(
                "type map points at a store of a different type",
            ))
    }

    // --- Entities -----------------------------------------------------

    /// Spawns a new live entity with no components.
    ///
    /// Direct use is for setup/loading code; systems mid-iteration must
    /// spawn through the [`crate::CommandBuffer`] (SPEC §6).
    pub fn spawn(&mut self) -> Entity {
        self.entities.spawn()
    }

    /// Despawns a live entity, removing its components from every store in
    /// registration order.
    pub fn despawn(&mut self, entity: Entity) -> Result<(), EcsError> {
        self.entities.despawn(entity)?;
        for store in &mut self.stores {
            store.remove_index(entity.index());
        }
        Ok(())
    }

    /// True iff the handle refers to a live entity.
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
    }

    /// Live entities in ascending index order.
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities.iter_alive()
    }

    /// Number of live entities.
    pub fn entity_count(&self) -> usize {
        self.entities.alive_count()
    }

    // --- Components ----------------------------------------------------

    /// Inserts a component on a live entity, returning the previous value if
    /// one was replaced. Errors on dead entities and unregistered types.
    pub fn insert<T: Component>(
        &mut self,
        entity: Entity,
        component: T,
    ) -> Result<Option<T>, EcsError> {
        if !self.entities.is_alive(entity) {
            return Err(EcsError::DeadEntity(entity));
        }
        let store = self.typed_store_mut::<T>()?;
        Ok(store.store.insert(entity.index(), component))
    }

    /// Removes a component from a live entity, returning it if present.
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Result<Option<T>, EcsError> {
        if !self.entities.is_alive(entity) {
            return Err(EcsError::DeadEntity(entity));
        }
        let store = self.typed_store_mut::<T>()?;
        Ok(store.store.remove(entity.index()))
    }

    /// Reads a component. `None` for dead entities or absent components;
    /// errors only on unregistered types.
    pub fn get<T: Component>(&self, entity: Entity) -> Result<Option<&T>, EcsError> {
        if !self.entities.is_alive(entity) {
            return Ok(None);
        }
        Ok(self.typed_store::<T>()?.store.get(entity.index()))
    }

    /// Mutably reads a component. Same semantics as [`World::get`].
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Result<Option<&mut T>, EcsError> {
        if !self.entities.is_alive(entity) {
            return Ok(None);
        }
        Ok(self.typed_store_mut::<T>()?.store.get_mut(entity.index()))
    }

    /// Iterates `(Entity, &T)` in ascending entity-index order (the
    /// canonical query order, SPEC §6).
    pub fn iter<T: Component>(&self) -> Result<ComponentIter<'_, T>, EcsError> {
        let store = self.typed_store::<T>()?;
        Ok(ComponentIter {
            inner: store.store.iter(),
            entities: &self.entities,
        })
    }

    /// Iterates `(Entity, &mut T)` in ascending entity-index order.
    pub fn iter_mut<T: Component>(&mut self) -> Result<ComponentIterMut<'_, T>, EcsError> {
        let slot = self.store_slot::<T>()?;
        let World {
            entities, stores, ..
        } = self;
        let store = stores
            .get_mut(slot)
            .and_then(|s| s.as_any_mut().downcast_mut::<StoreCell<T>>())
            .ok_or(EcsError::InternalCorruption(
                "type map points at a store of a different type",
            ))?;
        Ok(ComponentIterMut {
            inner: store.store.iter_mut(),
            entities,
        })
    }

    // --- RNG -------------------------------------------------------------

    /// The named deterministic RNG stream (SPEC §2). Never share one stream
    /// across systems.
    pub fn rng(&mut self, name: &str) -> &mut Pcg64 {
        self.rng.stream(name)
    }

    // --- Persistence & hashing (consumed by `persistence`/`sim_time`) ----

    /// Canonical component blobs `(NAME, bytes)` in registration order.
    pub fn component_blobs(&self) -> Result<Vec<(String, Vec<u8>)>, EcsError> {
        self.stores
            .iter()
            .map(|s| Ok((s.component_name().to_owned(), s.to_canonical_bytes()?)))
            .collect()
    }

    /// Restores component stores from save blobs. Strict two ways: every
    /// blob must match a registered store and every registered store must
    /// receive exactly one blob (ADR 0002 §6).
    pub fn load_component_blobs(&mut self, blobs: &[(String, Vec<u8>)]) -> Result<(), EcsError> {
        if blobs.len() != self.stores.len() {
            return Err(EcsError::ComponentBlobMismatch(format!(
                "save has {} component blobs, world registered {}",
                blobs.len(),
                self.stores.len()
            )));
        }
        for (store, (name, bytes)) in self.stores.iter_mut().zip(blobs) {
            if store.component_name() != name {
                return Err(EcsError::ComponentBlobMismatch(format!(
                    "save blob `{name}` does not match registered store `{}` at this position",
                    store.component_name()
                )));
            }
            store.load_canonical_bytes(bytes)?;
        }
        Ok(())
    }

    /// Canonical bytes of the entity allocator.
    pub fn entities_to_bytes(&self) -> Result<Vec<u8>, EcsError> {
        Ok(codec::to_bytes(&self.entities)?)
    }

    /// Restores the entity allocator from canonical bytes.
    pub fn restore_entities(&mut self, bytes: &[u8]) -> Result<(), EcsError> {
        self.entities = codec::from_bytes(bytes)?;
        Ok(())
    }

    /// Canonical bytes of the RNG registry (all stream states, exactly).
    pub fn rng_to_bytes(&self) -> Result<Vec<u8>, EcsError> {
        Ok(codec::to_bytes(&self.rng)?)
    }

    /// Restores the RNG registry from canonical bytes.
    pub fn restore_rng(&mut self, bytes: &[u8]) -> Result<(), EcsError> {
        self.rng = codec::from_bytes(bytes)?;
        Ok(())
    }

    /// Absorbs the full world state into `hasher` in canonical order:
    /// entity allocator, then each store (name + contents) in registration
    /// order, then RNG stream states. All fields are length-framed.
    pub fn hash_into(&self, hasher: &mut StableHasher) -> Result<(), EcsError> {
        hasher.write_frame(&self.entities_to_bytes()?);
        hasher.write_u64(self.stores.len() as u64);
        for store in &self.stores {
            store.hash_into(hasher)?;
        }
        hasher.write_frame(&self.rng_to_bytes()?);
        Ok(())
    }
}

/// Immutable component query yielding `(Entity, &T)` in ascending
/// entity-index order.
///
/// Invariant: components only exist for live entities, so every store index
/// resolves to a live handle; a store index without a live entity indicates
/// an ECS bug and is skipped (it would still surface in state hashes).
pub struct ComponentIter<'a, T: Component> {
    inner: StoreIter<'a, T>,
    entities: &'a EntityAllocator,
}

impl<'a, T: Component> Iterator for ComponentIter<'a, T> {
    type Item = (Entity, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        for (index, value) in self.inner.by_ref() {
            if let Some(entity) = self.entities.entity_at(index) {
                return Some((entity, value));
            }
        }
        None
    }
}

/// Mutable component query yielding `(Entity, &mut T)` in ascending
/// entity-index order. Same invariants as [`ComponentIter`].
pub struct ComponentIterMut<'a, T: Component> {
    inner: StoreIterMut<'a, T>,
    entities: &'a EntityAllocator,
}

impl<'a, T: Component> Iterator for ComponentIterMut<'a, T> {
    type Item = (Entity, &'a mut T);

    fn next(&mut self) -> Option<Self::Item> {
        for (index, value) in self.inner.by_ref() {
            if let Some(entity) = self.entities.entity_at(index) {
                return Some((entity, value));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StorageKind;
    use core_rng::RngCore;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Position(u64);
    impl Component for Position {
        const NAME: &'static str = "test.position";
        const STORAGE: StorageKind = StorageKind::Dense;
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Label(String);
    impl Component for Label {
        const NAME: &'static str = "test.label";
        const STORAGE: StorageKind = StorageKind::Sparse;
    }

    fn world_with_registrations() -> World {
        let mut w = World::new(Seed::new(1));
        w.register::<Position>().unwrap();
        w.register::<Label>().unwrap();
        w
    }

    #[test]
    fn insert_get_remove_lifecycle() {
        let mut w = world_with_registrations();
        let e = w.spawn();
        assert_eq!(w.insert(e, Position(5)).unwrap(), None);
        assert_eq!(w.insert(e, Position(6)).unwrap(), Some(Position(5)));
        assert_eq!(w.get::<Position>(e).unwrap(), Some(&Position(6)));
        assert_eq!(w.remove::<Position>(e).unwrap(), Some(Position(6)));
        assert_eq!(w.get::<Position>(e).unwrap(), None);
    }

    #[test]
    fn dead_entities_reject_mutation_and_read_as_absent() {
        let mut w = world_with_registrations();
        let e = w.spawn();
        w.insert(e, Position(1)).unwrap();
        w.despawn(e).unwrap();
        assert!(matches!(
            w.insert(e, Position(2)),
            Err(EcsError::DeadEntity(_))
        ));
        assert_eq!(w.get::<Position>(e).unwrap(), None);
    }

    #[test]
    fn despawn_clears_components_so_recycled_slots_start_clean() {
        let mut w = world_with_registrations();
        let e = w.spawn();
        w.insert(e, Position(9)).unwrap();
        w.insert(e, Label("old".into())).unwrap();
        w.despawn(e).unwrap();
        // Slot is recycled (LIFO) with a new generation; must carry nothing.
        let e2 = w.spawn();
        assert_eq!(e2.index(), e.index());
        assert_eq!(w.get::<Position>(e2).unwrap(), None);
        assert_eq!(w.get::<Label>(e2).unwrap(), None);
    }

    #[test]
    fn unregistered_component_is_a_typed_error() {
        #[derive(Debug, Serialize, Deserialize)]
        struct Never(u8);
        impl Component for Never {
            const NAME: &'static str = "test.never";
            const STORAGE: StorageKind = StorageKind::Sparse;
        }
        let mut w = world_with_registrations();
        let e = w.spawn();
        assert!(matches!(
            w.insert(e, Never(0)),
            Err(EcsError::UnregisteredComponent("test.never"))
        ));
    }

    #[test]
    fn duplicate_registration_is_a_typed_error() {
        let mut w = world_with_registrations();
        assert!(matches!(
            w.register::<Position>(),
            Err(EcsError::DuplicateComponent(_))
        ));
    }

    #[test]
    fn iteration_is_entity_index_order() {
        let mut w = world_with_registrations();
        let entities: Vec<Entity> = (0..6).map(|_| w.spawn()).collect();
        // Insert out of order.
        for &i in &[4usize, 0, 5, 2] {
            w.insert(entities[i], Position(i as u64)).unwrap();
        }
        let order: Vec<u32> = w
            .iter::<Position>()
            .unwrap()
            .map(|(e, _)| e.index())
            .collect();
        assert_eq!(order, vec![0, 2, 4, 5]);
    }

    #[test]
    fn blob_round_trip_restores_exact_state_and_hash() {
        let mut w = world_with_registrations();
        let a = w.spawn();
        let b = w.spawn();
        w.insert(a, Position(1)).unwrap();
        w.insert(b, Position(2)).unwrap();
        w.insert(b, Label("b".into())).unwrap();
        w.despawn(a).unwrap();
        let _ = w.rng("test.stream").next_u64();

        let blobs = w.component_blobs().unwrap();
        let entities = w.entities_to_bytes().unwrap();
        let rng = w.rng_to_bytes().unwrap();

        let mut restored = world_with_registrations();
        restored.restore_entities(&entities).unwrap();
        restored.load_component_blobs(&blobs).unwrap();
        restored.restore_rng(&rng).unwrap();

        let mut h1 = StableHasher::new();
        w.hash_into(&mut h1).unwrap();
        let mut h2 = StableHasher::new();
        restored.hash_into(&mut h2).unwrap();
        assert_eq!(h1.finish(), h2.finish());
    }

    #[test]
    fn blob_mismatch_is_strict_both_ways() {
        let w = world_with_registrations();
        let blobs = w.component_blobs().unwrap();

        // World registered fewer components than the save.
        let mut fewer = World::new(Seed::new(1));
        fewer.register::<Position>().unwrap();
        assert!(matches!(
            fewer.load_component_blobs(&blobs),
            Err(EcsError::ComponentBlobMismatch(_))
        ));

        // Save with a renamed blob.
        let mut renamed = blobs.clone();
        renamed[0].0 = "test.other".into();
        let mut w2 = world_with_registrations();
        assert!(matches!(
            w2.load_component_blobs(&renamed),
            Err(EcsError::ComponentBlobMismatch(_))
        ));
    }
}
