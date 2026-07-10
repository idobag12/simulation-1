//! Firms and the posted-price market (SPEC §12, §15 Phase 4; design in
//! ADR 0007): production batches, daily firm-to-firm procurement, and
//! cost-plus pricing with an inventory controller. Labor, banking, firm
//! entry/exit, and auctions arrive in Phases 5–6.
//!
//! Invariants owned by this crate:
//! - Money and goods only ever TRANSFER here; the systems create goods
//!   solely through recipes (counted in `EconCounters::produced`) and
//!   never create or destroy money (ADR 0007 §§2, 4).
//! - Every transaction updates wallets/inventories, both firms' books,
//!   and the counters in the same system call — atomically (ADR 0007 §4).
//! - No RNG; exact integer arithmetic throughout (prices in mills,
//!   quantities in units).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod bank;
pub mod components;
pub mod config;
pub mod config_money;
mod construction;
pub mod genesis;
pub mod housing;
pub mod housing_market;
pub mod labor;
pub mod payroll;
pub mod systems;
mod transact;
mod vault;

pub use bank::BankSystem;
pub use components::{Firm, Production};
pub use config::{
    EconTables, EconomyConfig, FirmDef, FirmsConfig, GoodQty, LaborConfig, LaborTables, RecipeDef,
    RecipesConfig, RetailDef,
};
pub use config_money::{BankConfig, HousingConfig, MoneyTables, TaxesConfig};
pub use housing::{RentSystem, RentalMarketSystem};
pub use housing_market::PurchaseMarketSystem;
pub use labor::LaborMarketSystem;
pub use payroll::PayrollSystem;
pub use systems::{PricingSystem, ProductionSystem, TradeSystem};
