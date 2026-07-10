//! Utility AI core (SPEC §11, §15 Phase 3; design in ADR 0006): the
//! action framework, need-based float scoring weighted by personality,
//! sleep schedules as a scoring bias, and inspectable decision dumps.
//! Phase 7 (ADR 0010): the relationship graph drifts by co-presence,
//! gossip spreads price beliefs, and school attendance raises skills.
//!
//! Invariants owned by this crate:
//! - Floats appear ONLY inside scoring (`+ − × ÷` and comparisons — no
//!   transcendental functions, so results are IEEE-754-exact on every
//!   platform); every persisted value and every action effect is exact
//!   integer arithmetic (SPEC §2/§11).
//! - Ties break by candidate enumeration order, which is location-entity
//!   order × satisfier data order (deterministic, SPEC §11).
//! - No RNG: decisions are pure functions of world state.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod components;
pub mod config;
pub mod systems;
pub mod systems_lod;
pub mod systems_social;

pub use components::{CandidateAction, CurrentAction, DailyPlan, LastDecision, ScoredCandidate};
pub use config::{AiConfig, AiTables, LodConfig, SocialConfig};
pub use systems::{ActSystem, DecideSystem, PlanSystem};
pub use systems_lod::{
    SpotlightSystem, TierAssignSystem, TierBSystem, TierCSystem, demote_all_to_c,
};
pub use systems_social::{RelationshipDecaySystem, SocialDriftSystem};
