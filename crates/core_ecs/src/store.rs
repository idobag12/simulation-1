//! Typed component stores: dense `Vec<Option<T>>` or sparse
//! `BTreeMap<u32, T>` per component (SPEC §6), behind one canonical
//! serialized form.

use std::any::Any;
use std::collections::BTreeMap;

use core_types::StableHasher;
use core_types::codec;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::EcsError;

/// Storage layout for a component type. Chosen per component and documented
/// at the component's definition site (SPEC §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageKind {
    /// `Vec<Option<T>>` indexed by entity index. For components most live
    /// entities carry.
    Dense,
    /// `BTreeMap<u32, T>`. For components only a small subset of entities
    /// carry.
    Sparse,
}

/// A component type.
///
/// Invariants:
/// - `NAME` is the component's identity in saves and hashes. It is stable
///   forever: renaming it (or reusing it for a different type) is a
///   save-format break requiring a migration (ADR 0002 §6). Rust type names
///   never appear in the format.
/// - `STORAGE` may change between versions without breaking saves, because
///   both layouts share one canonical serialized form.
pub trait Component: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// Stable, namespaced identity, e.g. `"people.needs"`.
    const NAME: &'static str;
    /// Storage layout; the choice is documented where the component is
    /// defined.
    const STORAGE: StorageKind;
}

/// The typed storage for one component type.
///
/// Invariants:
/// - Keys are entity indices; a component exists only for live entities
///   (despawn removes from every store).
/// - Iteration is in ascending entity-index order for both layouts.
/// - Serializes to the canonical ascending `Vec<(u32, T)>` of live pairs,
///   independent of layout, capacity, or trailing-`None` padding
///   (ADR 0002 §6).
#[derive(Debug)]
pub(crate) enum TypedStore<T> {
    /// Dense layout.
    Dense(Vec<Option<T>>),
    /// Sparse layout.
    Sparse(BTreeMap<u32, T>),
}

impl<T: Component> TypedStore<T> {
    pub(crate) fn new() -> Self {
        match T::STORAGE {
            StorageKind::Dense => TypedStore::Dense(Vec::new()),
            StorageKind::Sparse => TypedStore::Sparse(BTreeMap::new()),
        }
    }

    /// Inserts, returning the previous value if any.
    pub(crate) fn insert(&mut self, index: u32, value: T) -> Option<T> {
        match self {
            TypedStore::Dense(slots) => {
                let slot = index as usize;
                if slot >= slots.len() {
                    slots.resize_with(slot + 1, || None);
                }
                match slots.get_mut(slot) {
                    Some(cell) => cell.replace(value),
                    // Unreachable: just resized to cover `slot`.
                    None => None,
                }
            }
            TypedStore::Sparse(map) => map.insert(index, value),
        }
    }

    pub(crate) fn remove(&mut self, index: u32) -> Option<T> {
        match self {
            TypedStore::Dense(slots) => slots.get_mut(index as usize).and_then(Option::take),
            TypedStore::Sparse(map) => map.remove(&index),
        }
    }

    pub(crate) fn get(&self, index: u32) -> Option<&T> {
        match self {
            TypedStore::Dense(slots) => slots.get(index as usize).and_then(Option::as_ref),
            TypedStore::Sparse(map) => map.get(&index),
        }
    }

    pub(crate) fn get_mut(&mut self, index: u32) -> Option<&mut T> {
        match self {
            TypedStore::Dense(slots) => slots.get_mut(index as usize).and_then(Option::as_mut),
            TypedStore::Sparse(map) => map.get_mut(&index),
        }
    }

    /// Iterates `(entity index, &component)` in ascending index order.
    pub(crate) fn iter(&self) -> StoreIter<'_, T> {
        match self {
            TypedStore::Dense(slots) => StoreIter::Dense(slots.iter().enumerate()),
            TypedStore::Sparse(map) => StoreIter::Sparse(map.iter()),
        }
    }

    /// Iterates `(entity index, &mut component)` in ascending index order.
    pub(crate) fn iter_mut(&mut self) -> StoreIterMut<'_, T> {
        match self {
            TypedStore::Dense(slots) => StoreIterMut::Dense(slots.iter_mut().enumerate()),
            TypedStore::Sparse(map) => StoreIterMut::Sparse(map.iter_mut()),
        }
    }

    /// Canonical form: ascending live pairs (ADR 0002 §6).
    fn to_canonical_pairs(&self) -> Vec<(u32, &T)> {
        self.iter().collect()
    }

    fn from_canonical_pairs(pairs: Vec<(u32, T)>) -> Self {
        match T::STORAGE {
            StorageKind::Dense => {
                let len = pairs
                    .iter()
                    .map(|&(i, _)| i as usize + 1)
                    .max()
                    .unwrap_or(0);
                let mut slots: Vec<Option<T>> = Vec::new();
                slots.resize_with(len, || None);
                for (index, value) in pairs {
                    if let Some(cell) = slots.get_mut(index as usize) {
                        *cell = Some(value);
                    }
                }
                TypedStore::Dense(slots)
            }
            StorageKind::Sparse => TypedStore::Sparse(pairs.into_iter().collect()),
        }
    }
}

/// Immutable store iterator in ascending entity-index order.
pub(crate) enum StoreIter<'a, T> {
    Dense(std::iter::Enumerate<std::slice::Iter<'a, Option<T>>>),
    Sparse(std::collections::btree_map::Iter<'a, u32, T>),
}

impl<'a, T> Iterator for StoreIter<'a, T> {
    type Item = (u32, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            StoreIter::Dense(inner) => {
                for (index, slot) in inner.by_ref() {
                    if let Some(value) = slot.as_ref() {
                        return Some((index as u32, value));
                    }
                }
                None
            }
            StoreIter::Sparse(inner) => inner.next().map(|(&index, value)| (index, value)),
        }
    }
}

