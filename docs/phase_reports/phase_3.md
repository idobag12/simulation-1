# Phase 3 report — Utility AI core

- Date: 2026-07-09
- Scope: SPEC §15 Phase 3 (action framework, need-based scoring with
  traits, schedules, locations as abstract nodes, Tier A at small scale,
  decision-dump debugging)
- Design decisions: ADR 0006 (written before the code)
- Verdict: **complete** — all exit criteria demonstrated by CI-blocking
  tests; `scripts/check.sh` green.

## What was built

| Deliverable | Where |
|---|---|
| `core_ecs::sim_interface` (SPEC §4's shared-component home, first needed this phase): `Needs`/`NeedLevel`/`Personality` moved from `sim_people` with NAMEs unchanged (invisible to saves); new `Position`, `Location`, `Residence` | `crates/core_ecs/src/sim_interface.rs` |
| Locations as abstract nodes: data-defined kinds (`data/locations.ron`, order = stable integer id), one home per household + counted public instances, flat travel time — no map until Phase 9 | `crates/sim_world` |
| `DecideSystem` (tick): enumerate own-home + public candidates × satisfiers, score in `f64` (`gain × urgency(deficit^e) × trait_factor − time_cost + sleep_bias`), argmax with enumeration-order tie-break; float discipline: `+ − × ÷` only, never persisted (SPEC §2/§11) | `crates/sim_ai/src/systems.rs` |
| `ActSystem` (tick): travel countdown → arrival (`Position` update) → performance applying exact integer per-tick need gains, ending on need-full or duration; bounded idling | `crates/sim_ai/src/systems_act.rs` |
| `PlanSystem` (hour, at the data-defined compile hour): per-citizen sleep windows shifted by industriousness; the window biases rest-at-home scoring — a real schedule in its smallest honest form (ADR 0006 §5) | `crates/sim_ai/src/systems.rs` |
| Decision dumps: `LastDecision` with the full scored candidate list (scores quantized to micro units — floats never persisted); inspector renders position, current action, and every ranked candidate with resolved names | `crates/sim_ai/src/components.rs`, `crates/headless/src/inspect.rs` |
| Data + validation: `locations.ron`, `balance/ai.ron`; cross-checks (need/trait ids exist, exactly one home kind, rest must be satisfiable somewhere, hours/exponent/weights in range); `data_defs::resolve_ai` builds the index-based tables systems run on, so `sim_ai` never sees other sim crates' config types | `crates/data_defs` |
| Save format v4: six appended registrations, pure `v3→v4` migration, chain `v1→…→v4` with fixture-level content-continuity proofs for v2 AND v3; goldens re-recorded per policy; new v4 golden fixture saved with actions in flight | `crates/persistence`, `tests/tests/save_compat.rs` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 3) | Test |
|---|---|
| Citizens visibly satisfy needs | `ai::citizens_visibly_satisfy_needs` — 500 citizens (Tier A small scale), 5 days: mean hunger/rest/social stay above 25% (a Phase 2 world is pinned at 0 by day two) and hunger RISES for >100 citizens between observations (impossible without satisfaction) |
| …with individually distinct patterns | `ai::citizens_show_individually_distinct_patterns` — hourly location-kind profiles over 3 days: >¼ of citizens have unique profiles; plus `ai::citizens_sleep_at_home_at_night` (≥⅔ home at 03:00 — the plan bias demonstrably shapes behavior) |
| Every decision inspectable | `ai::every_decision_is_inspectable` — every living citizen carries a coherent `LastDecision` (valid chosen index, non-empty candidates, idle-consistency), and the inspector renders the ranked list with resolved names from a save |
| Determinism holds | `ai::ai_town_replays_identically_across_save_load` (mid-flight save with actions in progress), the determinism suite (unchanged, still green), and the v4 golden fixture load/resume |

Also this phase:

- The 10k-citizen demographic regression now runs the passive schedule
  (decay + mortality, no AI) with the reasoning documented in the test:
  full per-tick Tier A at 10k is precisely what Phase 8's LOD tiers exist
  to make affordable (SPEC §10; §15 Phase 3 pins Tier A at ~500). Mortality
  — the regression's subject — is identical either way.
- `certain_mortality` now asserts that only location entities survive a
  full die-off (places persist; people don't).
- The exact-decay arithmetic test runs a decay-only schedule so AI
  satisfaction gains cannot mask it; the AI/decay interplay is `ai.rs`'s
  job.

## Verification process

`scripts/check.sh` green at the tagged commit (fmt, clippy `-D warnings`,
full workspace suite — 100+ tests including the 1M-tick and 10k-citizen
runs). This phase's review was a **solo pass** (the multi-agent
adversarial review used in Phases 0–2 runs on request): re-reading the
diff against SPEC §§2–4, 6, 8–9, 11 and ADR 0006 with particular attention
to — registration order matching the frozen migration constants
(byte-for-byte, checked); float confinement (no transcendental functions,
no persisted floats — dump scores quantized); two-pass borrow patterns in
all three systems (no mid-iteration mutation); `--load` deriving the AI
schedule from save content (the Phase 2 fix holds for AI towns, proven by
the v4 resume test using `derive_spec_from_world`); and the SPEC §3
module-size rule (two modules split when they crossed 500 lines).

## Deviations from spec (all pre-declared in ADRs)

- Scheduler-state placement, delivery semantics etc. carried over from
  ADRs 0002/0004/0005 unchanged.
- The compiled schedule contains only the sleep block — obligations don't
  exist until Phase 5 jobs; the plan biases rather than dictates
  (ADR 0006 §5, matching SPEC §11's override semantics).
- Location capacity deliberately absent until the map exists (ADR 0006 §2).
- Migrated pre-v4 towns have citizens but no locations: the AI idles in
  them honestly — migrations never invent state (documented at the
  migration and asserted by the re-recorded v3 resume golden).

## Known limitations (clean seams, not fakes)

- Planning layer (goals/intentions), memory, and relationships are Phase 7
  (SPEC §11 lists them; §15 assigns them).
- One decision retained per citizen (the last); decision *history* belongs
  to the event log when a consumer exists.
- Tier A only — tier assignment/promotion is Phase 8; test scale honors
  the ~500 target.
- Idle-heavy towns re-decide every `idle_ticks`; fine at Phase 3 scale,
  Phase 9's profiling owns anything beyond it.

## Performance note (no budget applies until Phase 9)

500-citizen AI town: 5 simulated days ≈ 0.4 s release, ≈ 6 s debug. Full
workspace suite ≈ 60 s debug. The Phase 8 200-ticks/sec-at-10k budget will
need the LOD tiers, as designed.
