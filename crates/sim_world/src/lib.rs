//! The town's places (SPEC §15 Phase 3 slice): locations as abstract
//! nodes — no map, no coordinates, no travel topology until Phase 9
//! (ADR 0006 §2). Buildings, housing markets, and pathing arrive in
//! Phases 6/9.
//!
//! The shared `Location`/`Position`/`Residence` components live in
//! `core_ecs::sim_interface` (`sim_ai` reads them; SPEC §4).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod config;
pub mod genesis;
