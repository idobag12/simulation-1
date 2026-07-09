# ADR 0006 — Phase 3 design: utility AI core, locations, decision dumps

- Status: Accepted
- Date: 2026-07-09
- Phase: 3

SPEC §15 Phase 3 ("Action framework, need-based scoring with traits,
schedules, locations as abstract nodes, Tier A only at 500 citizens,
decision-dump debugging") and §11 underdetermine several mechanics; per
SPEC §16.7 the decisions are recorded before the code.

## 1. `sim_interface` is born; three components move, NAMEs unchanged

Phase 3 is the first time two sim crates need the same components:
`sim_ai` scores against `Needs`/`Personality` (until now `sim_people`'s)
and against locations (`sim_world`'s). Per SPEC §4, shared components move
into `core_ecs::sim_interface`:

| Component | NAME (unchanged) | Previously |
|---|---|---|
| `Needs` / `NeedLevel` | `people.needs` | `sim_people` |
| `Personality` | `people.personality` | `sim_people` |
| `Position` | `world.position` | new |
| `Location` | `world.location` | new |

`sim_people` re-exports the moved types (source compatibility); component
`NAME`s do not change, so the move is invisible to saves (ADR 0005 §1
promised exactly this). Components used by ONE sim crate stay put:
`Identity`/`Household`/`HouseholdMember` in `sim_people`;
`CurrentAction`/`DailyPlan`/`LastDecision` in `sim_ai`.

Cross-crate genesis stays orchestrated by the application (`headless`),
which may depend on every sim crate: `sim_people` populates citizens,
then `sim_world::genesis` creates locations and homes them — receiving
plain entity lists, never reaching into `sim_people` types.

## 2. Locations as abstract nodes

A location is an entity with `Location { kind: u32 }` (index into
`data/locations.ron`'s kind list, data order — the SPEC §8 stable-integer-
id pattern's first use). No map, no coordinates (Phase 9): travel between
ANY two locations costs a flat data-defined `travel_ticks`. Kinds declare
which needs they satisfy and at what per-tick, per-million rate; `is_home`
kinds are instantiated one per household at genesis (the household's
members' home), others `count`-many public instances. Capacity is
deliberately absent until the map exists (SPEC §16.3).

## 3. Action framework and the decision loop (Tier A, per tick)

Two systems, tick rate, in this order (SPEC §6 explicit order):

1. **`DecideSystem`** — for each citizen with no `CurrentAction` (entity
   order): enumerate candidates, score, argmax, write `CurrentAction` and
   `LastDecision`. Candidates: for every location and every need its kind
   satisfies → "go satisfy need N at location L"; plus `Idle` (baseline
   score 0, id-ordered last). A citizen already at L skips travel.
2. **`ActSystem`** — advances every `CurrentAction`: travel counts down,
   then performance applies the location's exact integer per-tick need
   gain; the action ends when the satisfied need reaches
   [`NeedLevel::MAX`] or a data-defined maximum duration elapses.
   Completion clears `CurrentAction` (next tick the citizen decides
   again).

## 4. Scoring: floats confined, effects exact (SPEC §2/§11)

`score = gain_estimate × urgency(level) × trait_factor − time_cost`, in
`f64`, exactly as SPEC §11 prescribes ("Scoring uses floats; the chosen
action and its effects are exact"):

- `urgency(level) = deficit^e` where `deficit = (MAX − level)/MAX ∈ [0,1]`
  and `e` is a data-defined small integer exponent, computed by repeated
  multiplication — the nonlinear urgency SPEC §11 requires (low hunger
  ignorable, critical hunger dominant).
- `trait_factor` blends data-defined per-need trait weights
  (`need_trait_weights`: which trait amplifies which need, per-mille).
- `time_cost` = data-defined cost per tick of travel+performance.
- Float discipline: only `+ − × ÷` and comparisons — IEEE-754-exact on
  every platform; no transcendental functions (libm varies across
  platforms and would break cross-platform determinism). Ties broken by
  candidate index (enumeration order is location-entity order × need
  order — deterministic). Floats are never stored: the chosen action's
  effects are integer, and dump scores are quantized (§6).

## 5. Schedules: a real bias, not a stub

Each citizen carries a `DailyPlan { sleep_start_hour, sleep_end_hour }`,
compiled by `PlanSystem` (hour rate, at the data-defined
`plan_compile_hour`) from the data base window shifted by the citizen's
industriousness trait (early risers exist). During a citizen's sleep
window, rest-satisfying candidates at the citizen's HOME get a
data-defined score bias. This is the SPEC §11 schedule in its smallest
honest form: obligations don't exist yet (no jobs until Phase 5), so the
compiled plan contains exactly the one block reality currently affords —
and it demonstrably shapes behavior (biases scoring), which Tier A
scoring can still override (a starving citizen skips sleep). Richer
compilation (work shifts, school) arrives with the phases that create
obligations.

## 6. Decision dumps: every decision inspectable (SPEC §11/§13/§15)

`LastDecision { tick, chosen: u32, candidates: Vec<ScoredCandidate> }`
with `ScoredCandidate { location, need_index, score_micro: i64 }` —
scores quantized to millionths for storage, because floats are never
persisted (SPEC §2). Quantization of a deterministic float is
deterministic, so hashes stay stable. The inspector renders the full
ranked candidate list with resolved location-kind and need names ("why
did Mara go to the tavern?" is answerable from a save). One decision per
citizen is retained (the last); decision *history* is what the event log
is for, and stays out until a consumer exists (SPEC §16.3).

## 7. Data files

- `data/locations.ron`: ordered kind list — id, `is_home`, `count`,
  satisfiers (need id → per-tick per-million rate).
- `data/balance/ai.ron`: `travel_ticks`, `urgency_exponent`,
  `time_cost_micro_per_tick`, `max_perform_ticks`, sleep window
  (base start/end hour, trait shift per-mille, bias), `plan_compile_hour`,
  `need_trait_weights` list.
- Validation (data_defs): every satisfier's need id exists in needs.ron;
  every weight's trait id exists in traits.ron; hours < 24; exponent ≥ 1;
  at least one home kind and one rest satisfier (a town whose citizens
  cannot sleep is a data error, not an emergent tragedy).

## 8. Save format v4

Registration grows by the new components (`world.position`,
`world.location`, `ai.current_action`, `ai.daily_plan`,
`ai.last_decision`); no new events. Per the ADR 0005 §9 permanent rule:
`FORMAT_VERSION` 4, mechanical v3→v4 migration (append empty stores),
chain `v1→…→v4`, new v4 golden fixture, goldens re-recorded with
content-continuity proofs for v3 (the same commit).

## 9. Exit-criteria test shape

- **Citizens visibly satisfy needs**: a 500-citizen AI town runs 5 days;
  mean satisfiable-need levels stay above a data-defined floor and every
  citizen's hunger oscillates (rises at least once) — the Phase 2 world
  (decay-to-zero) fails this by construction.
- **Individually distinct patterns**: over a week, per-citizen visit
  tallies by location kind are not all identical across citizens with
  differing traits (assert several distinct distributions exist).
- **Every decision inspectable**: after a run, every citizen that acted
  carries a `LastDecision` whose candidate list is non-empty and whose
  chosen index is valid; the inspector renders it.
- **Determinism holds**: the determinism/save-compat suites run the AI
  town (two fresh runs, mid-flight save/load across decisions and
  actions).

## Dependencies

No new external dependencies. (Tier A cap ~500 citizens is honored by
test scale; enforcing tiers is Phase 8 machinery.)
