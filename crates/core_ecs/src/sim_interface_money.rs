//! Banking, treasury, and housing components shared between sim crates
//! (Phase 6, ADR 0009). Split file for the SPEC §3 module-size rule.

use core_types::Money;
use serde::{Deserialize, Serialize};

use crate::entity::Entity;
use crate::store::{Component, StorageKind};

/// One outstanding loan (ADR 0009 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loan {
    /// The borrowing entity (a firm in Phase 6).
    pub borrower: Entity,
    /// Principal still owed, mills.
    pub principal: Money,
    /// Interest per day, per-million of outstanding principal.
    pub rate_per_million_daily: i64,
    /// The fixed daily payment (interest first, then principal).
    pub day_payment: Money,
}

/// The bank's book (ADR 0009 §§1–2), on the single bank entity beside
/// its `Wallet`. The vault identity the auditor enforces:
/// `wallet == Σ deposits + equity − Σ outstanding principal`.
/// Money never enters or leaves the world here — every flow is a
/// transfer, and deposit interest moves value from `equity` into
/// deposit rows entirely inside the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankBook {
    /// The bank's own capital: genesis seed + retained interest earnings
    /// − deposit interest paid − default write-offs.
    pub equity: Money,
    /// Today's policy rate, per-million per day (the Taylor-rule output).
    pub policy_rate_per_million_daily: i64,
    /// The previous price-index observation, milli-units (0 = unset).
    pub last_price_index_milli: i64,
    /// Deposit rows `(owner, balance)`, kept in owner entity-index order.
    pub deposits: Vec<(Entity, Money)>,
    /// Outstanding loans, in grant order.
    pub loans: Vec<Loan>,
    /// Lifetime interest collected from borrowers — a counter, not a
    /// balance (the mills sit in `equity`); the loan-repayment station
    /// of SPEC §15's "full monetary loop" made measurable.
    pub interest_received: Money,
    /// Lifetime deposit interest credited to savers (counter).
    pub deposit_interest_paid: Money,
}

impl Component for BankBook {
    const NAME: &'static str = "econ.bank_book";
    // Sparse: exactly one bank.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// The treasury's receipt counters (ADR 0009 §4), on the treasury entity
/// beside its `Wallet` + `FirmBooks` (public payroll books like any
/// employer's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TreasuryBook {
    /// Lifetime income-tax receipts.
    pub income_tax_received: Money,
    /// Lifetime sales-tax receipts.
    pub sales_tax_received: Money,
}

impl Component for TreasuryBook {
    const NAME: &'static str = "econ.treasury_book";
    // Sparse: exactly one treasury.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// Who owns a home (ADR 0009 §3), on the home location entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ownership {
    /// The owning entity (citizen, builder firm, or the treasury).
    pub owner: Entity,
}

impl Component for Ownership {
    const NAME: &'static str = "world.ownership";
    // Sparse: homes only.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// A rental agreement (ADR 0009 §3), on the tenant citizen. The rent
/// flows daily to the home's CURRENT owner (resolved live, so
/// inheritance keeps working without stale landlord handles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenancy {
    /// The rented home entity.
    pub home: Entity,
    /// Daily rent, mills.
    pub rent_per_day: Money,
}

impl Component for Tenancy {
    const NAME: &'static str = "world.tenancy";
    // Sparse: renters are the minority.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// A defaulted borrower's cooldown (ADR 0009 §2): no credit until the
/// stamped day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorrowerStatus {
    /// First day (day index) the entity may borrow again.
    pub uncreditworthy_until_day: u64,
}

impl Component for BorrowerStatus {
    const NAME: &'static str = "econ.borrower_status";
    // Sparse: defaults are rare.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// A loan was granted (a fact; ADR 0009 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoanGranted {
    /// The borrower.
    pub borrower: Entity,
    /// Principal transferred.
    pub principal: Money,
    /// The contract rate, per-million daily.
    pub rate_per_million_daily: i64,
}

impl crate::Event for LoanGranted {
    const NAME: &'static str = "econ.loan_granted";
}

/// A loan defaulted; the bank wrote the remainder off against equity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoanDefaulted {
    /// The failed borrower.
    pub borrower: Entity,
    /// Principal written off.
    pub written_off: Money,
}

impl crate::Event for LoanDefaulted {
    const NAME: &'static str = "econ.loan_defaulted";
}

/// A tenancy began at the rental clearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenancyStarted {
    /// The tenant.
    pub tenant: Entity,
    /// The home.
    pub home: Entity,
    /// The cleared daily rent.
    pub rent_per_day: Money,
}

impl crate::Event for TenancyStarted {
    const NAME: &'static str = "econ.tenancy_started";
}

/// A home changed owners at the purchase clearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeSold {
    /// The home.
    pub home: Entity,
    /// Previous owner.
    pub seller: Entity,
    /// New owner.
    pub buyer: Entity,
    /// The price paid.
    pub price: Money,
}

impl crate::Event for HomeSold {
    const NAME: &'static str = "econ.home_sold";
}

/// A builder completed a new home (ADR 0009 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeBuilt {
    /// The building firm.
    pub builder: Entity,
    /// The new home entity.
    pub home: Entity,
}

impl crate::Event for HomeBuilt {
    const NAME: &'static str = "econ.home_built";
}

/// Which tax a collection was (ADR 0009 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaxKind {
    /// Withheld from a wage at payroll.
    Income,
    /// Split out of a retail purchase at the till.
    Sales,
}

/// Tax landed in the treasury (a fact).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxCollected {
    /// Who bore the tax.
    pub payer: Entity,
    /// The amount, mills.
    pub amount: Money,
    /// Income or sales.
    pub kind: TaxKind,
}

impl crate::Event for TaxCollected {
    const NAME: &'static str = "econ.tax_collected";
}
