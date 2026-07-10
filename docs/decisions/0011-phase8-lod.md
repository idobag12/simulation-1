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
  loan defaulted, married, born to) is PINNED to Tier A for
  `highlight_days`, ahead of the focus fill (the cap still binds;
  pins beyond the cap wait in entity order). This is SPEC §10's
  "currently involved in a high-signal situation". Pins are a
  persisted component (`Spotlight { until_tick }`, `"lod.spotlight"`)
  written by a tick-rate reader of the event bus — the facts are
  emitted by day-rate systems, so they are readable exactly one tick
  after the day boundary and the reader is O(readable events) on all
  other ticks; a system-internal pin map would not survive save/load.
  Pins land on CITIZENS only — firms borrow and default too, and a
  firm must not carry citizen-LOD state. **Amended:** widowhood does
  NOT pin — `PersonDied` is a `sim_people`-local event carrying no
  spouse linkage, and `sim_ai` cannot read it; a widowhood fact can
  join the pin set if a later phase promotes one to the shared bus.

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
  over them). One REAL retail purchase may resolve per hour, for the
  most deficient need RETAIL can serve (which need not be the venue
  need — a hungry citizen buys bread even when loneliness is deeper),
  when at least half a unit's gain lands (round-to-nearest coverage,
  like the embodied buyer whose clamped gain wastes the unit's tail).
  **Amended:** a successful purchase hour replaces the venue visit —
  the buyer stands at the till (observable presence); a failed one
  (empty shelf, short wallet) falls through to the venue instead.
  Needs gains are the venue's per-tick rate × 60, exact integer,
  applied once per hour; the sleep block's home rates require an
  actual Residence (an evictee sleeping rough gains nothing —
  demotion must not repeal homelessness).
- **Tier C** (per-day, `TierCSystem` in `sim_ai`): each citizen's
  `DayModel` executes once per day: per need in data order, buy the
  deficit from the cheapest KNOWN offer (believed price, posted
  fallback — the same knowledge Tier A scores with) unit by unit,
  while at least half a unit's gain lands, through the shared purchase
  path (real stock, real money, real tax — SPEC §10 "their demand
  aggregates into market orders tagged with their identity"), then
  satisfy the residual with the home/venue rates the model carries
  (abstract presence — Position parks at home, or NOWHERE for the
  roofless: a ghost at their last venue would keep meeting the social
  hour forever). School-age C citizens attend statistically: the
  school gain lands once per school day. **Amended:** employed Tier C
  citizens STAND AT THEIR WORKPLACE through the data shift (two
  Position writes a day) — production gates batch starts on PRESENT
  workers, and statistical labor is still real labor (SPEC §10 "labor
  supplied"); without it a mostly-coarse town's supply collapses and
  its prices climb away from the embodied twin's. Employment, payroll,
  rent, mortality, fertility, marriage all run unchanged (they are
  day-rate already).

## 3. The DayModel

`DayModel` (shared component `"lod.day_model"`, Sparse — **amended:**
only Tier C citizens carry one; Tier B recomputes its blocks from the
tables hourly, and persisted state nothing reads would be dead weight
in every save and hash): per-need planned daily gain from passive
sources — the home's rates over the sleep window (under a REAL roof
only), and ONE leisure venue's rates over the data leisure block (the
kind best satisfying the citizen's most deficient need at derivation;
a per-need best-venue sum would credit the same minutes once per need
— a citizen present everywhere at once, over-satisfied days, and
structurally deflated Tier C retail demand), plus the work-need gain
over the shift when employed. The LIVE `Needs` row remains the
trajectory's state — the model never shadows it (one source of truth;
the hourly decay system and the tier integrators write the same row
every tier reads). Derived at demotion and refreshed daily; it holds
NO money and NO inventory.

## 4. Promotion and demotion

Both happen inside `TierAssignSystem`, in entity order, at the day
boundary:

