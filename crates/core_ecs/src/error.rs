//! Typed ECS errors (SPEC §3: no unwrap/expect in simulation code).

use core_events::EventError;
use core_types::ArithmeticError;
use core_types::codec::CodecError;
use thiserror::Error;

use crate::entity::Entity;

/// Errors produced by ECS operations, schedules, and systems.
#[derive(Debug, Error)]
pub enum EcsError {
    /// An operation targeted an entity that is not alive (stale generation
    /// or already despawned).
    #[error("entity {0:?} is not alive")]
    DeadEntity(Entity),

    /// A component type was used before being registered with the world.
    #[error("component `{0}` is not registered")]
    UnregisteredComponent(&'static str),

    /// A component name or type was registered twice.
    #[error("component `{0}` is already registered")]
    DuplicateComponent(String),

    /// A save blob referenced a component name the world has not registered,
    /// or a registered component was missing from the save (strict both
    /// ways, ADR 0002 §6).
    #[error("save/world component mismatch: {0}")]
    ComponentBlobMismatch(String),

    /// The world's internal store index diverged from its type map. This
    /// indicates a bug in `core_ecs` itself, surfaced as an error instead of
    /// a panic.
    #[error("internal store corruption: {0}")]
    InternalCorruption(&'static str),

    /// A simulation invariant that must never break was found broken — a
    /// conserved quantity drifted or a ledger stopped balancing (SPEC §12:
    /// auditors halt the run, debug and release; a violation is never
    /// ignorable). The message names the identity and the diff.
    #[error("invariant violation: {0}")]
    InvariantViolation(String),

    /// Canonical encoding/decoding failed.
    #[error(transparent)]
    Codec(#[from] CodecError),

    /// The event system failed (unregistered type, registration mismatch).
    #[error(transparent)]
    Event(#[from] EventError),

    /// Checked arithmetic overflowed inside a system or the tick loop.
    #[error(transparent)]
    Arithmetic(#[from] ArithmeticError),

    /// A system failed; wraps the underlying error with the system's name so
    /// tick-loop failures are attributable.
    #[error("system `{system}` failed: {source}")]
    System {
        /// `System::name()` of the failing system.
        system: &'static str,
        /// The underlying failure.
        source: Box<EcsError>,
    },
}
