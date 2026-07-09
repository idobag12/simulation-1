//! Invariant auditors (SPEC §12 "Auditor (permanent)", §13; ADR 0007 §5):
//! recompute the conservation identities from scratch every day and halt
//! the run — debug AND release — on any drift. Inspector queries and the
//! metrics registry join in later phases.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod audit;

pub use audit::{AuditSystem, audit_economy};
