# Phase 9 report — performance & the town map

- Date: 2026-07-10
- Scope: SPEC §15 Phase 9 (profile-guided optimization, spatial layout
  with districts and travel times, commute costs entering utility and
  housing prices) against §4's `sim_world` charter
- Design decisions: ADR 0012 (written before the code; §§1–3, 5, 7
  amended in-phase, each deviation documented before the deviating
  code — the housing-market amendments were forced by measurement, see
  below)
- Verdict: **complete** — both exit criteria demonstrated by
  CI-blocking tests; `scripts/check.sh` green.

## What was built

| Deliverable | Where |
|---|---|
| The map: `data/map.ron` — an ordered district list and a symmetric door-to-door travel matrix in ticks (the DIAGONAL is the intra-district trip: no teleports; the matrix IS the path — no grid, no A*, no geometry until the viewer wants pixels); the shipped map has a genuine CENTER (`old_town`) and a REMOTE edge (`outfields`) | `data/map.ron`, `crates/sim_world/src/config.rs::MapConfig` |
| Siting: `Sited { district }` (persisted data-order index) on every location — genesis round-robins WITHIN each kind after every location-creating pass, so the matrix, not placement, differentiates; the builder sites new homes in the least-built district | `crates/core_ecs/src/sim_interface.rs::Sited`, `crates/sim_world/src/genesis.rs::site_locations`, `crates/sim_economy/src/systems.rs` |
| Commute costs in utility: every embodied travel decision and duration uses the pair lookup (`travel_between`, flat fallback for un-sited ends) — decide-time scoring pays the commute through the EXISTING time-cost term (no new tunable), and travel actions genuinely take the matrix's ticks | `crates/sim_ai/src/{config,systems,systems_score}.rs` |
| The coarse tiers pay too: Tier B/C shop choice adds the commute's money-equivalent to the believed price, and Tier B's leisure hour loses the trip's minutes — the macro-twin criterion stays inside its band WITH the map wired | `crates/sim_ai/src/systems_lod_tiers.rs` |
| Commute costs in housing prices: the rental match is spatial (district ask + commute + ACCESS — the mean travel to every sited destination: a home is a base for every errand); the rent midpoint takes the bidder's LOCATION-ADJUSTED willingness (every trip the home adds comes off what they pay — never an assigned premium); `HousingBook` grows per-district asks steered by each district's own clearing OUTCOMES (unsold-beside-priced-out pushes down, contention probes up, no inventory HOLDS — a market with nothing offered has no price to discover); the purchase market weighs the workplace commute in its buyer choice (the double auction's midpoint keeps pricing the sale) | `crates/sim_economy/src/housing.rs`, `crates/core_ecs/src/sim_interface_money.rs` |
| Save format v10: `world.sited` appended AND the housing book's shape grew — `v9→v10` mechanically rewrites that one store (old fields verbatim, empty ask vector appended; the continuity proof checks the rewrite row by row); a 2,500-citizen v10 fixture on the shipped map (sited town, live district asks, green audits — all asserted); goldens re-recorded per policy | `crates/persistence`, `tests/tests/save_compat.rs` |
| Data + validation: map bounds (square, symmetric, every entry ≥ 1, ≤ a day, unique ids, the commute rate inside an overflow ceiling) rejected at load with precise messages, rejection-tested | `crates/data_defs/src/validate_map.rs`, `crates/data_defs/src/tests.rs` |
| Performance floors raised and CI-guarded: ≥ 300 ticks/sec at 10k WITH the map wired, week catch-up ≤ 2.5 s (release suite) | `tests/tests/lod.rs` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 9) | Test |
|---|---|
| Budgets met with headroom | `lod::ten_thousand_citizens_hit_the_tick_budget` (≥ 300 t/s at 10k, 1.5× the SPEC §10 budget) and `lod::a_week_of_catchup_fits_the_five_second_budget` (≤ 2.5 s, 2× the SPEC §15 budget) — both release-mode, both WITH the map's spatial lookups live. Measured (release, 10k): BEFORE the map — genesis 15 ms, 852/875 ticks/sec (first day/steady), week catch-up 0.41 s; AFTER the map — genesis 15 ms, 846/865 ticks/sec, catch-up 0.46 s. The spatial lookups cost ~1% and the Phase 8 budgets already held >4×, so no optimization was justified (SPEC §16.3) and the phase GUARDS the headroom instead (per-system attribution deferred to the viewer phase's tracing — ADR 0012 §5 as amended) |
| Location visibly stratifies rents and shop revenues | `map::location_stratifies_rents_and_shop_revenues` — a land-constrained lifecycle town (no builder, small households, zeroed-spark renter cohorts — all legal data) runs 48 fast-years under the daily audit: center shops' mean lifetime revenue exceeds the edge's (buyers pay real travel), the rental market genuinely clears, and per-district asks are live; `map::a_controlled_clearing_prices_the_center_above_the_edge` — six identical homes across the districts, one bidder pool, ONE real clearing: center rents clear above edge rents and every center tenant out-bids every edge tenant (assortative access) |

