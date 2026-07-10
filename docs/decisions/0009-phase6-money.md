# ADR 0009 — Phase 6 design: housing, the bank, taxes, treasury

- Status: Accepted
- Date: 2026-07-10
- Phase: 6

SPEC §15 Phase 6 ("Housing stock, rental/purchase matching, the bank
(deposits/loans/policy rule), tax pipeline, treasury, public employees.
Exit: full monetary loop closes; conservation audit spans every ledger
including bank and treasury; a rate change measurably shifts credit and
construction") and §12's housing/banking/taxes bullets underdetermine
mechanics; per SPEC §16.7 the decisions precede the code.

## 1. Money stays fully accounted: the bank is a vault, not a printer

SPEC §12's auditor demands `Σ money across all ledgers == issuance −
destruction`. Phase 6 keeps issuance genesis-only by making the bank
**full-reserve in cash terms**: a deposit moves mills from the citizen's
wallet into the bank's wallet and writes an equal `deposits` liability
row; a loan moves mills from the bank's wallet to the borrower's and
writes a `loans` asset row. The bank can only lend cash it physically
holds (equity seeded at genesis + deposits). Nothing is created or
destroyed; the audit identity stays `Σ wallets == issued`, with two NEW
cross-ledger identities: `bank.wallet == Σ deposit rows + equity −
Σ outstanding principal` (the vault balances its book) and
`treasury.wallet` is just a wallet (taxes are transfers). Credit-money
creation is a later economy-opening step, not Phase 6.

## 2. The bank

One `Bank` entity (spawned by econ genesis beside the ledger), carrying
`Wallet` + `BankBook { equity_seed, deposits: Vec<(Entity, Money)>,
loans: Vec<Loan> }` (shared, `sim_interface` — `debug_tools` audits it).
`Loan { borrower, principal_outstanding, rate_per_mille_daily,
day_payment }`.

- **Wealth signals read savings, not pocket cash**: with most wealth in
  the vault, the marginal utility of wealth (purchase scoring) and the
  reservation wage read `wallet + deposit balance`; affordability at the
  till stays wallet-only (you spend what you carry).
- **Deposits/withdrawals** (day-rate `BankSystem`, in `sim_economy`):
  citizens keep a data-defined float (`target_cash_mills`) in the wallet
  and deposit the excess; they withdraw when below the float (bounded by
  their deposit row). Deposit interest accrues daily from the bank's
  wallet at `policy_rate − deposit_spread` (floored at 0), paid into the
  deposit row (compounding inside the vault, still fully backed —
  interest is a transfer from bank equity/earnings).
- **Loans**: firms (Phase 6 borrowers) request a loan when cash falls
  below a data-defined working-capital floor; size = data multiple of
  their daily payroll; granted only if the bank holds the cash and the
  borrower's books look serviceable (revenue over the trailing window ≥
  data multiple of the payment — risk scoring in its smallest honest
  form; risk premium per-mille added to the policy rate). Repayment is a
  daily transfer (interest first, then principal); a borrower who cannot
  pay defaults: the bank writes the loan off against equity (the loss is
  a transfer already made — no money vanishes) and the borrower is
  flagged uncreditworthy for a data cooldown.
- **Policy rate** (`PolicySystem`, day): a Taylor-style rule on MEASURED
  inflation — the price index is the average posted price of the retail
  goods basket; inflation is its N-day log-free relative change (integer
  per-mille); `rate = neutral + sensitivity × (inflation − target)`,
  clamped to data bounds. The rate is the one macro lever and it is a
  modeled actor (SPEC §12).

## 3. Housing: ownership, rent, and the two matchings

Homes become owned: each genesis home gets an `Ownership { owner }` —
the household's lowest-indexed adult member (deterministic). Citizens
without an owned home RENT: `Tenancy { home, landlord, rent_per_day }`.
Rent is a daily transfer tenant→landlord (a booked expense for firm
landlords; plain transfer between citizens). The two markets share the
double-auction machinery labor uses (SPEC §12 "the same matching
machinery"):

- **Rental market** (day): landlords with vacant owned homes ask
  (cost-plus floor: data upkeep + tax share; controller on vacancy);
  homeless households bid from wealth/wages. Matches create `Tenancy`.
- **Purchase market** (every N days, data): owners with a second home
  (construction output, inheritance) offer; renters with savings above a
  data multiple of the price bid, financed cash-first, remainder as a
  bank mortgage (a `Loan` with the home as collateral). Ownership
  transfers atomically with the money legs.

Phase 6 genesis keeps every household housed (as today) — the markets
matter at the margins (new adults, immigrants later, construction
output, deaths), which is honest for a small town.

## 4. Taxes and the treasury

A `Treasury` entity carrying `Wallet` + `FirmBooks` (public payroll
books like any employer's; taxes book as revenue, so the per-firm
ledger identity keeps auditing it) + `TreasuryBook` receipt counters +
its town-hall `Location`. Data-defined: **income tax** (per-mille,
withheld by payroll at transfer time and routed treasury-ward in the
same atomic call — the treasury's own payroll is not taxed back into
itself), **sales tax** (per-mille, split out of the posted price at the
till). The treasury IS the public employer: it bids `public_positions`
slots at its data wage in the same labor clearing, and the ordinary
payroll pays (or, in austerity, fires) its workers from its wallet.
No `town_hall` firm kind exists — a firm without a recipe would be a
fake; the treasury employs directly.

## 5. Construction closes the rate→real-economy loop

A `builder` firm kind with a `construction` pseudo-recipe: consuming
timber over a data batch time with workers present yields a NEW home
entity (owned by the builder, offered on the purchase market, rentable
meanwhile). Builders start a house only when the market price of homes
(last purchase-market clearing average, or the data floor before any
sale) exceeds data cost × margin — and they finance materials with bank
credit when cash is short: the bank sizes the loan to the batch's
materials plus a payroll cushion (one loan at a time), and the start
hurdle doubles as the serviceability screen — a granted materials loan
is one whose sale already covers cost + financing + margin. Builders
never take the payroll-multiple working-capital loan (their revenue is
lumpy by nature; screening it would misprice them both ways). Higher policy rate → costlier credit → fewer
starts; lower → more. Housing starts are measured by counting home
locations beyond the genesis stock (no persisted counter — the world
itself is the record), plus the `HomeBuilt` facts.

## 6. Save format v7

Components (append): `econ.bank_book, econ.treasury_book,
world.ownership, world.tenancy, econ.borrower_status`. Events:
`econ.loan_granted, econ.loan_defaulted, econ.tenancy_started,
econ.home_sold, econ.home_built, econ.tax_collected`.
FORMAT_VERSION 7, mechanical v6→v7, chained from v1, goldens re-recorded
with continuity proofs, new v7 fixture, pinned snapshot `data_v7`.
Migrated pre-v7 towns have no bank/treasury/ownership — the new systems
no-op without the bank entity (the catch-up lesson from ADR 0008 §8's
review: gate on `EconCounters`, create the bank/treasury rows on first
use ONLY where that is honest — here it is NOT: a bank needs seeded
equity, which migrations must not invent, so migrated towns simply have
no bank until a later re-genesis mechanism; documented limitation).

## 7. Data

`data/balance/bank.ron` (equity seed, deposit spread, target cash
float, loan multiples, serviceability window/multiple, risk premium,
default cooldown, policy neutral/target/sensitivity/bounds, index
basket = retail goods, N days). `data/balance/housing.ron` (upkeep,
vacancy controller, rent floor, purchase cadence, savings multiple,
mortgage LTV cap, home price floor, construction cost margin).
`data/balance/taxes.ron` (income/sales per-mille, treasury seed, the
public employer's slots/wage/location kind). `data/firms.ron` gains the
`builder` kind; `data/recipes.ron` gains the construction recipe
(timber in, `output: None`, `builds_home: true` — validated: None iff
builds_home; retail kinds must produce goods).
`data/locations.ron` appends `town_hall` and `builder_yard` kinds.
Validation: rates/per-milles in range, savings+LTV cover the price,
policy bounds ordered, the public location kind exists with count 0.

## 8. Exit criteria mapping

- *Full monetary loop closes*: one integration run where a single mill
  can be traced through every station — wages → income tax → treasury →
  public wage → retail purchase (sales tax) → firm revenue → loan
  repayment → deposit interest — asserted by counters all moving while
  `Σ wallets == issued` holds daily.
- *Audit spans every ledger*: the auditor gains the bank-book identity
  (`bank wallet == deposits + equity − outstanding`) and treasury checks;
  seeded drift in a deposit row halts a scheduled run.
- *A rate change measurably shifts credit and construction*: two
  same-seed runs with different data policy bounds (a test-shaped rate
  peg, like the flat-mortality pattern): the high-rate town shows less
  outstanding credit and fewer housing starts than the low-rate twin
  over the same horizon.

## Dependencies

No new external dependencies.
