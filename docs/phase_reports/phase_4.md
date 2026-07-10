# Phase 4 report — Goods, firms, first market

- Date: 2026-07-10
- Scope: SPEC §15 Phase 4 (inventories, recipes, the production chain,
  firms with the double-entry slice, posted-price retail market, citizens
  buying food from seeded wealth, money & goods auditors live)
- Design decisions: ADR 0007 (written before the code; §8a amendment —
  death estates — added when implementation surfaced the gap, before the
  estate code was written)
- Verdict: **complete** — both exit criteria demonstrated by CI-blocking
  tests; `scripts/check.sh` green.

## What was built

| Deliverable | Where |
|---|---|
| Shared economy carriers (SPEC §4): `Wallet`, `Inventory`, `RetailOffer`, `FirmBooks`, `EconCounters` + `GoodsPurchased`/`PriceChanged` events | `crates/core_ecs/src/sim_interface_econ.rs` |
| Six-good chain (`grain, flour, water, bread, timber, firewood`), recipes with batch times, spoilage as the counted explicit sink — `spoil_stock` is the ONLY destruction path, shared by the daily system and seeded-shock interventions | `crates/sim_goods`, `data/goods.ron`, `data/recipes.ron` |
| Firms (`farm ×2, well ×2, forest, mill ×2, bakery ×3, woodyard`): production batches (hour rate), daily firm-to-firm procurement from the cheapest posted seller, cost-plus pricing under an inventory controller with bounded per-day movement (SPEC §12) | `crates/sim_economy`, `data/firms.ron`, `data/balance/economy.ron` |
| Citizens buy: retail firms are locations carrying `RetailOffer`; `DecideSystem` scores purchases with the marginal utility of wealth (`mu = scale/(1 + wallet/half_wealth)`, floats in scoring only); `ActSystem` executes the purchase atomically at performance start and aborts cleanly if stock/cash moved during travel | `crates/sim_ai` (new `Buy` candidates; `BuyTravel`/`BuyPending`/`Consume` actions — enum variants appended, old saves decode unchanged) |
| Money: citizen wallets seeded from `demographics.ron`, firm cash from `firms.ron`; the seeded total recorded as `issued` at genesis — the one modeled source; every later movement is a transfer | `crates/sim_people/src/genesis.rs`, `crates/sim_economy/src/genesis.rs` |
| Death estates (ADR 0007 §8a): the deceased's cash passes to the lowest-indexed surviving household member, else escheats to the ledger's wallet — money survives every death | `crates/sim_people/src/systems.rs::settle_death` |
| Auditors live (SPEC §12, "permanent"): every day, FIRST in the day list, both conservation identities and every firm's ledger identity are recomputed from the stores; any drift is `EcsError::InvariantViolation` — the tick aborts and the run halts, debug and release | `crates/debug_tools/src/audit.rs` |
| Data plumbing: `goods/recipes/firms/economy` schemas live with their consuming crates; `data_defs` loads, cross-validates (recipes↔goods, firms↔recipes↔location-kinds↔needs, every good has a producer, exactly one firm kind per retail location kind), and resolves index tables so sim crates never see each other's config types | `crates/data_defs` (`validate_econ.rs`, `resolve.rs`) |
| Save format v5: seven appended component registrations + two appended events, pure `v4→v5` migration, chain from v1, content-continuity proofs for v2/v3/v4 fixtures, new v5 fixture saved with a live economy (purchases counted, batches mid-flight) | `crates/persistence`, `tests/tests/save_compat.rs` |
| Tooling (SPEC §13): `embervale economy --load` renders every firm's price/stock/cash/books, the counters, and the live audit verdict; citizen dumps show wallets and purchase actions/candidates | `crates/headless/src/{inspect,cli}.rs` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 4) | Test |
|---|---|
| Audits pass over 100 days | `economy::conservation_audits_pass_over_100_days_of_live_economy` — 80 citizens, 100 days with the auditor INSIDE the schedule (any drift halts the run); final assertions prove the century was economically alive (>1000 citizen purchases, >1000 production inputs, >100 spoiled units, real firm revenue) and re-check every identity from outside |
| Prices respond to seeded supply shocks in the correct direction | `economy::prices_rise_after_a_seeded_supply_shock` — baseline vs. same-seed shocked run; every bakery's bread destroyed at day 10 through the counters-consistent spoilage path (auditor keeps passing); two days later every shocked posted price strictly exceeds its baseline twin AND stays within the controller's bounded movement. The glut direction is unit-tested at the controller (`sim_economy::systems_tests::pricing_cuts_in_glut_raises_in_shortage_and_respects_the_floor`) |

