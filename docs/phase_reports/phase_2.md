# Phase 2 report — People exist

- Date: 2026-07-09
- Scope: SPEC §15 Phase 2 (citizen entities: identity, traits, needs with
  decay, lifecycle, households, name generation; no AI yet; inspector v1)
- Design decisions: ADR 0005 (written before the code)
- Verdict: **complete** — all exit criteria demonstrated by CI-blocking
  tests; `scripts/check.sh` green.

## What was built

| Deliverable | Where |
|---|---|
| Citizen components: `Identity` (age always derived from `birth_tick`), `Needs` (per-million fixed-point, data-ordered vector), `Personality` (per-mille traits), `HouseholdMember`/`Household` (member lists in entity-index order) | `crates/sim_people/src/components.rs` |
| `NeedsDecaySystem` (hour rate): data-defined per-hour decay, clamped at 0 — no satisfaction sources by design ("no AI yet") | `crates/sim_people/src/systems.rs` |
| `MortalitySystem` (day rate): data-defined per-billion daily death chance by age band, one draw per citizen per day from `people.mortality`; emits `PersonDied` (the SPEC §7 exemplar fact); removes the deceased from their household and dissolves emptied households through the command buffer | `crates/sim_people/src/systems.rs` |
| Deterministic genesis: weighted age pyramid, sex ratio, authored name lists, household grouping — every draw from `people.genesis`, fixed order | `crates/sim_people/src/genesis.rs` |
| Config schemas live with their consumer crate; `data_defs` loads and validates them (weights sum to 1000, strictly ascending mortality bands, per-mille ranges, non-empty name lists, precise per-file messages) | `crates/sim_people/src/config.rs`, `crates/data_defs` |
| Data: `needs.ron`, `traits.ron`, `mortality.ron`, `demographics.ron` (including the acceptance bands as data), `names/*.ron` | `data/` |
| Save format v3: appending registrations is a format bump (ADR 0005 §9); pure `v2→v3` migration appends empty people stores and losslessly extends the saved event-name list (`core_events::extend_registration_bytes`); v1 chain-migrates `v1→v2→v3` | `crates/persistence/src/migrations.rs` |
| Inspector v1: `inspect --entity` (citizen/household dump), `demography` (population, age decades, sexes, household sizes), `run --stats-csv` (daily population/deaths time-series), `--citizens N` world assembly | `crates/headless/src/inspect.rs`, `cli.rs` |
| `CommandBuffer::run`: deferred arbitrary mutations (needed for cross-entity household updates on death) | `crates/core_ecs/src/command.rs` |
| Frozen data snapshot for fixture suites, so live balance edits cannot shift save-fixture goldens | `tests/fixtures/data_v3/` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 2) | Test |
|---|---|
| 10k citizens simulate a year | `demographics::ten_thousand_citizens_simulate_a_year_within_demographic_bands` (172,800 ticks, ~12 s debug) and `…_is_deterministic` (an identical second year reaches the identical hash) |
| Demographics within data-defined bands | Same test: the crude annual death rate is asserted inside `demographics.ron`'s `annual_death_rate_{min,max}_per_mille` band — the thresholds are data, not code (ADR 0005 §6); golden-seed run ≈ 15‰ |
| Conservation trivially holds | Same test: no money or goods exist anywhere in a pure town (documented absence assertion); real auditors go live with the first ledgers in Phase 4 |

Additional coverage added this phase:

- Determinism suite now runs a living town (300 citizens + fixture) through
  every same-seed and save/load/resume comparison, so needs decay,
  mortality draws, deaths, and household dissolution are all inside the
  hash comparisons — including a mid-flight save with live scheduled state.
- `save_compat` covers all three format versions with byte-frozen fixtures:
  v1 and v2 load through the real migration chain to re-recorded goldens
  (re-record justified: the registration set grew — ADR 0004 §10 /
  ADR 0005 §9 — with byte-for-byte content continuity proven by
  `persistence::migrations::tests::v1_fixture_content_survives_migration_verbatim`);
  new v3 fixture (town + fixture saved mid-flight at tick 3,000) loads and
  resumes across a day boundary to recorded goldens.
- Migration unit tests: v1 chain-migration field pass-through; v2→v3
  event-name extension restoring strictly into the grown registration.
- `core_events::extend_registration_bytes` proven lossless (old state
  intact, new type usable, identical subsequent behavior).
