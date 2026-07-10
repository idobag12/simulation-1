//! Simulation level-of-detail tiers and promotion/demotion (SPEC §10).
//!
//! Intentionally empty (Phase 8, ADR 0011 §9): the tier systems live
//! in `sim_ai` — tier execution IS agent behavior, and Tier B/C reuse
//! `sim_ai`-private machinery (`purchase_unit`, `CurrentAction`,
//! `DailyPlan`) that SPEC §4's no-`sim_→sim_`-calls rule would
//! otherwise force into the shared interface. The shell remains so the
//! workspace layout matches SPEC §4; a future phase may migrate the
//! assignment layer here if it ever stops needing `sim_ai` internals.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