Also this phase:

- The macro-indistinguishability, cycling, catch-up, social,
  money, and labor suites all still pass over the mapped town — the
  spatial layer changed trajectories, not invariants.
- Migrated pre-v10 worlds are provably non-spatial: the v9 fixture
  loads with zero `Sited` rows (asserted) and every travel lookup falls
  back to the flat constant — the exact pre-map behavior.

## In-phase design amendments (ADR 0012, each written before the code)

The housing-market §3 went through four measured iterations — each
failure is documented in the ADR because the failures are the design
rationale:

- A single global ask cannot stratify: realized-rent midpoints track
  bids and timing, not location.
- Per-district asks steered by vacancy RATIOS ratchet unboundedly in a
  supply-constrained district (zero vacancy → infinite price with zero
  transactions): the shipped controller steers on clearing OUTCOMES
  and HOLDS with no inventory.
- Binary sold-out signals saturate when demand swamps every district:
  the shipped probe uses CONTENTION (first-choice bidders vs offered
  homes).
- Workplace commute alone makes every worker prefer their workplace's
  own district (centrality never binds): the shipped match adds the
  ACCESS term, and the shipped price takes the tenant's
  location-adjusted willingness.

## Verification process

`scripts/check.sh` green at the tagged commit (fmt, clippy `-D
warnings`, full workspace suite, release LOD + map suites).
Adversarial multi-agent review (ultracode): five reviewers over the
phase diff, findings verified against the code before acting.

Confirmed review findings (five reviewers: conservation/atomicity,
determinism/ordering, save-format v10, SPEC+ADR conformance, edge
cases/failure modes), all fixed in the review commit:

- **Two suites were red on the review tree** (critical): the rental
  clearing's new `Sited` reads broke a `sim_economy` unit world and
  the flag-less CLI resume test still pointed at the pre-map frozen
  snapshot (which the loader now rightly refuses — `map.ron` is
  mandatory). Fixed: the test world registers `Sited`; the resume test
  pins `data_v10`; the stale `data_v9` snapshot directory is gone.
- **Unchecked commute arithmetic + no rate ceiling** (major): a
  validation-legal `commute_mills_per_tick` near `i64::MAX` made every
  clearing overflow. Fixed: the rate has an overflow ceiling (the
  bank-rate rule) and every commute computation saturates; a corrupt
  save's negative district ask is now sanitized like the global ask
  (it could sign a REVERSED rent stream).
- **The global priced-out brake defeated the per-district hold band**
  (major): one structurally unhousable pauper anywhere cut EVERY
  district's ask each day. Fixed: the priced-out signal is per-bucket,
  attributed to each refused bidder's first-choice market.
- **The ask controller had no behavioral test** (major): `steer_ask`
  is now a free function with every branch bitten (hold without
  inventory, both down-brakes, the contention probe, the floor).
- **No byte-continuity proof for v9** (major): the first
  store-rewriting migration shipped without the proof that justifies
  the golden re-record. Fixed: the v9 fixture's migration is checked
  row by row (housing book mechanically grown, events byte-identical).
- **The ADR's profile-pass promise was undeliverable** (major): the
  schedule exposes no per-system spans; §5 is amended to the
  end-to-end before/after measurement the report now carries
  (852/875 → 846/865 t/s at 10k: the map costs ~1%).
- **Module size** (major): `validate_map` moved to its own file.
- Docs and honesty minors: stale "zero diagonal" invariants corrected
  (the validator requires ≥ 1 everywhere); the purchase market's
  claim tightened to what it does (workplace-commute choice only);
  the v10 fixture now asserts every district ask is a real price and
  the ADR says exactly that; the migration chain doc and
  FORMAT_VERSION history extended to v10; the save-compat header's
  snapshot reference fixed; the lifecycle exit test's comments now
  claim only what it asserts (dead accumulation and probe scaffolding
  deleted); dead per-district formation counters removed; the rental
  match comment documents the ACCESS term.

## Deviations from SPEC

- "Pathing (abstract)" ships as the district matrix — no grid or
  coordinates until the viewer needs pixels (ADR 0012 §1).
- Tier B/C presence stays abstract block moves; their commute costs
  enter as money-equivalents and lost block minutes rather than
  tick-by-tick travel (ADR 0012 §2).
- The lifecycle rent criterion is carried by a controlled clearing
  over the real machinery: a lifecycle town's rental flow concentrates
  where renters already live, so cross-district formation statistics
  cannot be sampled from one seed on any humane horizon (ADR 0012 §7).

## Tag

`phase-9` (local; tag pushes are blocked by the environment's proxy —
the branch carries the content).
