# Phase 5 report — Labor & payroll

- Date: 2026-07-10
- Scope: SPEC §15 Phase 5 (vacancies, job search, hiring, wages, payroll,
  firing, unemployment measurement; citizens earn instead of living on
  seeded wealth)
- Design decisions: ADR 0008 (written before the code)
- Verdict: **complete** — all three exit criteria demonstrated by
  CI-blocking tests; `scripts/check.sh` green.

## What was built

| Deliverable | Where |
|---|---|
| Every firm is a place (four new workplace location kinds, appended); production is labor-gated: a batch starts only with the kind's `min_workers` employees present, presence derived from `Position == employer` — never duplicated state | `crates/sim_economy/src/{systems,genesis}.rs`, `data/{firms,locations}.ron` |
| `Employment { employer, wage_per_day }` + `WorkingAge` marker + `LaborStats` shared carriers; `Hired`/`Fired` events | `crates/core_ecs/src/sim_interface*.rs` |
| The daily double auction (SPEC §12): firm bids = data share of the worker's expected daily marginal product, wallet-clamped; citizen asks = reservation wage (base, raised by wealth with the saturating half-wealth shape, discounted by industriousness); bids desc × asks asc, ties by entity index, wages clear at the midpoint | `crates/sim_economy/src/labor.rs::LaborMarketSystem` |
| Daily payroll: booked atomic wage transfers; a firm that cannot cover a wage FIRES instead of paying (`Insolvent`); headcount above positions fires extras (`Redundant`); nothing despawns — collapse leaves an honest husk the market routes around | `crates/sim_economy/src/labor.rs::PayrollSystem` |
| Work behavior: `WorkTravel`/`Work` appended action variants; a strong data-defined shift bias (obligations bias, never dictate — a starving worker still eats); working satisfies `purpose` a little; the wage only ever arrives via payroll | `crates/sim_ai` |
| Labor-force boundary: `WorkingAge` stamped at genesis and promoted on the birthday crossing the data threshold, by `sim_people` (where age lives) — the market never reads `Identity` | `crates/sim_people/src/{genesis,systems}.rs` |
| Measurement, never assignment: `LaborStats` (working-age, employed, seeking, unmatched, lifetime hires/firings) written by each clearing; surfaced in the `economy` report and new stats-CSV columns | `crates/headless/src/{inspect,cli}.rs` |
| Data + validation: `balance/labor.ron`; firms gain `location_kind_id`/`positions`/`min_workers` (validated: exists, count 0, claimed once, `1 ≤ min_workers ≤ positions`); shift/reservation/bid ranges checked | `crates/data_defs` |
| Save format v6: three appended components + two events, pure `v5→v6`, chain from v1, continuity proofs for v2–v5 fixtures, new v6 fixture with live employment; goldens re-recorded under `data_v6` per policy | `crates/persistence`, `tests/tests/save_compat.rs` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 5) | Test |
|---|---|
| Labor market clears | `labor::labor_market_clears_when_workers_outnumber_positions` (every slot fills; the surplus is measured unmatched) and `…when_positions_outnumber_workers` (everyone working-age hired; unemployment measures zero) — both regimes, data-shaped towns |
| Wage distribution emerges | `labor::a_wage_distribution_emerges` — ≥5 distinct cleared wages across ≥20 workers with a ≥20% spread, driven by heterogeneous reservations and marginal products; dispersion also unit-asserted at the clearing |
| A firm's collapse produces measurable local unemployment | `labor::a_firm_collapse_produces_measurable_unemployment` — bakeries with a day of capital, shelves and inputs burned through the counted sink: payroll drains the till, everyone is fired (`LaborStats.firings` covers the whole workforce), the husks employ nobody, the unmatched count spikes, and conservation still audits green |

Also this phase:

- Wages close the loop: `labor::wages_flow_from_firms_to_workers` (worker
  wallets exceed anything the seed could grant, conservation intact).
- Payroll fires the unpayable with the `Insolvent` fact; the clearing
  matches, measures, and prices at the unit level
  (`sim_economy::systems_tests`).
