//! The LOD layer's shared components and events (Phase 8, ADR 0011):
//! tier membership, the statistical day model, and the tier-change
//! fact. Split file for the SPEC §3 module-size rule.

use serde::{Deserialize, Serialize};

use crate::entity::Entity;
use crate::store::{Component, StorageKind};

/// A citizen's simulation level of detail (SPEC §10). Append-only enum
/// (persisted): every citizen is always REAL — identity, ledger,
/// employment, bonds — the tier only chooses how their behavior is
/// integrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    /// Embodied: full per-tick utility AI.
    A,
    /// Scheduled: per-hour block execution of the same obligations.
    B,
    /// Statistical: per-day `DayModel` integration.
    C,
}

/// Tier membership (ADR 0011 §1), assigned deterministically each day.
/// A MISSING row means Tier A — migrated pre-v9 citizens keep exactly
/// their old embodied behavior until the first assignment pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LodTier {
    /// The current tier.
    pub tier: Tier,
}

impl Component for LodTier {
    const NAME: &'static str = "lod.tier";
    // Dense: every citizen carries one once assignment has run.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// The statistical day model (SPEC §10 Tier C; ADR 0011 §3): per-need
/// passive daily gain from the places the citizen's day passes through
/// (home over the sleep window, a leisure venue over the data leisure
/// block). The LIVE `Needs` row remains the trajectory's state — this
/// model never shadows it, and it holds NO money and NO inventory
/// (wallets stay real at every tier; that is what makes A↔C cycling
/// conserve money exactly).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DayModel {
    /// Per need (data order): passive satisfaction per day, per-million.
    pub passive_gain_per_day: Vec<i64>,
}

impl Component for DayModel {
    const NAME: &'static str = "lod.day_model";
    // Sparse: only demoted (B/C) citizens carry one.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// A citizen changed simulation tier (a fact, for observability).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierChanged {
    /// The citizen.
    pub citizen: Entity,
    /// The tier left.
    pub from: Tier,
    /// The tier entered.
    pub to: Tier,
}

impl crate::Event for TierChanged {
    const NAME: &'static str = "lod.tier_changed";
}

/// A citizen was touched by a high-signal fact and is pinned to the
/// embodied tier until `until_tick` (ADR 0011 §1): written by the
/// tick-rate spotlight reader from the event bus, read by the daily
/// assignment, dropped on expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spotlight {
    /// Last tick (exclusive) the pin holds.
    pub until_tick: u64,
}

impl Component for Spotlight {
    const NAME: &'static str = "lod.spotlight";
    // Sparse: a handful of citizens are newsworthy at a time.
    const STORAGE: StorageKind = StorageKind::Sparse;
}
