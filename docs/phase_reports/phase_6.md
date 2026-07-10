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
| Credit: working-capital loans (payroll multiple, requested below the data floor) screened on the PAYMENT — lifetime revenue must cover term × day-payment, so dear money tightens credit; materials credit for builders (sized to the batch's inputs + payroll cushion, the construction hurdle doubling as the serviceability screen); one loan per borrower; risk premium over policy; interest-first amortization; default → equity write-off + cooldown; servicing auto-debits the borrower's own deposit row before defaulting a depositor who can pay | `bank.rs::{originate,service_loans}` |
| Policy: a Taylor-style rule on the MEASURED retail price index (average posted price of the retail basket, N-day period), clamped to data bounds — the one macro lever, itself a modeled actor | `bank.rs::run_policy` |
| The treasury IS the public employer: `Wallet` + `FirmBooks` (so the per-firm ledger identity audits it) + `TreasuryBook` receipt counters + its town-hall `Location`; it bids `public_positions` at the data wage in the same clearing, and payroll knows its slot count (no phantom redundancy) | `sim_interface_money.rs`, `genesis.rs`, `labor.rs` |
| Taxes at the atomic transfer: income tax withheld inside `transfer_wage` (never on the treasury's own payroll), sales tax split inside `execute_purchase` at the till; both booked as treasury revenue + receipt counters + `TaxCollected` facts | `crates/sim_economy/src/labor.rs`, `crates/sim_ai/src/systems_act.rs` |
| Housing: genesis homes get `Ownership` (first working-age member); daily `RentSystem` (tenant → live owner, eviction on nonpayment); rental clearing (cost-plus asks × cash-share bids, midpoint rent); purchase clearing every N days (vacant non-citizen-owned homes; buyers are citizens WITHOUT a home, richest-first; cash down + LTV mortgage from the vault, ownership and every money leg in one pass) | `crates/sim_economy/src/housing.rs`, `crates/sim_world/src/genesis.rs` |
| Construction closes the rate→real loop: `builds_home` recipes output a NEW home entity (owned by the builder); starts gated on price ≥ (materials at cheapest posted + financing surcharge when cash-short) × margin — a higher policy rate suppresses starts, and the twins prove it | `crates/sim_economy/src/{construction,systems}.rs` |
| Committed money is not free cash: procurement now reserves wages + debt service before buying inputs (the Phase 5 bid-clamp lesson applied to tills — without it every borrower spent the loan the same day and defaulted on a payment it could afford) | `systems.rs::TradeSystem` |
| Estates grew: deposit rows merge to the heir (or escheat to the treasury); owned homes pass to the heir (or the treasury) | `crates/sim_people/src/systems.rs` |
| Wealth signals read savings: purchase scoring's marginal utility and the reservation wage read wallet + deposit row; affordability at the till stays wallet-only | `crates/sim_ai/src/systems.rs`, `labor.rs` |
| The auditor spans the new ledgers: the vault identity (`bank wallet == Σ deposits + equity − Σ outstanding`), non-negative rows, and the treasury's books audited like any firm's — daily, in-schedule, halting | `crates/debug_tools/src/audit.rs` |
| Data + validation: `balance/{bank,housing,taxes}.ron`; `builder` firm kind, `town_hall`/`builder_yard` location kinds, `output: None ⇔ builds_home` recipes; rates/per-milles in range, savings+LTV cover the price, policy bounds ordered, public kind exists with count 0 | `data/`, `crates/data_defs` |
| Save format v7: five appended components + six events, pure `v6→v7`, chain from v1, continuity proofs for v2–v6 fixtures, new v7 fixture saved mid-shift AND mid-loan (deposits, outstanding mortgages, both taxes, moved policy rate); goldens re-recorded under `data_v7` per policy | `crates/persistence`, `tests/tests/save_compat.rs` |
| Observability: the economy report gains the bank line (vault/deposits/equity/outstanding/loans/rate/interest in-out) and the treasury line (cash, receipts); builder firms show homes | `crates/headless/src/inspect.rs` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 6) | Test |
|---|---|
| Full monetary loop closes | `money::the_full_monetary_loop_closes` — twelve days under daily in-schedule audits: wages paid (and withheld), sales taxed at the till, public wages OUT of the treasury, deposits banked and earning, mortgages written at the purchase clearing, loan interest collected — every station's counter moved while `Σ wallets == issued` held every day (a halt would fail the run) |
| Conservation audit spans every ledger including bank and treasury | The vault identity + treasury books audited daily; `money::a_scheduled_run_halts_on_seeded_deposit_drift` — one mill of drift in a deposit row (a book entry no Phase 4 identity can see) halts the scheduled run at the next boundary with the vault violation |
| A rate change measurably shifts credit and construction | `money::a_rate_change_shifts_credit_and_construction` — same-seed, same-data twins, Taylor rule pegged (min == max) at 100 vs 150 000 per-million/day: cheap money's builder exhausts its till, takes materials credit, and keeps building; dear money's financing surcharge pushes the hurdle past the home price — strictly fewer starts and strictly less outstanding credit, with the low twin's credit real and serviced |

Also this phase:

- The public employer end to end at the unit level: the treasury's bid
  clears, its hires survive redundancy, payroll pays gross from its
  wallet (`sim_economy::systems_tests::the_treasury_employs_and_pays_public_workers`).
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

<!-- REVIEW FINDINGS -->

## Deviations from SPEC

None beyond those recorded in ADR 0009 (and its in-phase amendments,
each documented before the code moved).

## Tag

`phase-6` (local; tag pushes are blocked by the environment's proxy —
the branch carries the content).
