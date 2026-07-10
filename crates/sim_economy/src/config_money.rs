//! Phase 6 tunables: the RON schemas for `data/balance/{bank,housing,
//! taxes}.ron` (SPEC §8; ADR 0009 §7) and their resolved tables. Split
//! file for the SPEC §3 module-size rule.

use serde::Deserialize;

/// `data/balance/bank.ron` (ADR 0009 §2). All rates are per-million per
/// day (the calendar year is short; annual quotes would be misleading).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BankConfig {
    /// The bank's genesis capital, mills (counted in issuance).
    pub equity_seed_mills: i64,
    /// Citizens keep this much cash in the wallet; the excess deposits.
    pub target_cash_float_mills: i64,
    /// Deposit rate = policy − spread, floored at 0.
    pub deposit_spread_per_million_daily: i64,
    /// Loan principal = per-mille × the borrower's committed daily
    /// payroll (3000 = 3×).
    pub loan_payroll_multiple_per_mille: i64,
    /// Borrow when cash falls below this many days of payroll.
    pub working_capital_floor_days: i64,
    /// Serviceability: lifetime revenue × per-mille/1000 must cover the
    /// loan's full obligation — term × day payment, rate included, so
    /// dear money tightens credit (risk scoring in its smallest honest
    /// form, ADR 0009 §2).
    pub serviceability_revenue_per_mille: i64,
    /// Risk premium added to the policy rate on every loan.
    pub risk_premium_per_million_daily: i64,
    /// Days a defaulted borrower stays uncreditworthy.
    pub default_cooldown_days: u64,
    /// Amortization term: day_payment = principal/term + interest.
    pub repay_term_days: i64,
    /// Taylor rule: the neutral rate.
    pub policy_neutral_per_million_daily: i64,
    /// Taylor rule: target inflation per measurement period, per-mille.
    pub policy_target_inflation_per_mille: i64,
    /// Taylor rule: rate response per per-mille of inflation gap.
    pub policy_sensitivity_per_million: i64,
    /// Policy rate floor.
    pub policy_min_per_million_daily: i64,
    /// Policy rate ceiling.
    pub policy_max_per_million_daily: i64,
    /// Measure the price index (and re-run the rule) every N days.
    pub index_period_days: u64,
}

/// `data/balance/housing.ron` (ADR 0009 §§3, 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HousingConfig {
    /// Landlord's daily cost basis, mills (the rent ask floor input).
    pub upkeep_mills_per_day: i64,
    /// Rent ask = upkeep × (1000 + margin) / 1000.
    pub rent_margin_per_mille: i64,
    /// A homeless bidder offers this per-mille of their cash as daily
    /// rent.
    pub rent_bid_per_mille: i64,
    /// The purchase market clears every N days.
    pub purchase_period_days: u64,
    /// The asking price for a home, mills.
    pub home_price_mills: i64,
    /// A buyer must hold savings ≥ per-mille × price / 1000.
    pub buyer_savings_per_mille: i64,
    /// Mortgage cap: loan ≤ per-mille × price / 1000.
    pub mortgage_ltv_per_mille: i64,
    /// Builders start a home only when the price covers estimated cost ×
    /// (1000 + margin) / 1000 — financing costs included (the rate
    /// lever's teeth, ADR 0009 §5).
    pub construction_margin_per_mille: i64,
}

/// `data/balance/taxes.ron` (ADR 0009 §4).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaxesConfig {
    /// Income tax withheld from every wage, per-mille.
    pub income_per_mille: i64,
    /// Sales tax split out of every retail purchase, per-mille.
    pub sales_per_mille: i64,
    /// The treasury's genesis cash, mills (counted in issuance).
    pub treasury_seed_mills: i64,
    /// Public worker slots at the town hall.
    pub public_positions: u32,
    /// The treasury's standing wage bid, mills/day (affordability-
    /// clamped like any firm's).
    pub public_wage_bid_mills: i64,
    /// The location kind (from `data/locations.ron`) the treasury
    /// occupies.
    pub public_location_kind_id: String,
}

/// Resolved Phase 6 tables (string ids → indices), carried inside
/// `EconTables`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoneyTables {
    /// Bank tunables (used verbatim — no ids to resolve).
    pub bank: BankConfig,
    /// Housing tunables.
    pub housing: HousingConfig,
    /// Tax rates and the public employer.
    pub income_per_mille: i64,
    /// Sales tax, per-mille.
    pub sales_per_mille: i64,
    /// Treasury genesis cash.
    pub treasury_seed: core_types::Money,
    /// Public worker slots.
    pub public_positions: u32,
    /// Public wage bid, mills.
    pub public_wage_bid_mills: i64,
    /// The treasury's location kind index.
    pub public_location_kind: u32,
    /// The home location kind index (new homes spawn as this).
    pub home_location_kind: u32,
}
