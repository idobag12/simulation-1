# Phase 0 report — Skeleton & determinism harness

- Date: 2026-07-09
- Scope: SPEC §15 Phase 0
- Verdict: **complete** — all exit criteria demonstrated by CI-blocking tests;
  `scripts/check.sh` green.

## What was built

| Deliverable | Where |
|---|---|
| Workspace with all 16 layered crates, downward-only dependency rules encoded in manifests | `Cargo.toml`, `crates/*/Cargo.toml` |
| Unit newtypes with checked typed-error arithmetic (`Money` in mills, `Ticks`, `Seed`), canonical codec (single bincode config), frozen FNV-1a stable hasher | `crates/core_types` |
| `RngRegistry`: named, independently seeded `Pcg64` streams; frozen in-house seed derivation (splitmix64 + FNV-1a); exact serde round-trip | `crates/core_rng` |
| ECS: generational `Entity`/allocator (LIFO free list), dense/sparse typed stores behind one canonical serialized form, `World` with stable-name component registration, `CommandBuffer`, `Schedule` with explicit ordered per-tick system list, attributable system errors | `crates/core_ecs` (~1,050 lines incl. ~340 lines of tests; non-test code within the ~800-line budget) |
| Fixed-timestep tick loop and canonical `state_hash()` (tick + entities + stores in registration order + RNG streams, all length-framed) | `crates/sim_time` |
| Save format v1: magic + version header outside a zstd-compressed canonical body; strict two-way component-blob matching; migration pipeline seam | `crates/persistence` |
| `embervale` CLI: `run` (CSV of tick,hash; save/load), `verify` (two fresh runs), `save-load-check` (uninterrupted vs resume); tri-state exit-code contract (0 ok / 1 mismatch / 2 error) implemented in a unit-tested library module with a thin binary shim | `crates/headless/src/cli.rs`, `src/main.rs` |
| Determinism fixture (random-walk balances, bounded population churn exercising spawn/despawn/free-list/sparse+dense stores/RNG) — tooling, never scheduled unless requested | `crates/headless/src/fixture.rs` (ADR 0002 §10) |
| Cross-crate determinism suite | `tests/tests/determinism.rs` |
| Byte-frozen golden v1 save fixture + compatibility suite enforcing "old saves must load forever" from day one | `tests/fixtures/v1_seed7_fixture50_tick1000.embersave`, `tests/tests/save_compat.rs` |
| CI-equivalent gate: fmt + clippy `-D warnings` + full test suite | `scripts/check.sh` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 0) | Test |
|---|---|
| Two runs of 1M empty ticks hash-identical | `determinism::two_fresh_empty_runs_of_one_million_ticks_are_hash_identical` (101 checkpoints at 10k-tick intervals, SPEC §9) |
| Save/load/continue matches | `determinism::save_load_continue_matches_uninterrupted_run` (hash **and** full serialized-state equality at tick 20,000 after a save/load at 10,000) |

Additional determinism suite coverage (SPEC §14a/b):

- `two_fresh_fixture_runs_are_hash_identical` — same-seed identity under real
  state churn (mutation, spawn/despawn, RNG advance), 50k ticks.
- `different_seeds_produce_different_trajectories` — the hash actually
  observes state (no vacuous equality).
- `save_load_preserves_rng_stream_positions` — streams resume mid-sequence
  exactly; lazily-created streams are identical before/after a round-trip
  (SPEC §9 "RNG streams are saved and restored exactly").
- `fixture_population_stays_bounded_and_churning` — the harness's churn
  cannot die out or explode, so long-run suites keep exercising state.
- `save_compat::v1_golden_save_loads_to_the_exact_golden_state` and
  `…_resumes_deterministically` — a committed, byte-frozen v1 save must
  load to a hardcoded golden state hash and evolve to a second golden hash
  100 ticks later. This pins the codec configuration, `SaveBody` layout,
  canonical store form, component names, RNG serialization, seed
  derivation, and hash algorithm across builds — the cross-build half of
  SPEC §9 that same-process round-trip tests cannot see.
- Unit suites per crate: fixed-point overflow edges, codec trailing-byte
  strictness, FNV-1a golden values (algorithm freeze), seed-derivation
  golden vectors computed by an independent implementation (splitmix64
  reference vector included), allocator recycle order across serde,
  canonical store bytes independent of layout/padding, command-buffer
  ordering, schedule order and error attribution, save
  header/version/registration-mismatch/io-vs-corruption errors.
- CLI coverage: `headless::cli` unit tests (flag parsing, usage errors, the
  tri-state dispatch contract, in-process save→load resume) plus
  `headless/tests/cli_binary.rs` spawning the real executable (exit codes 0
  and 2, `PASS:`/CSV stdout shape, file-based save→resume equality,
  missing-file error reporting). Exit code 1 (mismatch) is unreachable
  through a real invocation precisely because the sim is deterministic; its
  mapping is covered at the `dispatch` tri-state level.

## Verification process

`scripts/check.sh` (fmt check, clippy `-D warnings` across all targets, full
workspace test suite) is green at the tagged commit. After the initial
implementation commits, an adversarial multi-agent review (five dimensions:
determinism risks, correctness, coding standards, anti-shortcut directives,
test coverage; every finding independently re-verified against code and
spec before acceptance) produced ten confirmed findings — most notably a
tautological seed-derivation "freeze" test that recomputed its expected
values with the functions under test, the absence of a byte-frozen golden
save, and zero automated CLI coverage. All confirmed findings were fixed in
the "Phase 0 review fixes" commit(s) preceding the `phase-0` tag; refuted
findings and reasoning are recorded in the session log, and the
deferred-replay deviation the review surfaced is now ADR 0003.

## Deviations from spec (all pre-declared in ADRs)

- `System::run` takes `&mut CommandBuffer` and returns `Result` — ADR 0002 §5.
- RNG registry lives in `World`; `TickContext` stays immutable — ADR 0002 §4.
- Phase 0 schedule carries only the per-tick rate; hour/day/season/year
  rates arrive with the Calendar in Phase 1 — ADR 0002 §9.
- Migration "intermediate dynamic representation" deferred until the first
  real migration (v2) — ADR 0002 §8.
- Replay-from-input-log (SPEC §9/§14c) deferred until the first
  external-input path exists — ADR 0003 (with the obligation it places on
  the phase that introduces inputs).
- Repository root is the project root (no `embervale/` nesting) — ADR 0001.

## Known limitations (clean seams, not fakes)

- The schedule is reconstructed by the application on load (like component
  registrations), not recorded in the save. A caller who loads a fixture
  save without `--fixture` runs no systems; the hash trail makes this
  visible. Revisit when Phase 1 data-driven world assembly defines what a
  "world spec" is.
- No split-borrow query API (component iterator + RNG simultaneously);
  Phase 0's fixture pre-draws instead. Deliberately deferred to Phase 3,
  when real systems define the shape that API must take.
- `state_hash()` serializes state to hash it — fine for test/CLI paths; if
  it ever enters a hot path, Phase 9 owns optimizing it.

## Performance note (no budget applies until Phase 9)

Debug-build test suite ≈ 10 s including the 1M-tick empty run; release CLI
runs 100k fixture ticks (~200–400 entities) in well under a second.
