# ADR 0011 — Phase 8 design: LOD tiers & catch-up

- Status: Accepted
- Date: 2026-07-10
- Phase: 8

SPEC §15 Phase 8 ("DayModels, Tier B/C execution, promotion/demotion,
catch-up controller. Exit: 10k citizens hit the 200 ticks/sec budget;
A↔C cycling conserves money exactly; catch-up of one week < 5s; macro
time-series statistically indistinguishable (defined tolerance) between
all-Tier-A small towns and tiered ones") against §10's tier
definitions and §5's catch-up contract. Per SPEC §16.7 the decisions
precede the code; per §16.3 each mechanic ships in its smallest honest
form the exit criteria exercise.

## 1. The tier component and membership

`LodTier { tier: Tier }` with `Tier { A, B, C }` (shared component
`"lod.tier"`, Dense — every citizen has one; append-only enum). A
MISSING row means Tier A: migrated pre-v9 citizens keep exactly their
old behavior until the first assignment pass stamps them (migrations
never invent state).

Membership is assigned by a day-rate `TierAssignSystem` (first among
the day systems, before the audit), deterministic and in entity order
(SPEC §10 "rule-based on focus distance and event flags"):

- **Focus**: headless has no player, so focus is the STABLE FRONT of
  the town — the lowest-index citizens fill Tier A up to
  `tier_a_cap`, then Tier B up to `tier_b_cap`, the rest are C (caps
  in `data/balance/lod.ron`). The viewer (Phase 10) may later replace
  the focus rule; the CONTRACT (deterministic, entity-order, capped)
  is what Phase 8 ships.
- **Event flags**: a citizen touched by a high-signal fact (fired,
  loan defaulted, married, born to, widowed) is PINNED to Tier A for
  `highlight_days`, ahead of the focus fill (the cap still binds;
  pins beyond the cap wait in entity order). This is SPEC §10's
  "currently involved in a high-signal situation". Pins are a
  persisted component (`Spotlight { until_tick }`, `"lod.spotlight"`)
  written by a tick-rate reader of the event bus — the facts are
  emitted by day-rate systems, so they are readable exactly one tick
  after the day boundary and the reader is O(readable events) on all
  other ticks; a system-internal pin map would not survive save/load.

## 2. What each tier runs

Money is NEVER modeled: wallets, deposits, employment, tenancies,
ownership stay live components at every tier, and every economy
day-system (payroll, rent, labor clearing, markets, bank, taxes)
remains tier-agnostic — a Tier C citizen is paid through the same
payroll transfer and pays the same rent row as a Tier A one. This is
what makes A↔C cycling conserve money EXACTLY: there is nothing to
copy back.

- **Tier A** (per-tick): unchanged — DecideSystem/ActSystem skip
  citizens whose `LodTier` is B or C.
- **Tier B** (per-hour, `TierBSystem` in `sim_ai`): executes the day
  as blocks from the SAME obligations Tier A's biases read — the sleep
  window (home, analytic rest gain), the work shift when employed (at
  the workplace; the work need integrates analytically), school hours
  when school-age (the data gain once per day, `SchoolAttended`
  emitted), else leisure at the venue best satisfying the most
  deficient need (Position moves there — Tier B citizens genuinely
  appear at venues, so the Phase 7 social hour keeps drifting bonds
  over them). One REAL retail purchase resolves per hour when the
  block's need has a retail satisfier and the deficit exceeds one
  unit's gain (the shared purchase path: stock, till, tax, beliefs,
  counters). Needs gains are the venue's per-tick rate × 60, exact
  integer, applied once per hour.
