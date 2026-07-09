//! Utility AI core (SPEC §11, §15 Phase 3; design in ADR 0006): the
//! action framework, need-based float scoring weighted by personality,
//! sleep schedules as a scoring bias, and inspectable decision dumps.
//! Planners with goals, memory, and relationships arrive in Phase 7.
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

pub use components::{CandidateAction, CurrentAction, DailyPlan, LastDecision, ScoredCandidate};
pub use config::{AiConfig, AiTables};
pub use systems::{ActSystem, DecideSystem, PlanSystem};
