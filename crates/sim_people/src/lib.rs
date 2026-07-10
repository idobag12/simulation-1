//! Citizens (SPEC §15 Phase 2): identity, traits, needs with decay,
//! lifecycle (age, deterministic mortality), households, name generation.
//! No AI yet — needs decay and people age (design in ADR 0005).
//!
//! Invariants owned by this crate:
//! - Everything persisted is fixed-point integers (SPEC §2): need levels in
//!   per-million units, traits in per-mille, death chances in per-billion.
//! - Ages are never stored; they derive from `birth_tick` and the current
//!   tick (ADR 0005 §2).
//! - Every random draw comes from named `people.*` streams; all iteration
//!   is in entity-index or data order (SPEC §3).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod components;
pub mod config;
pub mod events;
pub mod genesis;
pub mod systems;

pub use components::{Household, HouseholdMember, Identity, NeedLevel, Needs, Personality, Sex};
pub use events::{DeathCause, PersonDied};
pub use systems::{MortalitySystem, NeedsDecaySystem, WorkingAgeSystem, validate_town};