Also proven this phase:

- Per-firm ledger identity (`economy::firm_books_balance_and_both_sides_move`).
- Conservation across a full die-off — every estate escheats
  (`people::certain_mortality_kills_everyone_and_dissolves_every_household`).
- Atomicity/counting at the unit level: production consume/land, trade
  transfer, genesis issuance (`sim_economy::systems_tests`), spoilage
  floor arithmetic and stock bounds (`sim_goods` tests), each drift kind
  named by the auditor (`debug_tools` tests).
- v5 fixture loads to its golden hash with a live economy and resumes
  deterministically; v1–v4 goldens re-recorded under the v5 registration
  and `data_v5` snapshot per the ADR 0004 §10 policy, with byte-level
  content-continuity proofs for every migrated version.
- The AI exit suite now runs on the economic town: hunger is only
  satisfiable by BUYING bread, and the Phase 3 criteria (mean levels,
  risers, distinct patterns, sleep-at-home) still hold — the market
  actually feeds the town.

## Verification process

`scripts/check.sh` green at the tagged commit (fmt, clippy `-D warnings`,
full workspace suite — 130+ tests including the 1M-tick, 10k-citizen, and
100-day-conservation runs). Adversarial multi-agent review (ultracode):
five independent reviewers (determinism, conservation/economics,
standards, anti-shortcut/ADR-fidelity, test-coverage) over the phase
diff, each finding then adversarially verified by an independent agent
instructed to refute it. Confirmed findings and their fixes are listed
below; refuted findings are recorded in the review transcript.

<!-- REVIEW-RESULTS -->

## Deviations from spec (all pre-declared in ADRs)

- Full double-entry account trees, labor, banking, imports/exports:
  later phases by design (ADR 0007 §§2–3); Phase 4 firms carry the
  testable `initial_cash/revenue/expenses` slice.
- All Phase 4 goods use posted prices; the double-auction mechanism
  SPEC §12 assigns to commodities/labor arrives with those markets.
- Trade buys one seller per input per day (partial fills wait for
  tomorrow) — documented simplification in `TradeSystem`.
- The overhead cost is a modeled stand-in for rent/wear until Phases 5–6
  price them (`data/balance/economy.ron`).

## Known limitations (clean seams, not fakes)

- No income until Phase 5 wages: citizens spend seeded wealth; over very
  long horizons the town pauperizes (money conserved — it pools in firm
  wallets). The 100-day exit horizon is comfortably inside solvency.
- Hunger does not yet kill (mortality stays age-based); starvation
  consequences arrive with later-phase health.
- Firms are immortal this phase — entry/exit/bankruptcy is SPEC §12
  machinery scheduled with labor and banking.
- Unclaimed estates pool in the ledger's escheat wallet until Phase 6's
  treasury absorbs them (ADR 0007 §8a).

## Performance note (no budget applies until Phase 9)

80-citizen economic town: 100 simulated days ≈ 1.4 s release, ≈ 16 s
debug. Full workspace suite ≈ 100 s debug. Fine for the phase; Phase 8's
LOD tiers own the 10k-at-200-ticks/sec budget.
