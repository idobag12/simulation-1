//! The thin custom ECS (SPEC §6): generational entities, per-component
//! dense/sparse stores, an explicitly ordered schedule, and a command
//! buffer.
//!
//! Invariants owned by this crate:
//! - Iteration is always in entity-index order or registration order —
//!   never in `HashMap` order (SPEC §3).
//! - Component identity in saves/hashes is the explicit `Component::NAME`,
//!   never a compiler-generated type name (ADR 0002 §6).
//! - Mid-iteration structural mutation is impossible: systems queue
//!   spawns/despawns/inserts/removes into a [`CommandBuffer`] that the
//!   schedule applies after each system runs (SPEC §6).
//! - No `unsafe` (stricter than SPEC §3, which would permit it here).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod command;
mod entity;
mod error;
mod store;
mod system;
mod world;

pub use command::CommandBuffer;
pub use entity::{Entity, EntityAllocator};
pub use error::EcsError;
pub use store::{Component, StorageKind};
pub use system::{Schedule, System, TickContext};
pub use world::{ComponentIter, ComponentIterMut, World};