- Data validation: seeded errors for weight sums, non-ascending mortality
  bands, inverted need ranges, empty name lists, out-of-range traits
  (`data_defs::tests::people_validation_catches_seeded_errors`).
- CLI: unit tests for the new subcommands and stats CSV shape
  (`cli::tests::run_save_inspect_demography_round_trip`), plus the
  existing binary-level exit-code suite.

## Verification process

`scripts/check.sh` green at the tagged commit. As in Phases 0–1, an
adversarial multi-agent review (determinism, correctness, standards,
anti-shortcut, test-coverage; every finding independently re-verified
against code and spec, several by mutation/reproduction experiments) ran
after the implementation commit (`a3d9b1a`). It confirmed 27 findings
(4 refuted), all fixed in the "Phase 2 review fixes" commit preceding the
`phase-2` tag. The most significant:

- **`run --load` schedule trap (major):** resuming a town save without
  re-passing `--citizens` silently ran zero people systems (frozen town,
  exit 0). Fixed: the CLI now derives the schedule from the save's actual
  content — the save is authoritative for the world's composition exactly
  as it is for the seed — with `--fixture`/`--citizens` ignored (and noted)
  on `--load`. This also discharges the Phase 0 report's open
  "schedule reconstruction is caller's responsibility" limitation.
  Regression test: `people::cli_resume_without_flags_matches_uninterrupted_run`.
- **Unvalidated save-vs-data vectors (major):** need/trait vectors from a
  save were silently zip-truncated against a changed data file. Fixed:
  `sim_people::validate_town` runs on every CLI load, and
  `NeedsDecaySystem` guards lengths (typed error, never reinterpretation).
- **`NeedLevel(i64)` newtype (major):** shipped as promised by ADR 0005 §2
  (serde-transparent, so the save format and goldens were untouched —
  proven by the unchanged save_compat suite).
- **Untested death path / genesis / decay (majors):** new behavioral suite
  `tests/tests/people.rs` — certain-death day (multi-death household
  dissolution under maximum contention), partial-mortality household-graph
  consistency, deaths replaying identically across save/load, exact decay
  arithmetic, and genesis distribution/membership invariants at n=5,000.
- **Missing calendar/age cross-check:** `validate_calendar_age_fit` now
  rejects data whose ages × year length overflow tick arithmetic (the
  check ADR 0005 §6 promised); mortality probabilities above 1.0 are also
  rejected. The v2 golden fixture gained its byte-for-byte
  content-continuity migration test, matching v1's.
- ADR 0005 amended where the review found the code's (better) choices
  undocumented: single genesis stream, schema-location rule.

## Deviations from spec (all pre-declared in ADRs)

- All Phase 2 components live in `sim_people`, not `sim_interface` —
  nothing is shared between sim crates yet (ADR 0005 §1).
- Needs/traits/probabilities are fixed-point vectors rather than one
  newtype per unit; the units are documented at the type and enforced by
  validation (ADR 0005 §2).
- Appending registrations bumps the save format (v3) with a mechanical
  list-extension migration — the permanent pattern for later phases
  (ADR 0005 §9).
- `data_defs` depends on `sim_people` (service → sim, downward): config
  schemas live with the crate that consumes them (ADR 0005 §6).

## Known limitations (clean seams, not fakes)

- Needs have no satisfaction sources and no consequences yet: Phase 3
  (AI acts on needs) and Phase 4 (goods satisfy them) own those. Citizens
  age and die; nothing else happens to them — exactly the SPEC's "no AI
  yet — needs decay and people age".
- No births (Phase 7 reproduction), so population is monotonically
  non-increasing; asserted as such in the exit suite.
- Age-band demographics of the *surviving* population are observable via
  `demography` but only the death-rate band is a hard assertion; richer
  statistical bands (age-pyramid drift, Gini etc.) join the golden-seed
  suite when the systems that move them exist (SPEC §14).
- Health, education, and skills (SPEC §15 lists health/education under
  `sim_people`'s eventual contents) arrive with the phases that consume
  them (Phase 6 services, Phase 7 education pipeline).

## Performance note (no budget applies until Phase 9)

10,000 citizens × one simulated year (172,800 ticks, hourly needs decay,
daily mortality) ≈ 6 s in a debug build, ~0.5 s in release. The full
workspace suite ≈ 25 s debug. Headroom is ample for the Phase 8 budget
work to build on.
