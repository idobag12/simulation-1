# Phase 6 report — Money: the bank, treasury, taxes, housing

- Date: 2026-07-10
- Scope: SPEC §15 Phase 6 (housing stock, rental/purchase matching, the
  bank — deposits/loans/policy rule — tax pipeline, treasury, public
  employees)
- Design decisions: ADR 0009 (written before the code; §§4–5 amended
  during implementation, deviations documented there first)
- Verdict: **complete** — all three exit criteria demonstrated by
  CI-blocking tests; `scripts/check.sh` green.

## What was built

| Deliverable | Where |
|---|---|
| The full-reserve bank: one entity carrying `Wallet` + `BankBook` (equity, policy rate, deposit rows, loans, lifetime interest counters); every flow is a transfer — deposits move cash into the vault against a liability row, loans move vault cash out against an asset row; nothing is minted (`Σ wallets == issued` still holds) | `crates/core_ecs/src/sim_interface_money.rs`, `crates/sim_economy/src/bank.rs`, `genesis.rs` |
| Deposits: citizens keep a data cash float and bank the rest daily; withdrawals refill the float; deposit interest (policy − spread, floored at 0) moves equity into rows entirely inside the vault | `bank.rs::BankSystem::deposits` |
| Credit: working-capital loans (payroll multiple, requested below the data floor) screened on the PAYMENT — lifetime revenue NET of past grants must cover term × day-payment, so dear money tightens credit and no loan inflates its borrower's own history; materials credit for builders (sized to the batch's inputs + payroll cushion, the construction hurdle doubling as the serviceability screen); one loan per borrower; risk premium over policy; interest-first amortization; servicing auto-debits the borrower's own deposit row before defaulting a depositor who can pay; default → partial RECOVERY (row netting + cash seizure + collateral repossession) with only the residual written off, plus the cooldown | `crates/sim_economy/src/{bank,vault}.rs` |
| Policy: a Taylor-style rule on the MEASURED retail price index (average posted price of the retail basket, N-day period), clamped to data bounds — the one macro lever, itself a modeled actor | `bank.rs::run_policy` |
| The treasury IS the public employer: `Wallet` + `FirmBooks` (so the per-firm ledger identity audits it) + `TreasuryBook` receipt counters + its town-hall `Location`; it bids `public_positions` at the data wage in the same clearing, and payroll knows its slot count (no phantom redundancy) | `sim_interface_money.rs`, `genesis.rs`, `labor.rs` |
| Taxes at the atomic transfer: income tax withheld inside `transfer_wage` (never on the treasury's own payroll), sales tax split inside `execute_purchase` at the till; both booked as treasury revenue + receipt counters + `TaxCollected` facts | `crates/sim_economy/src/labor.rs`, `crates/sim_ai/src/systems_act.rs` |
| Housing: genesis homes get `Ownership` (first working-age member); daily `RentSystem` (tenant → live owner, eviction on nonpayment); rental clearing with a vacancy CONTROLLER on the ask (cost-plus floored) and savings-based bids; purchase clearing every N days as a real double auction — any live owner's vacant home offers at the measured anchor, non-owner citizens bid a leveraged share of savings, midpoint prices, cash-first financing (less a living reserve) with the remainder an LTV-capped mortgage SECURED by the home; the clearing average persists on `HousingBook` as the market price construction reads; new adults leave the nest at promotion (the rental market's demand margin) | `crates/sim_economy/src/{housing,housing_market}.rs`, `crates/sim_world/src/genesis.rs`, `crates/sim_people/src/systems.rs` |
| Construction closes the rate→real loop: `builds_home` recipes output a NEW home entity (owned by the builder); starts gated on price ≥ (materials at cheapest posted + financing surcharge when cash-short) × margin — a higher policy rate suppresses starts, and the twins prove it | `crates/sim_economy/src/{construction,systems}.rs` |
| Committed money is not free cash: procurement now reserves wages + debt service before buying inputs (the Phase 5 bid-clamp lesson applied to tills — without it every borrower spent the loan the same day and defaulted on a payment it could afford) | `systems.rs::TradeSystem` |
| Estates grew — and debts settle FIRST: the deceased's loans repay from cash, then the deposit row (vault-internal netting), with a mortgage's collateral passing to the bank when the estate falls short; only then do the remaining row, cash, and unencumbered homes pass to the heir (or the treasury) | `crates/sim_people/src/systems.rs` |
| Wealth signals read savings: purchase scoring's marginal utility and the reservation wage read wallet + deposit row; affordability at the till stays wallet-only | `crates/sim_ai/src/systems.rs`, `labor.rs` |
| The auditor spans the new ledgers: the vault identity (`bank wallet == Σ deposits + equity − Σ outstanding`), non-negative rows, liveness (no dead depositor, borrower, or home owner — index-reuse hazards halt instead of aliasing), and the treasury's books audited like any firm's — daily, in-schedule, halting | `crates/debug_tools/src/audit.rs` |
| Data + validation: `balance/{bank,housing,taxes}.ron`; `builder` firm kind, `town_hall`/`builder_yard` location kinds, `output: None ⇔ builds_home` recipes; rates/per-milles in range, savings+LTV cover the price, policy bounds ordered, public kind exists with count 0 | `data/`, `crates/data_defs` |
| Save format v7: six appended components + six events, pure `v6→v7`, chain from v1, continuity proofs for v2–v6 fixtures, new v7 fixture saved mid-shift AND mid-loan (deposits, nine collateralized mortgages amortizing, both taxes, a measured clearing price, the rate off neutral — all asserted); goldens re-recorded under `data_v7` per policy; a functional test drives a REAL purchase clearing from restored state | `crates/persistence`, `tests/tests/save_compat.rs` |
| Observability: the economy report gains the bank line (vault/deposits/equity/outstanding/rate/interest in-out) with per-loan rows, the housing line (rent ask, clearing price, tenancies), and the treasury line; citizen dumps show deposits, loans, tenancy, owned homes, and credit status | `crates/headless/src/{inspect,inspect_econ}.rs` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 6) | Test |
|---|---|
| Full monetary loop closes | `money::the_full_monetary_loop_closes` — twelve days under daily in-schedule audits: wages paid (and withheld), sales taxed at the till, public wages OUT of the treasury, deposits banked and earning, mortgages written at the purchase clearing, loan interest collected — every station's counter moved while `Σ wallets == issued` held every day (a halt would fail the run) |
| Conservation audit spans every ledger including bank and treasury | The vault identity + treasury books audited daily; `money::a_scheduled_run_halts_on_seeded_deposit_drift` — one mill of drift in a deposit row (a book entry no Phase 4 identity can see) halts the scheduled run at the next boundary with the vault violation |
| A rate change measurably shifts credit and construction | `money::a_rate_change_shifts_credit_and_construction` — same-seed, same-data twins, Taylor rule pegged (min == max) at 100 vs 150 000 per-million/day: cheap money's builder exhausts its till, takes materials credit, and keeps building; dear money's financing surcharge pushes the hurdle past the home price — strictly fewer starts and strictly less outstanding credit, with the low twin's credit real and serviced |

