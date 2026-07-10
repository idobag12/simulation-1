# Phase 7 report — The social layer

- Date: 2026-07-10
- Scope: SPEC §15 Phase 7 (relationships graph, memory/beliefs, gossip,
  courtship/marriage/reproduction, education pipeline affecting skills)
- Design decisions: ADR 0010 (written before the code; §§1–2, 4–5
  amended in-phase, each deviation documented before the deviating code)
- Verdict: **complete** — both exit criteria demonstrated by CI-blocking
  tests; `scripts/check.sh` green.

## What was built

| Deliverable | Where |
|---|---|
| Skills + the education pipeline: `Skills` per-mille mastery vector; a real `school` location; `AttendSchool`/`SchoolTravel` action variants (appended) biased during data school hours — obligations bias, never dictate; completion raises the taught skill and emits `SchoolAttended`; a paid workday teaches the employer's recipe skill (learning by doing); the skill wage premium enters through RESERVATIONS (mastery is an outside option — the ask side knows its owner) | `crates/core_ecs/src/sim_interface_social.rs`, `crates/sim_ai/src/{components,systems,systems_act}.rs`, `crates/sim_economy/src/labor.rs`, `data/skills.ron` |
| The relationship graph: sparse capped edges (Kin/Spouse/Friend/Romance) with shared sorted-upsert helpers (kin never evicted); the social hour drifts bonds by co-presence at leisure venues (everyone meets their neighbor; singles ADDITIONALLY meet the next single — partner-seekers find each other); no romance between kin; daily decay dissolves what absence leaves; death removes the survivor's spouse edge (widows may love again) while kin edges remain | `sim_interface_social.rs`, `crates/sim_ai/src/systems_social.rs`, `crates/sim_people/src/systems.rs` |
| Bonds feed utility: satisfying the social need among present friends genuinely gains more (per-friend weight × strength multiplier on the exact integer gain) | `crates/sim_ai/src/systems_act.rs` |
| Beliefs + gossip: bounded believed-price rows written by every purchase (experience overwrites — you saw the tag) and exchanged at integer midpoints when citizens meet; purchase scoring weighs the BELIEVED price, the till corrects it — being wrong is honest and self-healing | `sim_interface_social.rs`, `crates/sim_ai/src/{systems,systems_act,systems_social}.rs` |
| Marriage: a Romance edge crossing the data threshold between single adults → spouse edges, the `Married` fact, and a NEW household of exactly the couple (the birth families keep their registers); a partner's home houses the couple, homeless nest-leavers start under the in-laws' roof; the vacated tenancy releases to the housing market | `crates/sim_people/src/systems_social.rs::MarriageSystem` |
| Reproduction: age-banded daily chances on the `people.fertility` stream, gated on marriage + a shared roof + the data household cap; a birth spawns a REAL child — heritable traits (parents' midpoint ± data mutation), zeroed skills, the mother's home and household, kinship edges both ways (parents, siblings), the `Born` fact; children age, school, come of age, work, wed, and parent — the pipeline closes | `systems_social.rs::FertilitySystem`, `data/balance/fertility.ron` |
| The birthday pass maintains `SchoolAge` beside `WorkingAge`; new adults leave the nest (the rental market's demand margin, ADR 0009 §3 delivered) | `crates/sim_people/src/systems.rs` |
| The narrative composer: a pure observer over the event-log ring composing REAL chains — marriage → birth, job loss → new work, default → renting again — into human-readable lines with true names and days; `embervale stories --load PATH [--entity N]` | `crates/debug_tools/src/narrative.rs`, `crates/headless/src/cli.rs` |
| Save format v8: four appended components + three events, pure `v7→v8` chained from v1, a v7-fixture continuity proof, the pinned snapshot moved to `data_v8`, a socially rich v8 fixture (learners, thousands of edges, concluded marriages — all asserted), goldens re-recorded per policy | `crates/persistence`, `tests/tests/save_compat.rs` |
| Data + validation: `skills.ron`, `balance/{social,fertility}.ron`, the `school` location kind, per-recipe skills; every id resolves, school ages below working age, hours ordered, gains/drifts/caps bounded, fertility bands ascending and non-overlapping, mutation ≤ 1000 | `data/`, `crates/data_defs/src/validate_money.rs::validate_social` |
| Observability: citizen dumps show skills, every bond (kind, name, strength), and believed prices; school actions render in the activity report | `crates/headless/src/inspect.rs` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 7) | Test |
|---|---|
| Multi-generation run produces kinship networks and skill mobility | `social::a_multi_generation_run_produces_kinship_networks_and_skill_mobility` — a data-shaped fast-generation town (8-day year, wide fertility band, gentle mortality — all legal data, the flat-mortality pattern applied to time) runs 60 years under daily audits: ≥10 native-born citizens, ≥3 SECOND-generation natives (native children of native parents — three linked kinship generations), ≥10 spouse edges, ≥100 kin edges, and ≥3 school-taught natives whose letters exceed every elder kin's — mobility in the skill sense, with the wage sense unit-proven (a master's reservation clears above a novice's) |
| Narrative composer surfaces coherent story lines from real event chains | The same run's event ring composes real stories (a marriage or birth line is asserted present); `social::the_composer_reads_chains_from_the_event_log` proves the chain patterns deterministically — hand-emitted Married + Born compose into "Alice Stone and Bob Stone married on day 3; their child Carol Stone was born on day 10", and subject filtering keeps only that citizen's lines |

Also this phase:

- The v8 fixture asserts its own coverage (learned skills, a live graph,
  concluded marriages, conservation) and resumes deterministically
  through the full Phase 7 day schedule.
- The whole Phase 3–6 exit suites still pass over the social layer —
  the town feeds itself, prices respond, the labor market clears, the
  monetary loop closes, and now the town also reproduces itself.

## In-phase design amendments (ADR 0010, written before the code moved)

- Marriage founds a NEW household: the designed merge COMPOUNDED — every
  wedding unioned two extended families into one ever-growing register
  that busted the fertility cap and sterilized the town within a
  generation.
- The singles pass in the social hour: without it the last singles sat
  between married neighbors in the meeting order and never paired — the
  generations stopped.
- No romance between kin; widowhood clears the survivor's spouse edge.
- The skill premium prices through reservations, not bids (a uniform
  double-auction bid cannot know its match; the ask knows its owner).
- Composer patterns are code, not data (presentation, not tunables).

## Verification process

`scripts/check.sh` green at the tagged commit (fmt, clippy `-D
warnings`, full workspace suite including the multi-generation, money,
labor, and save-compat suites). Adversarial multi-agent review
(ultracode): five reviewers over the phase diff, findings verified
against the code before acting.

<!-- REVIEW FINDINGS -->

## Deviations from SPEC

Episodic memory rings and professional/rivalry edges are deferred with
rationale in ADR 0010 (nothing in the Phase 7 exit criteria touches
them); hard skill gates likewise. All other deviations are the ADR
amendments above.

## Tag

`phase-7` (local; tag pushes are blocked by the environment's proxy —
the branch carries the content).
