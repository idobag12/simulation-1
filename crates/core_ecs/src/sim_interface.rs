//! Shared simulation components (SPEC §4): the ONLY place components used
//! by more than one `sim_` crate live. Sim crates never depend on each
//! other; they meet here (and on the event bus).
//!
//! Moved here in Phase 3 (ADR 0006 §1) with component `NAME`s unchanged —
//! a crate move is invisible to saves and hashes.

use serde::{Deserialize, Serialize};

use crate::entity::Entity;
use crate::store::{Component, StorageKind};

#[path = "sim_interface_econ.rs"]
mod econ;
pub use econ::{
    EconCounters, Employment, Fired, FiredReason, FirmBooks, GoodsPurchased, Hired, Inventory,
    LaborStats, PriceChanged, RetailOffer, Wallet,
};
#[path = "sim_interface_money.rs"]
mod money;
pub use money::{
    BankBook, BorrowerStatus, HomeBuilt, HomeSold, HousingBook, Loan, LoanDefaulted, LoanGranted,
    Ownership, TaxCollected, TaxKind, Tenancy, TenancyStarted, TreasuryBook,
};
#[path = "sim_interface_social.rs"]
mod social;
pub use social::{
    Beliefs, Born, Edge, Married, RelKind, Relationships, SchoolAge, SchoolAttended, Skills,
};
#[path = "sim_interface_lod.rs"]
mod lod;
pub use lod::{DayModel, LodTier, Spotlight, Tier, TierChanged};

/// One need level in per-million units (ADR 0005 §2): 0 = fully depleted,
/// [`NeedLevel::MAX`] = fully satisfied.
///
/// Invariants: always within `0..=1_000_000`; all mutation goes through
/// the clamping constructors/operations, so an out-of-range level cannot
/// exist. Serializes transparently as its inner `i64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NeedLevel(i64);

impl NeedLevel {
    /// Fully satisfied (per-million scale). Unit definition, not a tunable.
    pub const MAX: NeedLevel = NeedLevel(1_000_000);
    /// Fully depleted.
    pub const ZERO: NeedLevel = NeedLevel(0);

    /// Constructs a level, clamping into the valid range.
    pub const fn new_clamped(raw: i64) -> NeedLevel {
        if raw < 0 {
            NeedLevel::ZERO
        } else if raw > NeedLevel::MAX.0 {
            NeedLevel::MAX
        } else {
            NeedLevel(raw)
        }
    }

    /// The raw per-million value (always in range).
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Decays by `amount` per-million units, clamping at zero (exact
    /// integer arithmetic; SPEC §2).
    pub fn decay(self, amount: i64) -> NeedLevel {
        NeedLevel::new_clamped(self.0.saturating_sub(amount))
    }

    /// Gains `amount` per-million units, clamping at [`NeedLevel::MAX`]
    /// (exact integer arithmetic; SPEC §2).
    pub fn gain(self, amount: i64) -> NeedLevel {
        NeedLevel::new_clamped(self.0.saturating_add(amount))
    }
}

/// A citizen's need levels, one per data-defined need, in data order
/// (ADR 0005 §§2–3).
///
/// Invariant: the vector's length and order equal the loaded needs config
/// (validated at load; guarded by the systems that interpret it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Needs {
    /// Need levels, data order.
    pub levels: Vec<NeedLevel>,
}

impl Component for Needs {
    const NAME: &'static str = "people.needs";
    // Dense: every citizen carries it.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// A citizen's personality-trait weights, one per data-defined trait, in
/// data order. Weights are per-mille (ADR 0005 §2); they weight utility
/// scoring (SPEC §11) — how two citizens in identical situations behave
/// differently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Personality {
    /// Trait weights in per-mille units, data order.
    pub weights: Vec<i16>,
}

impl Component for Personality {
    const NAME: &'static str = "people.personality";
    // Dense: every citizen carries it.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// Where a mobile entity currently is: the location entity it last
/// occupied (travel-in-progress is `sim_ai`'s `CurrentAction` state; the
/// position updates on arrival).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// The location entity occupied.
    pub at: Entity,
}

impl Component for Position {
    const NAME: &'static str = "world.position";
    // Dense: every citizen carries it.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// A place in the abstract location graph (no map until Phase 9 — travel
/// between any two locations costs the same data-defined time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// Index into the data-defined location-kind list (data order — the
    /// SPEC §8 stable-integer-id pattern).
    pub kind: u32,
}

impl Component for Location {
    const NAME: &'static str = "world.location";
    // Sparse: locations are a small minority of entities.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// A citizen's home (their household's dwelling): private locations are
/// only afforded to their residents (SPEC §11 "afforded by location").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Residence {
    /// The home location entity.
    pub home: Entity,
}

impl Component for Residence {
    const NAME: &'static str = "world.residence";
    // Dense: every citizen carries it.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// Marks a citizen as at-or-above working age (Phase 5, ADR 0008 §2):
/// maintained by `sim_people` (stamped at genesis, promoted on the
/// birthday crossing the data-defined threshold), read by the labor
/// market — age itself stays in `sim_people`'s `Identity` (SPEC §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingAge;

impl Component for WorkingAge {
    const NAME: &'static str = "people.working_age";
    // Sparse: a subset of citizens.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn need_levels_clamp_at_both_ends() {
        assert_eq!(NeedLevel::new_clamped(-5), NeedLevel::ZERO);
        assert_eq!(NeedLevel::new_clamped(2_000_000), NeedLevel::MAX);
        assert_eq!(NeedLevel::new_clamped(37).raw(), 37);
        assert_eq!(NeedLevel::new_clamped(100).decay(150), NeedLevel::ZERO);
        assert_eq!(NeedLevel::new_clamped(100).decay(40).raw(), 60);
        assert_eq!(NeedLevel::new_clamped(999_999).gain(500), NeedLevel::MAX);
        assert_eq!(NeedLevel::new_clamped(10).gain(5).raw(), 15);
        // Serializes transparently as the inner i64 (save-format identity).
        assert_eq!(
            core_types::codec::to_bytes(&NeedLevel::new_clamped(42)).unwrap(),
            core_types::codec::to_bytes(&42i64).unwrap()
        );
    }
}
