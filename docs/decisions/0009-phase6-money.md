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
`Wallet` + `BankBook { equity, policy_rate, last_price_index, deposits:
Vec<(Entity, Money)>, loans: Vec<Loan>, interest_received,
deposit_interest_paid, granted: Vec<(Entity, Money)> }` (shared,
`sim_interface` — `debug_tools` audits it). `Loan { borrower,
principal, rate_per_million_daily, day_payment, collateral:
Option<Entity> }`. **Amended in-phase:** daily rates are integer
PER-MILLION (0.08%/day is inexpressible in per-mille); the lifetime
interest counters make the loop's repayment and deposit stations
measurable; `granted` records lifetime principal per borrower so the
serviceability screen reads revenue NET of borrowings (grants book as
revenue for the ledger identity — without the deduction every loan
would ratchet up its borrower's own credit history); `collateral`
secures mortgages (§3).

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
  borrower's books look serviceable (lifetime revenue net of past
  grants ≥ data multiple of the loan's FULL obligation — term × day
  payment, so dear money tightens credit; risk premium added to the
  policy rate). **Amended:** the promised trailing window is dropped
  for Phase 6 — towns are young and windowing adds state for little
  signal; net-of-borrowings lifetime revenue is the honest simple form.
  One loan per borrower (no stacking). Repayment is a daily transfer
  (interest first, then principal), auto-debited from the borrower's
  own deposit row when the wallet is short but the vault balance
  covers it (a depositor who can pay is not a defaulter). A borrower
  who cannot pay defaults: the bank RECOVERS what exists — the deposit
  row nets against principal, remaining cash is seized, a mortgage's
  collateral home passes to the bank (the defaulting occupant loses
  the Residence: measured homelessness) — and only the residual is
  written off against equity, with the cooldown flag. Equity may go
  negative: bank insolvency is a modeled state, not an invariant
  violation (deposit interest already stops when equity cannot fund
  it). The bank's own recoveries — rent on repossessed homes, resale
  proceeds — credit equity, keeping the vault identity.
- **Policy rate** (`PolicySystem`, day): a Taylor-style rule on MEASURED
  inflation — the price index is the average posted price of the retail
  goods basket; inflation is its N-day log-free relative change (integer
  per-mille); `rate = neutral + sensitivity × (inflation − target)`,
  clamped to data bounds. The rate is the one macro lever and it is a
  modeled actor (SPEC §12).

## 3. Housing: ownership, rent, and the two matchings

Homes become owned: each genesis home gets an `Ownership { owner }` —
the household's lowest-indexed adult member (deterministic). Citizens
without an owned home RENT: `Tenancy { home, rent_per_day }`
(**amended:** no stored landlord — rent resolves the home's CURRENT
owner live, so inheritance and repossession keep working without stale
handles). Rent is a daily transfer tenant→owner (booked revenue for
firm landlords; equity income for the bank's repossessions; plain
transfer between citizens); a tenant who cannot pay is evicted —
tenancy AND residence end, measured homelessness. The two markets
share the double-auction machinery labor uses (SPEC §12):

- **Rental market** (day): vacant owned homes ask the controller-
  steered rent — floored at the data cost-plus basis, stepped down
  when measured vacancy exceeds the data target and up when it falls
  short (**amended:** the "tax share" of the floor waits for a
  property tax to exist); homeless citizens bid a data share of their
  SAVINGS (wallet + vault row — §2's wealth-signal rule; the float
  sweep pins pocket cash). Midpoint rents; matches create `Tenancy`.
- **Purchase market** (every N days, data): every live owner's VACANT
  home is on offer — builders, the bank's repossessions, the
  treasury's escheats, citizens' inherited second homes — at the
  measured anchor price (the last clearing's average; the data floor
  before any sale). Buyers are citizens WITHOUT a home whose savings
  clear the data screen; they bid a data share of savings (a share
  above 1000‰ is deliberate — buyers bid leveraged, knowing the bank
  funds the stretch, bounded by what cash-first + the LTV cap can
  close). Midpoint prices; financing is cash-first — savings less a
  living reserve (the cash float: a buyer stripped to zero would
  default on the first payment) — with the remainder a mortgage capped
  by LTV, secured by the home, passing the same credit gates as any
  loan (no stacking, no cooldown lending). The clearing average
  persists on the ledger's `HousingBook` as the market price of homes.

The demand margin is real: a citizen coming of age in a home they do
not own LEAVES THE NEST (the Residence ends at promotion) — new
adults are the rental market's first customers, exactly the margin
this section always named; foreclosure evictions are the second.
Phase 6 genesis still keeps every household housed.

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
(the `HousingBook` anchor: last purchase-clearing average, or the data
floor before any sale) exceeds data cost × margin — and they finance
materials with bank credit when cash is short: the bank sizes the loan to the batch's
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
world.ownership, world.tenancy, econ.borrower_status,
econ.housing_book`. Events:
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
basket = retail goods, N days). `data/balance/housing.ron` (upkeep, rent margin + the vacancy
controller's target and step, rent bid share, purchase cadence, savings
screen, leveraged bid share, mortgage LTV cap, home price floor,
construction cost margin).
`data/balance/taxes.ron` (income/sales per-mille, treasury seed, the
public employer's slots/wage/location kind). `data/firms.ron` gains the
`builder` kind; `data/recipes.ron` gains the construction recipe
(timber in, `output: None`, `builds_home: true` — validated: None iff
builds_home; retail kinds must produce goods).
`data/locations.ron` appends `town_hall` and `builder_yard` kinds.
Validation: rates/per-milles in range with CEILINGS (policy_max + risk
premium ≤ 100%/day — overflow unreachable by construction), savings+LTV
cover the price, the leveraged bid share within what financing can
close, policy bounds ordered, the public location kind (read from
taxes.ron, never hard-coded) exists with count 0, and the degenerate
zeros rejected (public positions, default cooldown, cash float ≥ 1).

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
