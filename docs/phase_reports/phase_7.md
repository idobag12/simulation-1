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
| The relationship graph: sparse capped edges (Kin/Spouse/Friend/Romance) with shared sorted-upsert helpers (the cap bounds DRIFT edges; kin and spouses always record — privileged); the social hour drifts bonds by co-presence at leisure venues (everyone meets their neighbor; singles ADDITIONALLY meet the next single — partner-seekers find each other, each pair once per hour); no romance between kin (checked both sides); daily decay dissolves what absence leaves; death removes every survivor's spouse edge (widows may love again) while kin edges remain | `sim_interface_social.rs`, `crates/sim_ai/src/systems_social.rs`, `crates/sim_people/src/systems.rs` |
| Bonds feed utility: satisfying the social need among present friends genuinely gains more (per-friend weight × strength multiplier on the exact integer gain) | `crates/sim_ai/src/systems_act.rs` |
| Beliefs + gossip: bounded believed-price rows written by every purchase (experience overwrites and ALWAYS lands — at the cap the priciest belief is forgotten first; hearsay drops at the cap) and exchanged at integer midpoints when citizens meet; purchase scoring weighs the BELIEVED price, the till corrects it — being wrong is honest and self-healing | `sim_interface_social.rs`, `crates/sim_ai/src/{systems,systems_act,systems_social}.rs` |
| Marriage: a Romance edge crossing the data threshold between single non-kin adults → spouse edges, the `Married` fact, and a NEW household of exactly the couple (the birth families keep their registers); the wedding clears every Romance edge touching either newlywed (fidelity resets courtship); a partner's home houses the couple, homeless nest-leavers start under the in-laws' roof; the vacated tenancy releases to the housing market | `crates/sim_people/src/systems_social.rs::MarriageSystem` |
| Reproduction: age-banded daily chances on the `people.fertility` stream, one draw per couple (the father must be male), gated on marriage + a shared roof + the data cap counting those UNDER the roof (re-checked live before each birth); a birth spawns a REAL child — heritable traits (parents' midpoint ± data mutation, exactly the midpoint at zero mutation), zeroed skills, the mother's home and household, kinship edges both ways (parents, siblings), the `Born` fact; children age, school, come of age, work, wed, and parent — the pipeline closes | `systems_social.rs::FertilitySystem`, `data/balance/fertility.ron` |
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
- The wedding clears every Romance edge touching either newlywed —
  fidelity resets courtship (review-driven).
- The fertility cap counts those under the mother's roof, not the
  register (review-driven); experience always lands in a full belief
  list (review-driven).
- The skill premium prices through reservations, not bids (a uniform
  double-auction bid cannot know its match; the ask knows its owner);
  the WAGE sense of mobility is proven at the unit level — run wages
  confound wage-at-hire with skill-now (§8 amended).
- Composer patterns are code, not data (presentation, not tunables).

## Verification process

`scripts/check.sh` green at the tagged commit (fmt, clippy `-D
warnings`, full workspace suite including the multi-generation, money,
labor, and save-compat suites). Adversarial multi-agent review
(ultracode): five reviewers over the phase diff, findings verified
against the code before acting.

Bugs the phase's own instrumentation caught before review (each forced
a documented ADR amendment; all regression-guarded by the exit suite):

- **Marriage merges sterilized the town**: the designed household merge
  compounded into mega-registers that busted the fertility cap within a
  generation — a wedding now founds a NEW household.
- **The last singles never met**: adjacent-neighbor meetings skip
  singles separated by married neighbors — the singles pass fixed it.
- **Homeless newlyweds never parented**: two nest-leavers had no shared
  roof; the in-laws' fallback houses them.

Confirmed review findings (five reviewers: conservation/atomicity,
determinism/ordering, save-format v8, SPEC+ADR conformance, edge
cases/failure modes), all fixed in the review commit:

- **The school kind was inserted mid-list** (critical): `Location.kind`
  is a persisted index, so every pre-v8 save's town hall silently became
  the school and the builder yard the town hall. Fixed: the kind is
  APPENDED at the end of `locations.ron`; the v7 suite now asserts the
  migrated town hall and builder yard keep their kinds and that no
  school appears from thin air (migrated towns are schoolless by
  documented decision — counts do not retro-spawn).
