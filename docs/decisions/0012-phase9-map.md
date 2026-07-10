# ADR 0012 — Phase 9 design: performance & the town map

- Status: Accepted
- Date: 2026-07-10
- Phase: 9

SPEC §15 Phase 9 ("Profile-guided optimization, spatial layout with
districts and travel times, commute costs entering utility and housing
prices. Exit: budgets met with headroom; location visibly stratifies
rents and shop revenues") against §4's `sim_world` charter ("town map,
buildings, housing, pathing (abstract)"). Per SPEC §16.7 the decisions
precede the code; per §16.3 each mechanic ships in its smallest honest
form the exit criteria exercise.

## 1. The map: districts and travel times

`data/map.ron`: an ordered district list (stable string ids → data
order indices, the SPEC §8 pattern) and a square travel-time MATRIX in
ticks (`travel_ticks[from][to]`; validated: square, symmetric, every
entry positive — the DIAGONAL is the intra-district trip, so there are
no teleports — and bounded by a day). The shipped matrix is calibrated
around the pre-map flat constant, so the map REDISTRIBUTES travel
rather than abolishing it. The map is the
whole spatial model — "pathing (abstract)" means the matrix IS the
path; no grid, no A*, no coordinates until the viewer wants pixels
(deferral documented; nothing in the exit criteria needs geometry).

`Sited { district: u32 }` (new shared component `"world.sited"`,
Sparse — location entities only; the district is a persisted data-order
index, so the district list only ever grows at the end, like every
other stable id list). Genesis sites every location round-robin by
entity order within its kind — deterministic, and it spreads homes and
venues across every district so the MATRIX (central vs peripheral),
not the placement rule, is what differentiates locations. The builder
sites each NEW home in the district with the fewest homes (tie: lowest
index) — deterministic supply spreading; demand-responsive siting is
later texture.

Siting happens AFTER every genesis pass that creates a location
(homes, venues, firms). Migrated pre-v10 worlds have no `Sited` rows
and no districts: every travel lookup falls back to the flat data
`travel_ticks` — exactly the pre-map behavior, nothing invented.

**Amended:** `HousingBook` also grows in v10 (per-district asks), so
`v9→v10` mechanically rewrites that one store — old fields verbatim,
an empty ask vector appended; the continuity proof checks the rewrite
row by row.

## 2. Commute costs in utility

`AiTables` gains the resolved matrix and a `district_of(location)`
lookup; `travel_between(from, to)` returns the matrix entry when both
ends are sited, else the flat fallback. Everywhere embodied behavior
previously used the flat constant now uses the pair lookup: decide-time
travel candidates (the SPEC §11 time cost — commute disutility enters
scoring with NO new tunable, through the existing
`time_cost_micro_per_tick`), actual `Travel`/`BuyTravel`/`WorkTravel`/
`SchoolTravel` durations, and the work-shift bias's feasibility.
**Amended — the coarse tiers pay too:** their shop choice adds the
commute's MONEY-equivalent (`commute_mills_per_tick × travel`) to the
believed price (embodied buyers pay the time in scoring; a coarse
buyer who paid nothing would shop at the far discount forever), and
Tier B's leisure hour loses the trip's minutes — the block includes
getting there, exactly the minutes an embodied citizen loses to the
same walk. Their presence WRITES stay abstract block moves.

## 3. Commute costs in housing prices

The rental clearing (ADR 0009 §3) previously matched homeless bidders
to vacant homes positionally against one global ask. Phase 9 makes the
market SPATIAL end to end (**amended in-phase to the shipped design;
each step was forced by measurement**):

- **The match**: each bidder, best bid first, takes the affordable
  vacant home with the lowest EFFECTIVE cost — the home's district ask
  + `commute_mills_per_tick × (travel(home, workplace) + ACCESS)`,
  where access is the mean travel from the home's district to every
  sited non-home location: a home is a base for every errand, not just
  the commute (without the access term, every worker simply prefers
  their workplace's own district and centrality never binds).
- **The price**: the rent midpoint takes the bidder's
  LOCATION-ADJUSTED willingness — every trip the home adds comes off
  what they will pay (`effective bid = bid − commute value`, floored
  at the ask). Commute costs enter housing prices through the tenant's
  own valuation, never as an assigned district premium.
- **The asks**: `HousingBook` grows per-district asks (data order),
  each steered by ITS OWN clearing outcomes: unsold inventory beside
  bidders THAT BUCKET priced out pushes down (the signal is
  per-bucket, attributed to each refused bidder's first-choice market
  — a global count would let one unhousable pauper anywhere cut every
  district's ask); chronic slack above the vacancy
  target pushes down; more first-choice bidders than offered homes
  (CONTENTION — the binary sold-out signal saturates when demand
  swamps every district) probes up; and a district with NOTHING
  offered HOLDS — a market with no inventory has no price to discover
  (without the hold, a fully-occupied district ratchets forever to
  numbers nobody pays). The global ask survives as the un-sited
  bucket's controller (the migrated world's whole market).
- The purchase market weighs the WORKPLACE COMMUTE in its buyer
  choice (jobless buyers take entity order) — narrower than the
  rental match's full effective cost: a purchase is priced by the
  double auction's midpoint, so only the choice, not the price,
  is spatial there.

`commute_mills_per_tick` is data (`map.ron`) — it converts commute
ticks into money the way the labor market's day wage implicitly prices
time. Stratification is EMERGENT from these mechanisms.

## 4. Save format v10

Append `world.sited` (component; no new events). FORMAT_VERSION 10,
mechanical v9→v10 chained from v1, byte-continuity proof, pinned
snapshot `data_v10` (gains `map.ron`), a v10 fixture with sited
locations and LIVE per-district asks asserted (every ask a real
price; cross-district stratification is the exit suite's claim, not
the fixture's), goldens re-recorded with reasons.

## 5. Performance: measured, then guarded

Measured BEFORE this phase (release, 10k citizens, shipped caps):
genesis 15 ms, first day ~850 ticks/sec, steady ~875 ticks/sec, week
catch-up 0.41 s — the Phase 8 budgets (200 t/s, < 5 s) already hold
with >4× headroom. "Profile-guided" therefore means (**amended** to
what the engine can measure today): the documented evidence is the
END-TO-END measurement — genesis, first-day, steady-state, and
catch-up throughput at 10k, before and after the map — because the
schedule exposes no per-system spans yet (per-system attribution is
deferred to the viewer phase's tracing work, documented). With >4×
headroom and no regression from the spatial lookups, no optimization
is justified (SPEC §16.3); the phase instead RAISES THE ASSERTED
FLOORS so the headroom is CI-guarded: the release suite asserts
≥ 300 ticks/sec at 10k WITH the map wired and a week's catch-up
≤ 2.5 s.

## 6. Data + validation

`data/map.ron`: `districts: [DistrictDef(id)]`,
`travel_ticks: [[u32]]`, `commute_mills_per_tick: i64`. Validation:
at least one district; matrix square (districts × districts),
symmetric, every entry ≥ 1 (the diagonal is the intra-district trip),
entries ≤ one day; commute rate ≥ 0. The flat `ai.travel_ticks` stays as the un-sited
fallback (and the un-mapped world's behavior).

## 7. Exit criteria mapping

- *Budgets met with headroom*: the release LOD suite's floors rise to
  300 t/s (10k, mapped town) and 2.5 s (week catch-up); the profile
  pass and its numbers go in the phase report.
- *Location visibly stratifies rents and shop revenues* (**amended**
  to the shipped proof): a land-constrained lifecycle town (no
  builder, small households, renter cohorts from zeroed-spark
  nest-leavers — all legal data) runs 48 fast-years under the daily
  audit and proves the REVENUE half (mean lifetime takings of center
  shops exceed the edge's — buyers pay real travel) plus a genuinely
  clearing rental market with per-district asks. The RENT half is
  proven by a CONTROLLED CLEARING over the same machinery — six
  identical homes across the districts, one bidder pool, one real
  clearing: center rents clear above edge rents and the deepest
  pockets take the best access. (A lifecycle town's rental flow
  concentrates where renters already live — edge owner-families never
  vacate on any humane horizon — so cross-district formation
  statistics come from the controlled experiment, and the lifecycle
  carries the audits, the revenue channel, and the market's
  liveness.)

## Dependencies

No new external dependencies.
