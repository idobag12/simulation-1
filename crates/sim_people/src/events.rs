//! People-simulation events: facts, not commands (SPEC §7).

use core_ecs::{Entity, Event};
use serde::{Deserialize, Serialize};

/// Why a citizen died.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeathCause {
    /// Age-band mortality draw (the only cause in Phase 2).
    OldAge,
}

/// A citizen died (the SPEC §7 exemplar fact). Emitted by
/// [`crate::MortalitySystem`] on the tick of death; the entity is
/// despawned in the same tick's command buffer, so consumers reading this
/// event next tick must treat `person` as a historical handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonDied {
    /// The deceased (stale handle by delivery time).
    pub person: Entity,
    /// Cause of death.
    pub cause: DeathCause,
}

impl Event for PersonDied {
    const NAME: &'static str = "people.person_died";
}