- **Demote A→C**: remove the embodied rows (`CurrentAction`, the
  decision dump, `DailyPlan`), write the `DayModel`, park the citizen
  at home (roofless citizens park NOWHERE). **Amended — A→B**: the
  `DailyPlan` STAYS (Tier B executes the plan's sleep window) and no
  model is written (§3). Wallet, employment, tenancy untouched.
- **Promote C/B→A**: drop the `DayModel`, leave needs exactly as the
  coarse integrator last wrote them (the model IS the trajectory —
  materialization is reading it), place the citizen at home; no
  `CurrentAction` — the next tick's DecideSystem decides from the
  materialized state, which is the same code path a fresh Tier A
  citizen takes. **Amended:** promotion happens ONLY at the day
  boundary (the assignment is a day-rate system), where home-at-
  midnight IS the plausible instantaneous state — so SPEC §10's
  "deterministically from their RNG stream" needs NO draw (there is
  nothing stochastic left to sample: needs are live, the position is
  home, the activity is the next tick's real decision). A mid-day
  promotion path (viewer focus) would materialize time-of-day state
  and may draw then; it does not exist in Phase 8.
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

**Amended details (review-driven):**

- Scheduled bus entries are DEFERRED across the span
  (`begin_tick_coarse` rotates and logs pending emissions but drains
  nothing): only tick-rate systems read the bus, and a
  drained-but-unread scheduled chain would die — the fixture alarm
  re-arms only when READ. Deferred entries fire, late but intact, at
  the first normal tick after the span.
- Facts emitted DURING the span do not pin (the spotlight reader is
  tick-rate and never runs), except the final stride's — still pending
  at resume, they pin like any fresh news: the newest news is news.
- Citizens BORN during the span have never met the (omitted)
  assignment; the catch-up schedule's Tier C variant integrates
  unassigned citizens too — "everyone runs Tier C" includes newborns.
- `catch_up(…, 0)` is a no-op (it must not even demote), and the span
  multiplication is checked arithmetic.

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
demographics bands; sized by MEASUREMENT, see §8). Validation
(**amended** to what is actually enforced and why): `tier_a_cap ≥ 1`
(someone anchors the embodied town) and `highlight_days ≥ 1`;
`tier_b_cap = 0` and `leisure_hours_per_day = 0` are LEGAL degenerate
configurations (the all-Tier-A twin and isolation tests depend on
them); leisure hours fit inside the day; tolerance ≤ 1000. Retail
`gain_per_unit` is capped at a full need (1,000,000 per-million) so
the coarse tiers' whole-unit purchase gates stay reachable.

## 8. Exit criteria mapping

- *10k citizens hit the 200 ticks/sec budget*: a release-only test
  (`#[cfg(not(debug_assertions))]`) builds a 10k-citizen town with the
  shipped caps, runs a fixed span, and asserts ≥ 200 ticks/sec;
  `check.sh` gains a release-mode run of the LOD suite so the gate
  enforces it (debug builds skip the timing assert, never the
  correctness asserts).
- *A↔C cycling conserves money exactly* (**amended** to the checks
  the suite actually proves, stated exactly): (1) the transitions
  themselves move NOTHING — a full-town demote+promote leaves every
  wallet and need level byte-identical (money is never copied, so
  exactness is structural, not reconciled); (2) a FORCED organic
  A→C→A cycle inside the live schedule (a spotlight pin displaces the
  back of the embodied front to C; its expiry brings them home) under
  the daily audit; (3) the cycled citizen's needs against their
  all-Tier-A twin self within the tolerance PLUS one satisfaction
  quantum (a unit's gain or one hour-block — instantaneous levels
  swing on the timing of the last satisfaction alone); (4) Tier C
  citizens hold real jobs. Per-citizen ledger-flow attribution is not
  reconciled — the audit's global and per-book identities are the
  conservation proof, and transitions provably cannot transfer.
- *Catch-up of one week < 5s*: release-only timing assert around
  `catch_up(…, 7 days)` at 10k citizens; plus (all profiles) the
  coarse integrator's determinism — same save caught up twice gives
  identical hashes — and conservation audits green after catch-up.
- *Macro time-series indistinguishable* (**amended** to the shipped
  definition): THREE seeds; twin towns per seed, all-A vs tiny caps
  (87% coarse — far beyond the shipped shape); four warm-up days
  excluded (genesis cold starts differ by construction); then twelve
  measured days where the MEAN DAILY relative difference of each
  series — employment rate, seekers, mean posted price, treasury
  receipts per day, total citizen money, units consumed per day
  (cumulative counters compare as per-day deltas) — must sit inside
  `macro_tolerance_per_mille`. The band is 200‰, sized by
  measurement: every flow series measures ≤ 150‰; the price LEVEL
  carries a persistent warm-up offset (~170‰ mean, parallel dynamics)
  at this extreme coarseness. At the shipped caps a small town is
  all-A — the band only ever binds this stress test.

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
- **Placement** (SPEC §4 lists a `sim_lod` crate): the tier systems
  live in `sim_ai` because tier execution IS agent behavior — Tier B/C
  reuse `sim_ai`-private machinery (`purchase_unit`, `CurrentAction`,
  `DailyPlan`), and SPEC §4 forbids `sim_ → sim_` calls, so a separate
  crate would force that private machinery into the shared interface.
  `sim_lod` stays an intentionally empty shell with an honest doc.

## Dependencies

No new external dependencies.