- Unstaffed firms never start batches; input starvation still idles
  honestly; the cash bound on trade still binds (unit suites).
- The whole Phase 3/4 exit suites still pass on the labor economy: the
  town feeds itself with production now gated on real workers showing up.

## Verification process

`scripts/check.sh` green at the tagged commit (fmt, clippy `-D warnings`,
full workspace suite — 150+ tests including the 1M-tick, 10k-citizen,
100-day-conservation, and labor suites). Adversarial multi-agent review
(ultracode): three consolidated reviewers (determinism+conservation,
standards+anti-shortcut/ADR-fidelity, test-coverage) over the phase diff,
findings verified against the code before acting.

Confirmed findings, all fixed in the follow-up commits:

- **Migrated v5 economies never ran the labor systems** (major): payroll
  and the clearing gated on `LaborStats`, which only fresh genesis
  creates — with production now labor-gated, a migrated town's firms
  would idle forever, contradicting the documented catch-up. The labor
  systems now key off `EconCounters` like every other economy system and
  create the stats row on first use;
  `save_compat::v5_migrated_economy_catches_up_with_the_labor_market`
  proves the catch-up semantically (no invented state at load; real
  hires after the first day boundary). The v5 resume golden re-recorded
  with this reason.
- **Five modules crossed the SPEC §3 500-line rule** (major): split
  (migration tests, seeded-error validation tests, CLI tests,
  `PlanSystem`, and the economy transaction helpers each into their own
  files).
- **Bid affordability ignored committed payroll** (minor): firms could
  deterministically hire into a guaranteed next-morning insolvency
  firing, pumping the hire/fire counters. The clamp now reserves
  incumbents' wages before funding open slots (ADR 0008 §3 amended);
  unit-tested.
- **Unchecked add/sub amid checked multiplications** in the reservation
  and wage-midpoint arithmetic (minor): now checked.
- **Coverage gaps** (one major, several minor), all closed with direct
  tests: the v6 fixture is now saved MID-SHIFT with `Work`/`WorkTravel`
  asserted in flight (and resumes across a payroll boundary); redundancy
  firing; the reservation formula's wealth/trait terms, value by value;
  the committed-payroll bid clamp; the `min_workers ≥ 2` staffing
  boundary; the WorkingAge birthday promotion; and deaths reopening
  slots that the market then refills near capacity with conservation
  intact.

Verified clean by the reviewers (recorded): deterministic sorts and
tie-breaks throughout the auction; clean despawn of employed citizens
(estate settles, payroll never sees a dead roster entry); correct enum
variant appends; layering (labor reads only `sim_interface` types);
no silent fakes (wages only via booked payroll, hires only via the
clearing, the Work act produces nothing itself, unemployment measured
never assigned); registration order matching ADR 0008 §8 exactly.

## Deviations from spec (all pre-declared in ADRs)

- One town-wide shift (per-firm shifts are later texture); no overnight
  shifts (validated).
- Skills and skill requirements arrive with Phase 7's education pipeline;
  Phase 5 wage heterogeneity comes from wealth/trait reservations and
  per-firm marginal products.
- Firm exit/bankruptcy is Phase 6 — a collapsed firm is a husk, not a
  despawn.
- Rebalanced per-batch outputs (bread, firewood) compensate for
  production now living inside staffed shift hours (ADR 0008 §1; data
  edits only).

## Known limitations (clean seams, not fakes)

- The bid ignores already-committed payroll (it clamps to
  `wallet/positions` only) — an aggressive firm can hire into
  insolvency and fire the next day; honest, measured, and the seam
  Phase 6 credit will price properly.
- Children and retirees are simply outside the labor force; pensions,
  education, and dependency arrive with Phases 6–7.
- No job quitting or on-the-job search: separations are firings only
  until Phase 7's richer decision layer.

## Performance note (no budget applies until Phase 9)

150-citizen labor town: 5 simulated days ≈ 0.6 s release, ≈ 4.4 s debug.
Full workspace suite ≈ 120 s debug. Phase 8's LOD tiers own the
10k budget, as designed.