/// Mutable store iterator in ascending entity-index order.
pub(crate) enum StoreIterMut<'a, T> {
    Dense(std::iter::Enumerate<std::slice::IterMut<'a, Option<T>>>),
    Sparse(std::collections::btree_map::IterMut<'a, u32, T>),
}

impl<'a, T> Iterator for StoreIterMut<'a, T> {
    type Item = (u32, &'a mut T);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            StoreIterMut::Dense(inner) => {
                for (index, slot) in inner.by_ref() {
                    if let Some(value) = slot.as_mut() {
                        return Some((index as u32, value));
                    }
                }
                None
            }
            StoreIterMut::Sparse(inner) => inner.next().map(|(&index, value)| (index, value)),
        }
    }
}

/// Object-safe view of a store used by the world for save/hash/despawn
/// plumbing. Downcast via `as_any` for typed access.
pub(crate) trait ErasedStore: Send + Sync {
    /// The component's stable `NAME`.
    fn component_name(&self) -> &'static str;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Removes the component for `index` if present (used by despawn).
    fn remove_index(&mut self, index: u32);
    /// Canonical bytes (ADR 0002 §6) for saving and hashing.
    fn to_canonical_bytes(&self) -> Result<Vec<u8>, EcsError>;
    /// Replaces contents from canonical bytes.
    fn load_canonical_bytes(&mut self, bytes: &[u8]) -> Result<(), EcsError>;
    /// Absorbs `NAME` and canonical bytes into the world hash, length-framed.
    fn hash_into(&self, hasher: &mut StableHasher) -> Result<(), EcsError>;
}

/// The concrete erased wrapper around a [`TypedStore`].
pub(crate) struct StoreCell<T: Component> {
    pub(crate) store: TypedStore<T>,
}

impl<T: Component> StoreCell<T> {
    pub(crate) fn new() -> Self {
        StoreCell {
            store: TypedStore::new(),
        }
    }
}

impl<T: Component> ErasedStore for StoreCell<T> {
    fn component_name(&self) -> &'static str {
        T::NAME
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn remove_index(&mut self, index: u32) {
        let _ = self.store.remove(index);
    }

    fn to_canonical_bytes(&self) -> Result<Vec<u8>, EcsError> {
        Ok(codec::to_bytes(&self.store.to_canonical_pairs())?)
    }

    fn load_canonical_bytes(&mut self, bytes: &[u8]) -> Result<(), EcsError> {
        let pairs: Vec<(u32, T)> = codec::from_bytes(bytes)?;
        self.store = TypedStore::from_canonical_pairs(pairs);
        Ok(())
    }

    fn hash_into(&self, hasher: &mut StableHasher) -> Result<(), EcsError> {
        hasher.write_frame(T::NAME.as_bytes());
        hasher.write_frame(&self.to_canonical_bytes()?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct DenseC(u64);
    impl Component for DenseC {
        const NAME: &'static str = "test.dense";
        const STORAGE: StorageKind = StorageKind::Dense;
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct SparseC(u64);
    impl Component for SparseC {
        const NAME: &'static str = "test.sparse";
        const STORAGE: StorageKind = StorageKind::Sparse;
    }

    #[test]
    fn iteration_is_in_index_order_regardless_of_insert_order() {
        let mut dense: TypedStore<DenseC> = TypedStore::new();
        let mut sparse: TypedStore<SparseC> = TypedStore::new();
        for &i in &[5u32, 1, 9, 0, 3] {
            dense.insert(i, DenseC(u64::from(i)));
            sparse.insert(i, SparseC(u64::from(i)));
        }
        let d: Vec<u32> = dense.iter().map(|(i, _)| i).collect();
        let s: Vec<u32> = sparse.iter().map(|(i, _)| i).collect();
        assert_eq!(d, vec![0, 1, 3, 5, 9]);
        assert_eq!(s, vec![0, 1, 3, 5, 9]);
    }

    #[test]
    fn canonical_bytes_are_layout_and_padding_independent() {
        // A dense store that grew (trailing None padding after removal) must
        // serialize identically to one that never grew.
        let mut grown: TypedStore<DenseC> = TypedStore::new();
        grown.insert(0, DenseC(10));
        grown.insert(7, DenseC(70));
        grown.remove(7);

        let mut compact: TypedStore<DenseC> = TypedStore::new();
        compact.insert(0, DenseC(10));

        let a = StoreCell { store: grown }.to_canonical_bytes().unwrap();
        let b = StoreCell { store: compact }.to_canonical_bytes().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn canonical_round_trip_preserves_contents() {
        let mut cell: StoreCell<SparseC> = StoreCell::new();
        cell.store.insert(3, SparseC(33));
        cell.store.insert(1, SparseC(11));
        let bytes = cell.to_canonical_bytes().unwrap();

        let mut restored: StoreCell<SparseC> = StoreCell::new();
        restored.load_canonical_bytes(&bytes).unwrap();
        assert_eq!(restored.store.get(1), Some(&SparseC(11)));
        assert_eq!(restored.store.get(3), Some(&SparseC(33)));
        assert_eq!(restored.store.iter().count(), 2);
        assert_eq!(restored.to_canonical_bytes().unwrap(), bytes);
    }
}
