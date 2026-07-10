# ADR 0008 — Phase 5 design: labor, wages, payroll, unemployment

- Status: Accepted
- Date: 2026-07-10
- Phase: 5

SPEC §15 Phase 5 ("Vacancies, job search, hiring, wages, payroll,
firing, unemployment measurement. Citizens earn instead of living on
seeded wealth. Exit: labor market clears; wage distribution emerges; a
firm's collapse produces measurable local unemployment") and §12's labor
bullet underdetermine mechanics; per SPEC §16.7 the decisions precede
the code.

## 1. Every firm is a place; production takes people

Phase 4 firms produced unmanned — the honest Phase 5 correction is that
batches need workers. Every firm kind now names a `location_kind_id`
(new kinds `farm, well_site, forest_camp, mill` join `bakery, woodyard`;
all count 0 — firm genesis creates the instances, appended at the end of
`locations.ron` per the stable-id rule). `ProductionSystem` starts a
batch only when, in that hour, at least the kind's data-defined
`min_workers` employees are AT the firm working (the `Work` action
below). Running batches always finish — labor gates starts, not
completions. Firms without enough labor idle honestly: no output, no
input consumption, shelves drain, prices rise — which is exactly the
"collapse is measurable" seam the exit criterion wants.

Balance consequence (data edits): production now happens only during
staffed shift hours instead of around the clock, so per-batch outputs in
`recipes.ron` are rebalanced upward (bread, firewood) to keep the town
fed at the same firm counts — tunables moving in data, as SPEC §8
intends.

## 2. Employment and the shift

`Employment { employer: Entity, wage_per_day: Money }` (shared,
`sim_interface`) on citizens. The workday is one data-defined shift
(`labor.ron`: `shift_start_hour`, `shift_end_hour`, same for everyone —
per-firm shifts are texture for later phases). During the shift, the AI
scores a `Work` candidate for employed citizens with a strong
data-defined bias (like the sleep bias — obligations bias, they don't
dictate: a starving worker still eats first, SPEC §11). `CurrentAction` gains two
appended variants, mirroring the purchase pair: `WorkTravel { target,
remaining }` and `Work { at, remaining }`. Working satisfies the
data-defined need (`purpose`, small rate) — the wage is paid by payroll,
never by the act. Worker *presence* (what gates production) is derived,
never duplicated: an employee counts as present when their `Position` is
their employer's entity — both components shared, so `sim_economy` never
reads `sim_ai` state (SPEC §4).
Citizens younger than `labor.ron`'s `min_working_age_years` are outside
the labor force. Age lives in `sim_people`'s `Identity`, which
`sim_economy` may not read (SPEC §4) — so eligibility crosses the crate
boundary as a shared `WorkingAge` marker (`sim_interface`,
`people.working_age`): genesis stamps it on citizens already of age, and
a day-rate `sim_people` system promotes citizens on the birthday they
cross the threshold (the threshold and year length injected by the
application, like mortality's).

## 3. The market: one daily clearing

SPEC §12 assigns labor the daily double-auction. Day-rate
`LaborMarketSystem` (in `sim_economy`), after payroll:

- **Bids (firms):** each firm with `employees < positions` (positions
  per kind, data) posts one bid per open slot at its **bid wage**: the
  worker's expected daily marginal product — output units per worker-day
  at the current posted price — times a data-defined
  `bid_fraction_per_mille` (what the firm keeps as margin), clamped to
  what its wallet could actually pay for a day.
- **Asks (citizens):** every working-age, unemployed citizen asks their
  **reservation wage**: data base value scaled by wealth (the same
  marginal-utility-of-wealth shape purchases use: the richer, the
  choosier) and lowered by industriousness (per-mille weight, data).
  Integer arithmetic throughout; floats stay in AI scoring only.
- **Clearing:** bids sorted descending, asks ascending (ties: entity
  index — price-time priority with genesis order as "time"). Match while
  `bid >= ask`; the wage is the midpoint rounded down to whole mills.
  Matches create `Employment`, emit `Hired`, and count toward that day's
  employment. Unmatched asks are that day's measured unemployment.

## 4. Payroll, firing, collapse

Day-rate `PayrollSystem` (before the market, after the audit): each firm
pays each employee `wage_per_day` — an atomic transfer (wallet↔wallet,
booked as firm expense; ADR 0007 §4 discipline). A firm that cannot
cover a wage **fires instead of paying** (removes `Employment`, emits
`Fired { reason: Insolvent }`) — employees in entity order, so the
shortfall lands deterministically. Firms also fire down to `positions`
if data shrinks (defensive; reason `Redundant`). Nothing despawns:
Phase 6 owns bankruptcy/exit. A collapsed firm (no cash, no workers,
no production) sits as a husk that the market routes around — its former
workers re-enter the next clearing.

## 5. Measurement: counted, never assigned

`LaborStats` (shared component on the ledger entity, `econ.labor_stats`):
written by the market each clearing — `working_age`, `employed`,
`seeking` (asked this clearing), `unmatched` (seeking minus hired), plus
lifetime `hires`/`firings`. The inspector's `economy` report and the
stats CSV surface it; the unemployment RATE is derived by observers,
never stored (measured, not assigned — SPEC §12).

## 6. Wages close the loop

Citizens now earn (wages in) and spend (purchases out); firms earn
(sales) and spend (inputs, payroll). Seeded wealth becomes starting
capital instead of a lifetime allowance. Conservation is untouched:
wages are transfers, `Σ wallets == issued` holds, and payroll is booked
so the per-firm ledger identity keeps closing (wage expense against
sales revenue).

## 7. Data

`data/balance/labor.ron`: shift hours, `min_working_age_years`,
reservation base/wealth-shape/trait weight, `bid_fraction_per_mille`,
work-need satisfaction rate, work bias (micro), work action tick length.
`data/firms.ron` gains per kind: `location_kind_id` (mandatory now),
`positions`, `min_workers`. `data/locations.ron` appends the four new
workplace kinds (count 0, no satisfiers). Validation: every firm kind's
location kind exists with count 0; `min_workers <= positions`;
`positions >= 1`; shift/reservation/bid parameters in range; workplace
location kinds are claimed by exactly one firm kind (the retail rule
generalizes: a count-0 kind must be claimed by a firm).

## 8. Save format v6

Registration append: components `econ.employment, econ.labor_stats,
people.working_age`; events `econ.hired, econ.fired`. FORMAT_VERSION 6, mechanical v5→v6
(append stores + extend event names), chained from v1, goldens
re-recorded with continuity proofs (the schedule and balance changed —
old-fixture resume trajectories move), new v6 fixture with live
employment, pinned snapshot becomes `data_v6`. `CurrentAction::Work` is
an appended enum variant (old saves decode unchanged); `DailyPlan` stays
frozen — the work obligation derives from `Employment` + data, not from
a struct change.

## 9. Exit criteria mapping

- *Labor market clears*: within days of genesis, employment fills the
  smaller of positions and working-age supply; both surplus regimes
  (excess workers → unmatched asks measured; excess positions → unfilled
  vacancies) asserted with data-shaped towns.
- *Wage distribution emerges*: cleared wages show real dispersion
  (distinct wage levels across firms/citizens), driven by heterogeneous
  reservations and marginal products — asserted on the wage multiset.
- *A firm's collapse produces measurable local unemployment*: seeded
  shock — a firm's stock destroyed through the counted sink for several
  days (ADR 0007 §8 path) — drains its cash into payroll until it fires
  everyone; `LaborStats` and `Fired` events show the spike, and the
  survivors' re-matching (or measured unemployment, if no vacancies
  remain) shows the market routing around the husk.

## Dependencies

No new external dependencies.