Also this phase:

- The public employer end to end at the unit level: the treasury's bid
  clears, its hires survive redundancy, payroll pays gross from its
  wallet (`sim_economy::labor_tests::the_treasury_employs_and_pays_public_workers`).
- Withholding and booking proven at the unit level (gross expense, net
  wage, tax in the treasury's row).
- The v7 fixture loads to its golden and resumes deterministically
  through a day boundary running every money system.
- The whole Phase 3/4/5 exit suites still pass over the money layer:
  the town feeds itself, prices respond, the labor market clears — now
  with wages taxed, savings banked, and homes owned.

## Verification process

`scripts/check.sh` green at the tagged commit (fmt, clippy `-D
warnings`, full workspace suite — 46 test binaries including the
1M-tick, 10k-citizen, 100-day-conservation, labor, and money suites).
Adversarial multi-agent review (ultracode): five reviewers over the
phase diff (conservation/atomicity, determinism/ordering, save-format
v7/migrations, SPEC+ADR conformance, edge cases/failure modes),
findings verified against the code before acting.

Bugs the phase's own instrumentation caught before review (fixed and
regression-guarded in the exit suites):

- **Every mortgage defaulted unserviced**: borrowers' wealth sits in
  the vault, and the float refill ran after servicing — the bank now
  auto-debits the borrower's deposit row before declaring default.
