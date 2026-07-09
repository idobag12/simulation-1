# ADR 0005 — Phase 2 design: citizens, needs, lifecycle, households, names

- Status: Accepted
- Date: 2026-07-09
- Phase: 2

SPEC §15 Phase 2 ("People exist: identity, traits, needs with decay,
lifecycle, households, name generation — no AI yet") underdetermines the
representations. Per SPEC §16.7, decisions are recorded before the code.

## 1. All Phase 2 components live in `sim_people`

`core_ecs::sim_interface` exists for components/events shared between sim
crates (SPEC §4). Phase 2 has exactly one sim content crate, so nothing is
shared yet; hoisting components into the interface before a second crate
needs them would be speculative (SPEC §16.3). When Phase 4's economy needs
citizen wealth, the components it shares move to `sim_interface` in that
phase, with a save-compatible rename-free move (component `NAME`s do not
change when a type moves crates).

## 2. Fixed-point representations (SPEC §2: no floats in persisted state)

- **Needs**: each need level is an integer in **per-million units**
  (`NeedLevel(i64)`, 0 = fully depleted, 1_000_000 = fully satisfied),
  clamped at both ends. Decay rates come from data as per-million units
  **per hour** (needs decay is an hour-rate system, SPEC §5).
- **Traits**: per-mille weights (`i16`, nominal range 0..=1000) sampled at
  creation from data-defined uniform ranges (distributions get richer when
  heredity arrives in Phase 7).
- **Ages** are not stored: a citizen's age derives from `birth_tick` and
  the current tick (single source of truth; no aging system can drift).
  `birth_tick` may be negative-equivalent for citizens older than the
  world; it is stored as a signed tick offset (`i64` ticks relative to
  tick 0) for exactly this reason.

## 3. Phase 2 component set (dense unless noted)

| Component | NAME | Contents |
|---|---|---|
| `Identity` | `people.identity` | given/family name, sex, `birth_tick: i64` |
| `Needs` | `people.needs` | per-need `NeedLevel` vector, fixed order = data order |
| `Personality` | `people.personality` | per-trait per-mille weights, fixed order = data order |
| `HouseholdMember` | `people.household_member` | household entity handle |
| `Household` | `people.household` (sparse) | member entity list (kept in entity-index order) |

Households are entities carrying `Household`; citizens point at them via
`HouseholdMember`. Membership lists are Vec<Entity> sorted by entity index
(deterministic iteration, SPEC §3).

## 4. Systems and rates (explicit order, SPEC §6)

- `NeedsDecaySystem` (Hour): integrates each citizen's needs down by the
  data-defined per-hour decay, clamping at 0. No satisfaction sources yet —
  that is Phase 3 AI/Phase 4 goods scope; Phase 2 citizens simply get
  hungrier and more tired, which is exactly what "no AI yet" means. (Needs
  at 0 do not kill in Phase 2; need-linked health is a later-phase
  mechanism. Mortality is age-driven below.)
- `MortalitySystem` (Day): for each citizen in entity-index order, look up
  the per-day death probability for their age band (data-defined curve)
  and draw from the `"people.mortality"` stream; on death, emit
  `PersonDied { person, cause: OldAge }` (the SPEC §7 exemplar fact),
  remove the citizen from their household (dissolving empty households),
  and despawn via the command buffer.
- Probabilities are fixed-point: per-day death chance in **per-billion**
  units (`u32`), compared against `draw % 1_000_000_000` — exact integer
  comparison, no floats.

## 5. Deterministic population generation

`populate` (in `sim_people::genesis`) builds the initial town from
`data/balance/demographics.ron` + `data/names/*.ron` using dedicated
streams (`people.genesis.*`): ages sampled from data-defined weighted age
bands, sex ~ data ratio, names drawn from data name lists, traits from
data ranges, citizens grouped into households of data-defined size ranges.
Iteration order is construction order; every draw comes from named
streams; same seed + same data ⇒ bit-identical town.

## 6. Data files (SPEC §8: all tunables in data)

- `data/balance/needs.ron`: ordered need definitions (id name, decay per
  hour, initial range).
- `data/balance/traits.ron`: ordered trait definitions (id name, min/max
  per-mille).
- `data/balance/mortality.ron`: ascending age bands (max age in years →
  per-day death chance per-billion); validator requires full coverage to a
  terminal band and monotone band edges.
- `data/balance/demographics.ron`: age-band weights for genesis, sex
  ratio (per-mille), household size range, plus the **demographic
  regression bands** the exit criterion asserts (expected annual crude
  death rate range) — the acceptance thresholds themselves are data, not
  code constants.
- `data/names/given_female.ron`, `given_male.ron`, `family.ron`: name
  lists (non-empty, validated).
- The year length in ticks depends on `calendar.ron`'s `days_per_season`;
  age-in-years derives from the calendar. Validation cross-checks that
  mortality/demographics band ages fit in u32 years.

## 7. Inspector v1 (headless queries, SPEC §13/§15)

New CLI subcommands (tooling, per SPEC §3):
- `inspect --load SAVE [--data DIR] --entity INDEX`: full dump of one
  citizen — identity, age (years/days), needs, personality, household and
  co-members — or of a household entity.
- `demography --load SAVE [--data DIR]`: population count, age histogram
  by decade, sex counts, household size histogram — the numbers the
  demographic bands are asserted against.
- `run` gains `--stats-csv PATH`: appends one row per simulated day
  (tick, population, deaths so far) — the SPEC §13 time-series seam,
  fed by counting `PersonDied` events in the runner (observer, not a
  simulation system).

## 8. Exit-criterion test shape

`tests/tests/demographics.rs` (release-profile-friendly): golden-seed run
of 10,000 citizens for one simulated year (SPEC §15 Phase 2); asserts the
crude death rate falls inside the data-defined band, the population never
increases (no births until Phase 7 — a decreasing-only invariant this
phase), determinism of the run (hash-compare against a second run), and
conservation trivially holds (no money/goods exist; the audit asserts the
absence: no component store carries `Money` yet). Runtime bound: the suite
must stay under a few minutes in debug; if it cannot, the 10k×1yr test
moves behind `--release` in `scripts/check.sh` with the reason documented
in the phase report.

## 9. Event addition

`PersonDied { person, cause }` is the first real simulation event,
defined in `sim_people` (single-crate consumer for now; moves to
`sim_interface` when another sim crate subscribes). Save-format impact:
event registration order for the application grows — fixture events stay
first, then `people.person_died`; this is an event-registration change,
which `Events::restore` treats strictly. The committed v1/v2 fixtures
predate event registration mismatches… **no**: registration names are
part of saved event state (ADR 0004 §4), so adding a registration AFTER
the v2 fixture was recorded would make the fixture's saved name list
(fixture events only) mismatch the live registration. Resolution: this is
exactly the same class as adding a component, and the strictness is the
point. The fixture apps' registration is versioned: `register_world`
becomes the single place whose growth requires a save-format bump.
Phase 2 therefore bumps `FORMAT_VERSION` to 3 with a migration that
extends a v2 save's event-name list with newly registered names (pure,
mechanical: appended registrations cannot be referenced by any stored
event, so extending the list is lossless), and the suite gains a v3
golden fixture. This rule — "appending registrations = format bump with
list-extension migration" — is the permanent pattern for all later
phases.

## Dependencies

No new external dependencies.