- **Tier C** (per-day, `TierCSystem` in `sim_ai`): each citizen's
  `DayModel` executes once per day: per need in data order, buy the
  day's deficit from the cheapest KNOWN offer (believed price, posted
  fallback — the same knowledge Tier A scores with) unit by unit
  through the shared purchase path (real stock, real money, real tax —
  SPEC §10 "their demand aggregates into market orders tagged with
  their identity"), then satisfy the residual with the home/venue
  rates the model carries (abstract presence — Position stays home).
  School-age C citizens attend statistically: the school gain lands
  once per school day. Employment, payroll, rent, mortality,
  fertility, marriage all run unchanged (they are day-rate already).

## 3. The DayModel

`DayModel` (shared component `"lod.day_model"`, Sparse — only B/C
citizens carry one): per-need planned daily gain from passive sources
(computed from the data satisfier tables: the home's rates over the
sleep window, the default venue's rates over leisure hours). The LIVE
`Needs` row remains the trajectory's state — the model never shadows
it (one source of truth; the hourly decay system and the tier
integrators write the same row every tier reads). Derived at demotion
and refreshed daily; it holds NO money and NO inventory.

## 4. Promotion and demotion

Both happen inside `TierAssignSystem`, in entity order, at the day
boundary:

- **Demote A→B/C**: remove the embodied rows (`CurrentAction`,
  `Decider` dump, `DailyPlan`), write the `DayModel` from the live
  needs, park the citizen at home (their Residence, else their
  current Position). Wallet, employment, tenancy untouched.
- **Promote C/B→A**: drop the `DayModel`, leave needs exactly as the
  coarse integrator last wrote them (the model IS the trajectory —
  materialization is reading it), place the citizen at home or, mid
  work shift with a job, at the workplace; no `CurrentAction` — the
  next tick's DecideSystem decides from the materialized state, which
  is the same code path a fresh Tier A citizen takes.
- `TierChanged { citizen, from, to }` is emitted for observability.

An A→C→A cycle therefore conserves money EXACTLY (never copied) and
needs within the coarse integrator's tolerance — asserted by the exit
suite.

## 5. The catch-up controller

`sim_time::Simulation::run_ticks_coarse(schedule, ticks)` (SPEC §5):
advances the clock in HOUR strides — tick-rate systems are skipped,
hour-rate systems run once per stride, day/season/year systems run at
their boundaries, exactly as the normal loop would order them. The
runner's `catch_up(sim, schedule, days)` wraps it: demote every
citizen to Tier C (through the same demotion path), run the coarse
loop, and let the next day-boundary assignment restore tiers. Catch-up
is a DEFINED coarse integrator: deterministic (same state + same days
→ same result, a CI test), but not tick-equivalent — that is §5's
contract ("replaced by each agent's statistical day model", never "a
skip"). Surfaced as `embervale run --catch-up-days N` (applied before
the per-tick run) so a loaded save can fast-forward.

## 6. Save format v9

Components (append): `lod.tier`, `lod.day_model`, `lod.spotlight`.
Events: `lod.tier_changed`. FORMAT_VERSION 9, mechanical v8→v9 chained from
v1, continuity proofs, goldens re-recorded with reasons, a v9 fixture
saved mid-flight with mixed tiers, pinned snapshot `data_v9` (gains
`balance/lod.ron`). Migrated worlds carry no tier rows: every citizen
runs Tier A (the pre-v9 status quo) until the first day boundary
stamps assignments — nothing invented, behavior changes only where
the new data says so.

## 7. Data

`data/balance/lod.ron`: `tier_a_cap`, `tier_b_cap`, `highlight_days`,
`leisure_hours_per_day` (the model's default leisure block), and
`macro_tolerance_per_mille` (the exit criterion's "defined tolerance"
— it is an acceptance band, so it is data, like the Phase 2
demographics bands). Validation: caps and windows positive,
tolerance ≤ 1000, leisure hours < 24.

## 8. Exit criteria mapping

- *10k citizens hit the 200 ticks/sec budget*: a release-only test
  (`#[cfg(not(debug_assertions))]`) builds a 10k-citizen town with the
  shipped caps, runs a fixed span, and asserts ≥ 200 ticks/sec;
  `check.sh` gains a release-mode run of the LOD suite so the gate
  enforces it (debug builds skip the timing assert, never the
  correctness asserts).
- *A↔C cycling conserves money exactly*: force a demote-promote cycle
  across days (tiny caps make it happen naturally); assert the cycled
  citizen's wallet+deposit changed ONLY by the real flows the ledger
  recorded (wages, rent, purchases), the daily audit green throughout,
  and needs within the data tolerance after re-promotion.
- *Catch-up of one week < 5s*: release-only timing assert around
  `catch_up(…, 7 days)` at 10k citizens; plus (all profiles) the
  coarse integrator's determinism — same save caught up twice gives
  identical hashes — and conservation audits green after catch-up.
- *Macro time-series indistinguishable*: twin towns, same seed and
  data, one with caps ≥ population (all-A), one with tiny caps (mixed
  tiers), run N days; per-day series of employment rate, mean posted
  price index, treasury receipts, and total citizen money must agree
  within `macro_tolerance_per_mille` on average (the defined
  tolerance, in data).

## 9. Deferrals (documented)

- Tier C relationship drift: Tier C citizens do not visit venues, so
  their drift pauses (kin/spouse edges persist; decay pauses with
  drift — `RelationshipDecaySystem` skips C citizens so absence from
  the embodied world does not dissolve bonds the model cannot rebuild).
  The exit criteria measure ECONOMIC macro series; social-graph LOD
  fidelity is Phase 9+ texture.
- Real focus (player camera) arrives with the viewer (Phase 10); the
  assignment CONTRACT ships now.
- Deterministic parallelism stays out (SPEC §6: design permits, Phase 8
  does not build it).

## Dependencies

No new external dependencies.
