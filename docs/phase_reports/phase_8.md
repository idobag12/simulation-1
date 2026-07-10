# Phase 8 report — LOD tiers & catch-up

- Date: 2026-07-10
- Scope: SPEC §15 Phase 8 (DayModels, Tier B/C execution,
  promotion/demotion, catch-up controller) against §10's tier contract
  and §5's catch-up contract
- Design decisions: ADR 0011 (written before the code)
- Verdict: **complete** — all four exit criteria demonstrated by
  CI-blocking tests; `scripts/check.sh` green (now including a
  release-mode run of the LOD suite for the timing budgets).

## What was built

| Deliverable | Where |
|---|---|
| Tier membership: `LodTier` (A/B/C, missing row = A — migrations never invent state), assigned deterministically each day boundary in entity order — spotlighted citizens first, then the stable front, capped by `balance/lod.ron`; a tick-rate reader pins citizens touched by high-signal facts (fired, defaulted, married, born) into the embodied tier via the persisted `Spotlight` component | `crates/core_ecs/src/sim_interface_lod.rs`, `crates/sim_ai/src/systems_lod.rs::{SpotlightSystem, TierAssignSystem}` |
| Money is NEVER modeled: wallets, deposits, employment, tenancies, ownership stay live components at every tier and every economy day-system (payroll, rent, labor clearing, markets, bank, taxes) is tier-agnostic — which is what makes A↔C cycling conserve money EXACTLY (there is nothing to copy back) | ADR 0011 §2; the diff deliberately touches no `sim_economy` money path |
| Tier B (per-hour): the day executes as blocks from the SAME obligations the embodied biases read — sleep window at home, shift at the workplace, school in its hours (the data gain once per day), else leisure at the venue best satisfying the most deficient need; needs integrate analytically (rate × 60, exact integers), one REAL purchase may resolve per hour through the shared till, and Position genuinely moves so the Phase 7 social hour keeps drifting bonds over Tier B citizens | `systems_lod.rs::TierBSystem` |
| Tier C (per-day): each citizen's `DayModel` (per-need passive daily gain derived from the data satisfier tables and their job; NO money, NO shadow needs — the live row is the single source of truth) executes once per day: real purchases for the deficits retail can satisfy (cheapest KNOWN shop — believed price, posted fallback — unit by unit through the shared till, demand tagged with their identity), passive gains for the rest, the school day for school-age citizens | `sim_interface_lod.rs::DayModel`, `systems_lod.rs::TierCSystem` |
| Promotion/demotion, lossless: demotion removes the embodied rows, derives the model, parks the citizen at home; promotion drops the model and lets the next tick's DecideSystem take over from the LIVE needs row — the same code path a fresh citizen takes; `TierChanged` facts for observability | `systems_lod.rs::{demote, promote}` |
| The catch-up controller: `run_ticks_coarse` advances in hour strides (boundary-rate systems run exactly where the normal loop runs them; tick systems are skipped), `runner::catch_up` demotes everyone to Tier C through the normal path and integrates the span — SPEC §5's "everyone runs Tier C", a DEFINED deterministic coarse integrator, never a skip; surfaced as `embervale run --catch-up-days N` | `crates/core_ecs/src/system.rs::run_tick_coarse`, `crates/sim_time`, `crates/headless/src/{runner,cli}.rs` |
| Tier gating of the embodied systems: DecideSystem skips B/C, PlanSystem skips C (Tier B executes the plan's sleep window), RelationshipDecaySystem skips C (drift pauses with presence — absence from the EMBODIED world must not dissolve bonds the model cannot rebuild) | `crates/sim_ai/src/{systems,systems_plan,systems_social}.rs` |
| Save format v9: `lod.tier`, `lod.day_model`, `lod.spotlight` + `lod.tier_changed` appended; mechanical v8→v9 chained from v1 with a v8-fixture continuity proof; a 2,500-citizen v9 fixture with ALL THREE tiers live (asserted), day models, and green audits; goldens re-recorded per policy; pinned snapshot moved to `data_v9` (gains `balance/lod.ron`) | `crates/persistence`, `tests/tests/save_compat.rs` |
| Data + validation: `balance/lod.ron` (caps, pin length, leisure block, the macro acceptance band — the exit criterion's "defined tolerance" is data, like the Phase 2 demographic bands); caps/windows/tolerance validated at load | `data/balance/lod.ron`, `crates/data_defs` |
| Observability: citizen dumps show the tier; tier changes are events | `crates/headless/src/inspect.rs` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 8) | Test |
|---|---|
| 10k citizens hit the 200 ticks/sec budget | `lod::ten_thousand_citizens_hit_the_tick_budget` (release builds; `check.sh` runs the LOD suite in release) — a 10k town under the shipped caps, first day settled off the clock, then a timed span must clear 200 ticks/sec |
| A↔C cycling conserves money exactly | `lod::a_to_c_to_a_cycling_conserves_money_exactly` — the isolated full-town demote+promote cycle leaves every wallet and every need level BYTE-IDENTICAL (the transitions move nothing), and a tiered town cycles citizens for a week under the scheduled daily audit (finishing is the conservation proof) with Tier C citizens holding real jobs |
| Catch-up of one week < 5s | `lod::a_week_of_catchup_fits_the_five_second_budget` (release) at 10k citizens; plus (all profiles) `lod::a_week_of_catchup_is_deterministic_and_conserves` — the same save caught up twice lands on identical hashes, audits stay green, and the normal per-tick loop resumes cleanly |
| Macro time-series statistically indistinguishable (defined tolerance) | `lod::tiered_macro_series_match_the_all_tier_a_twin` — same seed, same data, only the caps differ (all-A vs 20/40 mixed); per-day series of employment, seekers, mean posted price, treasury receipts, and total citizen money agree on run-average within `macro_tolerance_per_mille` from the data |

Also this phase:

- Tier assignment lands on tick 0 (a day boundary), so a 10k genesis
  never runs 10k embodied citizens.
- The whole Phase 2–7 suite still passes over mixed tiers — the
  250-citizen exit towns now carry a Tier B tail, and the town still
  feeds itself, clears its markets, banks, marries, and reproduces.

## Verification process

`scripts/check.sh` green at the tagged commit (fmt, clippy `-D
warnings`, full workspace suite, and the release-mode LOD suite).
Adversarial multi-agent review (ultracode): five reviewers over the
phase diff, findings verified against the code before acting.

<!-- REVIEW FINDINGS -->

## Deviations from SPEC

- Focus is the stable front of the town (lowest entity indices) until
  the viewer exists — the deterministic, capped, event-pinned CONTRACT
  is what §10 specifies; a camera can replace the ordering rule later
  without touching the mechanism (ADR 0011 §1).
- Tier B "aggregate draws from the agent's own distributions" ships as
  deterministic analytic block integration (no per-agent distribution
  objects yet) — the smallest honest form the exit criteria exercise.
- Tier C relationship drift pauses (and decay pauses with it) rather
  than modeling social life statistically; the exit criteria measure
  economic macro series (ADR 0011 §9).

## Tag

`phase-8` (local; tag pushes are blocked by the environment's proxy —
the branch carries the content).