- **Borrowers spent the loan the same day**: procurement ignored
  committed obligations; the trade reserve fixed it (see table).
- **Public workers were never paid**: the redundancy check read the
  treasury's missing `Firm` as zero positions and fired all six every
  morning before payday; unit-tested now.
- **Second mortgages for the rich**: any citizen with savings could
  buy; ADR §3 says renters bid — buyers are now citizens without a
  home.

Confirmed review findings and their fixes are listed below.

Confirmed findings (five reviewers: conservation/atomicity,
determinism/ordering, save-format v7, SPEC+ADR conformance, edge
cases/failure modes), all fixed in the review commit:

- **The bank wallet could go negative on honest sequences** (critical):
  `vault_withdraw` never checked the vault's cash, the purchase market
  checked bank cash BEFORE the down-payment withdrawal drained it, and
  origination could lend the vault to the floor ahead of the float
  refills. Fixed structurally: the purchase clearing pre-checks the
  WHOLE draw (mortgage + top-up), float refills short-fill to what the
  vault holds, and the bank-liquidity crunch is unit-tested as a
  survivable, honest state.
- **Mortgages were unsecured and died with the borrower** (major): no
  collateral existed; estates kept home, cash, and deposits while the
  bank wrote off the full principal; the auto-debit was all-or-nothing.
  Fixed: `Loan.collateral`, repossession on default (the occupant loses
  the Residence — measured homelessness; the bank's resales and rents
  credit equity), estates settle debts BEFORE inheriting, defaults
  recover row + cash and write off only the residual. Negative equity
  is documented as modeled bank insolvency.
- **The rent station was unreachable** (major): genesis housed everyone,
  only eviction removed a Residence, eviction required a Tenancy, and
  tenancies required homelessness — a closed circle; rent bids also
  read the float-pinned wallet. Fixed: new adults leave the nest at
  promotion (ADR §3's own demand margin), foreclosure evicts, and
  rental bids read savings. Rent, eviction, and the clearing are
  unit-tested.
- **Serviceability self-ratchet** (major): loan principal books as
  revenue (ledger identity) and the screen read that same revenue —
  every grant improved its borrower's history. Fixed: `BankBook.granted`
  rows; the screen nets borrowings out (ADR amended: payment-priced,
  net-of-grants lifetime revenue).
- **No market price of homes; always-max-LTV mortgages; citizen sellers
  cut off; no vacancy controller** (major, conformance): the purchase
  clearing sold at a data constant. Fixed: the double auction above,
  cash-first financing, any live owner's vacant home offers, the
  controller on rents, and the measured average feeding the
  construction hurdle.
- **Core mechanics untested** (major): defaults, cooldowns, the screen,
  rent, eviction, the rental clearing, estate settlement, and the
  liquidity paths now have dedicated unit tests (`bank_tests.rs`,
  `sim_people` estate test); the v7 fixture asserts its own coverage
  claims.
- Minor: raw money arithmetic in housing/taxes routed through checked
  helpers with ONE shared day-payment formula; validation gained rate
  ceilings (overflow unreachable by construction), degenerate-zero
  rejections, and the public-location id read from `taxes.ron` instead
  of a hard-coded string; audit liveness clauses convert index-reuse
  hazards into halts; four modules split for the 500-line rule; the
  citizen dump shows deposits/loans/tenancy/ownership/credit status and
  the economy report lists every loan; ADR 0009 amended to record the
  shipped shapes (per-million rates, landlord-free `Tenancy`,
  `HousingBook`) and every in-phase design amendment.

## Deviations from SPEC

None beyond those recorded in ADR 0009 (and its in-phase amendments,
each documented before the code moved).

## Tag

`phase-6` (local; tag pushes are blocked by the environment's proxy —
the branch carries the content).
