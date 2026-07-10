//! The egui viewer (Phase 10, ADR 0013): a read-only window onto a
//! running simulation, split hard into a TOOLING layer (the snapshot
//! protocol and pane builders — plain data, unit-tested, carrying the
//! exit criterion) and a PAINT layer (`app`, egui) that only draws.
//!
//! Dependency rule (SPEC §4): `viewer → headless → sim_* → core_*`,
//! consuming snapshots only — `Snapshot::capture` is the single
//! function that touches `World`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod app;
pub mod events;
pub mod panes;
pub mod snapshot;
