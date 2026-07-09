//! Economy-owned components (ADR 0007 §§1, 5): the firm identity and its
//! running production batch. Components other sim crates touch (wallets,
//! inventories, books, offers, counters) live in
//! `core_ecs::sim_interface` instead (SPEC §4).

use core_ecs::{Component, StorageKind};
use core_types::Money;
use serde::{Deserialize, Serialize};

/// A firm: which data-defined kind it is, which recipe it runs, and the
/// one price it currently posts for its output good (SPEC §12 posted
/// prices; written daily by `PricingSystem`, read by `TradeSystem` and
/// mirrored into the firm's `RetailOffer` in the same pricing call).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Firm {
    /// Index into the data-defined firm-kind list (data order = stable id).
    pub kind: u32,
    /// Index into the data-defined recipe list.
    pub recipe: u32,
    /// Posted unit price for the recipe's output good, in mills.
    pub posted_price: Money,
}

impl Component for Firm {
    const NAME: &'static str = "econ.firm";
    // Sparse: firms are a small minority of entities.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// A production batch in progress (present = running; absent = the firm
/// starts one when inputs allow). Inputs were consumed — and counted —
/// when the batch started; outputs land when it finishes (ADR 0007 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Production {
    /// Hours until the batch completes (counts down at hour rate).
    pub remaining_hours: u32,
}

impl Component for Production {
    const NAME: &'static str = "econ.production";
    // Sparse: firms only.
    const STORAGE: StorageKind = StorageKind::Sparse;
}
