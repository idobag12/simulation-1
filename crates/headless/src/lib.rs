//! Headless runner library (SPEC §13): world assembly, deterministic runs
//! with periodic hashing, and the Phase 0 determinism fixture. The CLI
//! binary (`embervale`) and the cross-crate determinism suite both consume
//! this crate, so tests exercise exactly what the tool runs.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod cli;
pub mod fixture;
pub mod runner;
