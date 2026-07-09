//! Economy components and events shared between sim crates (Phase 4,
//! ADR 0007): money and goods move through `sim_economy`, `sim_goods`,
//! `sim_ai` (purchases), and `debug_tools` (auditors), so their carriers
//! live here (SPEC §4). Split from `sim_interface.rs` for the SPEC §3
//! module-size rule.

use core_types::Money;
use serde::{Deserialize, Serialize};

use crate::entity::Entity;
use crate::store::{Component, StorageKind};

/// Cash on hand (ADR 0007 §2). Carried by citizens and firms; every
/// movement is a transfer inside one atomic transaction (ADR 0007 §4) —
/// the only money *source* is genesis seeding, recorded as
/// [`EconCounters::issued`].
///
/// Invariant: never negative in Phase 4 — every spend is bounded by the
/// payer's balance before it executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wallet {
    /// Balance in mills.
    pub cash: Money,
}

impl Component for Wallet {
    const NAME: &'static str = "econ.wallet";
    // Dense: every citizen carries one.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// Goods on hand: one exact integer quantity per data-defined good, in
/// data order (the SPEC §8 stable-integer-id pattern, like `Needs`).
/// Only firms carry inventories in Phase 4 — citizen purchases are
/// consumed at the point of sale (ADR 0007 §6).
///
/// Invariant: the vector's length equals the loaded goods config;
/// quantities never go negative (every removal is bounded by stock).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    /// Quantity per good, data order.
    pub quantities: Vec<i64>,
}

impl Inventory {
    /// The stock of `good`, treating an out-of-range index as zero (a
    /// mismatch with the loaded goods list is caught by validation).
    pub fn stock(&self, good: u32) -> i64 {
        self.quantities.get(good as usize).copied().unwrap_or(0)
    }
}

impl Component for Inventory {
    const NAME: &'static str = "goods.inventory";
    // Sparse: only firm entities carry one in Phase 4.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// A firm's citizen-facing sale (ADR 0007 §6), carried by retail firm
/// entities beside their `Location`. `sim_ai` reads it to score and
/// execute purchases; `sim_economy`'s pricing writes `unit_price`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetailOffer {
    /// The good sold (data-order index).
    pub good: u32,
    /// The need one unit satisfies (data-order index).
    pub need_index: u32,
    /// Need satisfaction per unit, per-million (applied once, clamped).
    pub gain_per_unit: i64,
    /// Ticks the buyer spends consuming the unit after purchase.
    pub use_ticks: u32,
    /// Current posted unit price (updated daily by pricing).
    pub unit_price: Money,
}

impl Component for RetailOffer {
    const NAME: &'static str = "econ.retail_offer";
    // Sparse: a handful of retail firms.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// Phase 4's honest double-entry slice (ADR 0007 §3): enough accounts to
/// make `cash − initial_cash == revenue − expenses` testable per firm,
/// every day. Shared because the purchase side is written by `sim_ai`'s
/// act system. The full account tree arrives with banking (Phase 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmBooks {
    /// Cash the firm was seeded with at genesis (never changes).
    pub initial_cash: Money,
    /// Lifetime sales income.
    pub revenue: Money,
    /// Lifetime purchase spending.
    pub expenses: Money,
}

impl Component for FirmBooks {
    const NAME: &'static str = "econ.firm_books";
    // Sparse: firms only.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// The conservation ledger (ADR 0007 §§2, 4): one instance on a dedicated
/// entity per world with an economy. Every system that creates, destroys,
/// or consumes money/goods updates it in the same call that mutates the
/// stores, so the audit identities are exact at all times:
/// `Σ wallets == issued` and, per good,
/// `Σ inventories == produced − consumed_by_citizens − consumed_in_production − spoiled`.
///
/// Genesis seeding is the explicit modeled source for both money
/// (`issued`) and goods (seeded stock is recorded in `produced` at
/// creation), so the identities hold from tick 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconCounters {
    /// All money ever created (genesis seeding; Phase 6 adds the bank).
    pub issued: Money,
    /// Per good (data order): units ever produced, incl. genesis stock.
    pub produced: Vec<i64>,
    /// Per good: units bought and consumed by citizens.
    pub consumed_by_citizens: Vec<i64>,
    /// Per good: units consumed as recipe inputs.
    pub consumed_in_production: Vec<i64>,
    /// Per good: units lost to spoilage (the explicit sink).
    pub spoiled: Vec<i64>,
}

impl EconCounters {
    /// A zeroed ledger for `goods` goods.
    pub fn new(goods: usize) -> Self {
        EconCounters {
            issued: Money::ZERO,
            produced: vec![0; goods],
            consumed_by_citizens: vec![0; goods],
            consumed_in_production: vec![0; goods],
            spoiled: vec![0; goods],
        }
    }
}

impl Component for EconCounters {
    const NAME: &'static str = "econ.counters";
    // Sparse: exactly one instance.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// A completed purchase — goods for money, both already transferred
/// (a fact for observability, never used to reconstruct state; SPEC §7,
/// ADR 0007 §4). Emitted for citizen retail purchases and firm-to-firm
/// procurement alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoodsPurchased {
    /// Who paid.
    pub buyer: Entity,
    /// The selling firm.
    pub seller: Entity,
    /// The good (data-order index).
    pub good: u32,
    /// Units transferred.
    pub quantity: i64,
    /// Total money paid.
    pub total: Money,
}

impl crate::Event for GoodsPurchased {
    const NAME: &'static str = "econ.goods_purchased";
}

/// A firm's posted price moved (emitted by pricing when new ≠ old).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceChanged {
    /// The repricing firm.
    pub firm: Entity,
    /// The good whose price moved (the firm's output).
    pub good: u32,
    /// Yesterday's posted price.
    pub old: Money,
    /// Today's posted price.
    pub new: Money,
}

impl crate::Event for PriceChanged {
    const NAME: &'static str = "econ.price_changed";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_stock_treats_out_of_range_as_zero() {
        let inv = Inventory {
            quantities: vec![3, 0, 7],
        };
        assert_eq!(inv.stock(0), 3);
        assert_eq!(inv.stock(2), 7);
        assert_eq!(inv.stock(9), 0);
    }

    #[test]
    fn counters_start_zeroed_at_the_goods_length() {
        let c = EconCounters::new(4);
        assert_eq!(c.issued, Money::ZERO);
        for list in [
            &c.produced,
            &c.consumed_by_citizens,
            &c.consumed_in_production,
            &c.spoiled,
        ] {
            assert_eq!(list.len(), 4);
            assert!(list.iter().all(|q| *q == 0));
        }
    }
}