- **`Relationships::upsert` silently dropped Kin/Spouse at the cap**
  (major): a full heart forgot its newborn or spouse — one-sided
  marriages (bigamy past `has_spouse`), a defeated incest screen, and
  widows locked to corpses. Fixed: Kin/Spouse are privileged (they evict
  the weakest drift edge regardless of strength, or exceed the cap when
  only family remains), and `settle_death` now clears spouse edges by
  scanning the SURVIVORS' lists, not the deceased's.
- **The singles pass double-drifted most courting pairs** (major): its
  dedup guard tested raw entity-index adjacency, not adjacency in the
  present list, silently doubling the data-tuned courtship pacing.
  Fixed: dedup against the pairs the general pass actually met.
- **Same-sex couples double-drew fertility and could breach the cap**
  (major): the mother filter never checked the spouse's sex (two
  independent draws per F/F couple, the second birth overshooting
  `max_household_size` the same day). Fixed: the father must be male,
  and the cap re-checks LIVE state before each birth.
- **No unit tests for the social systems** (major): the 60-year
  statistical run cannot see doubled drift or one-sided edges. Fixed:
  sixteen in-crate unit tests — upsert cap/privilege semantics, belief
  experience-vs-gossip (cap bounds included), drift dedup and the kin
  bar, the friend-presence utility bonus, marriage household/fidelity/
  kin cases, fertility gates (father, roof cap, same-day double birth),
  trait-inheritance bounds, and widow remarriage through `settle_death`.
- **Kinship screens were one-sided; marriage had none** (minor): fixed —
  the drift screen and a new MarriageSystem bar both check BOTH lists.
- **The composer misattributed remarriage children** (minor): a Born
  matched the OLD marriage's line by one parent; the full couple must
  match now, unit-proven alongside the labor and money chains.
- **A dead tenant's home sheltered squatters** (minor): the Tenancy now
  passes to a co-resident, else every co-resident's Residence clears —
  and eviction (both branches) clears the WHOLE household.
- **Stale third-party Romance edges survived the wedding** (minor): a
  widow could remarry an old flame the day after the funeral with no new
  courtship. Fixed: the wedding clears every Romance edge touching
  either newlywed (ADR-documented: fidelity resets courtship).
- **The fertility cap counted nest-left adult children** (minor): grown
  children on the register sterilized their parents. Fixed: the cap
  counts household members under the mother's ROOF.
- **`trait_mutation_per_mille: 0` biased children upward** (minor): a
  spurious `.max(1)` made zero mutation drift every trait by +1/mille
  expected. Fixed and unit-proven: zero mutation is exactly the
  midpoint.
- **`max_household_size: 2` legally sterilized the town** (minor): the
  cap counts the couple. Validation floor raised to 3 with a message
  saying why.
- **Belief rows froze at the cap even for direct experience** (minor):
  a purchase at an unknown shop recorded nothing. Fixed: experience
  always lands (the most-expensive believed shop is forgotten first);
  hearsay dropping at the cap stays, documented.
- **Payroll's lazy Skills row was sized `skill + 1`** (minor): fixed to
  the full data `skill_count`, like every other creation site.
- Module-size violations (two files over the SPEC §3 limit → split into
  `systems_score.rs` and `payroll.rs`), stale MarriageSystem docs from
  the rescinded merge design, missing v8 history annotations, and a
  reason-less `#[allow]` — all fixed.

Goldens: the v4–v7 RESUME goldens moved with these fixes and were
re-recorded with reasons (migrated citizens GROW a social graph from
lived meetings, so the drift-dedup/fidelity/roof-cap fixes shift those
trajectories; every at-load hash is unchanged — the stored bytes and
registration are untouched); the v8 fixture was regenerated under the
fixed systems and asserts its own social coverage.

## Deviations from SPEC

Episodic memory rings and professional/rivalry edges are deferred with
rationale in ADR 0010 (nothing in the Phase 7 exit criteria touches
them); hard skill gates likewise. All other deviations are the ADR
amendments above.

## Tag

`phase-7` (local; tag pushes are blocked by the environment's proxy —
the branch carries the content).
