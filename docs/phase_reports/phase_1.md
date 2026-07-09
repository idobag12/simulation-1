# Phase 1 report — Time, events, data loading

- Date: 2026-07-09
- Scope: SPEC §15 Phase 1 (Calendar, scheduler, event bus + log, RON
  loading + validation, balance-file plumbing)
- Design decisions: ADR 0004 (written before the code)
- Verdict: **complete** — both exit criteria demonstrated by CI-blocking
  tests; `scripts/check.sh` green.

## What was built

| Deliverable | Where |
|---|---|
| Typed event bus: `emit` → next-tick read, emission order preserved, stable `Event::NAME` identity, strict registration (ADR 0004 §§3–4) | `crates/core_events` |
| Event log ring: bounded by `event_log_capacity` from `data/balance/engine.ron`, observability-only, serialized/hashed with world state | `crates/core_events` |
| Future-event scheduler: `(due tick, insertion sequence)` ordering, fires at start-of-tick ahead of rotated emissions, saved/restored exactly (ADR 0004 §2) | `crates/core_events` |
| `World` owns event state; systems reach it via `world.emit/events/schedule_event`; tick loop drives `begin_tick` (ADR 0004 §1) | `crates/core_ecs` |
| `Calendar`: tick → year/season/day/hour/minute; structure definitional (1440 ticks/day, 4 seasons), `days_per_season` tunable from `data/balance/calendar.ron` | `crates/sim_time`, `crates/core_types/src/calendar.rs` |
| Multi-rate schedules: tick/hour/day/season/year lists, boundary ticks fire coarsest-first (ADR 0004 §7); `TickContext` carries `CalendarTime` | `crates/core_ecs/src/system.rs` |
| Strict RON loader/validator: missing file, unknown field, syntax error, out-of-range value ⇒ precise typed errors naming the file; never a silent default (SPEC §8) | `crates/data_defs`, `data/balance/*.ron` |
| Save format v2 (+ event state); frozen `SaveBodyV1`; pure `v1→v2` migration (empty-marker events blob); the Phase 0 golden fixture loads through the real migration in CI | `crates/persistence` |
| v2 byte-frozen golden fixture saved mid-flight (live scheduled alarms, populated log ring, pending emissions) | `tests/fixtures/v2_seed13_fixture60_tick2000.embersave` |
| Fixture events: per-tick churn emissions + a self-perpetuating scheduled alarm chain, so every determinism run exercises the bus and scheduler | `crates/headless/src/fixture.rs` |
| CLI `--data DIR` flag: all world assembly flows through loaded, validated data (SPEC §8) | `crates/headless/src/cli.rs` |

## Exit criteria → proof

| Criterion (SPEC §15 Phase 1) | Test |
|---|---|
| Data validation catches seeded errors | `data_defs` unit suite: `missing_file_names_the_path`, `unknown_field_is_a_parse_error_not_a_silent_default`, `syntax_error_is_a_parse_error`, `out_of_range_values_are_validation_errors_with_precise_messages`; end-to-end: `cli::tests::missing_data_directory_is_a_precise_startup_error` |
| Scheduled events fire deterministically across save/load | `events::scheduled_events_fire_deterministically_across_save_load` — the fixture's alarm chain recorded delivery-by-delivery in an uninterrupted run vs. a run saved/loaded mid-chain; identical `(tick, generation)` logs. Plus `core_events` unit suite (`scheduled_events_fire_at_their_tick_in_sequence_order_before_emissions`, `save_restore_round_trips_mid_flight_state`, `past_due_entries_fire_at_the_next_tick_start`) |

Additional coverage added this phase:

- `rates::rates_fire_exactly_on_their_boundaries_over_two_years` — hour/
  day/season/year counts over two simulated years through the real Calendar
  + tick loop, plus coarsest-first order on boundary ticks (ADR 0004 §7);
  `core_ecs` unit tests cover per-rate order and mid-boundary ticks.
- `events::event_log_capacity_cannot_affect_trajectories` — two capacities,
  same seed: byte-identical entities/components/RNG (ADR 0004 §5), while
  the retained logs differ (the bound demonstrably works).
- `save_compat` now covers both format versions with byte-frozen fixtures
  and golden hashes: v1 loads **through the real migration** (empty event
  state, content continuity proven byte-for-byte by
  `persistence::migrations::tests::v1_fixture_content_survives_migration_verbatim`),
  v2 loads with live scheduled entries and resumes to a recorded golden.
- Determinism suite unchanged in intent, now with events in the hash
  domain and the fixture emitting/scheduling every tick; the mid-flight
  save in `save_load_continue_matches_uninterrupted_run` asserts live
  scheduled entries exist at the save point.
- Calendar golden conversions and boundary-flag/tick-arithmetic
  equivalence over two full (short) seasons.

## Golden re-record (ADR 0004 §10)

Adding event state to `World::hash_into` extended the hash domain, so the
v1 fixture's golden hashes were re-recorded in the same commit that made
the change (`Phase 1` main commit), with content continuity proven by the
byte-for-byte migration test above. The v1 fixture bytes themselves are
untouched.

## Verification process

`scripts/check.sh` green at the tagged commit. As in Phase 0, an
adversarial multi-agent review (determinism, correctness, standards,
anti-shortcut, test-coverage dimensions; findings independently
re-verified) ran after the implementation commit; confirmed findings were
fixed in the "Phase 1 review fixes" commit(s) preceding the `phase-1` tag.

## Deviations from spec (all pre-declared in ADRs)

- Scheduler queue lives in `core_events` beside the bus (state must be
  inside `World`); `sim_time` drives it — ADR 0004 §1.
- Scheduled entries are future event emissions, not callbacks — ADR 0004 §2.
- Single delivery semantic (emit in T ⇒ readable in T+1); same-tick
  delivery deliberately absent — ADR 0004 §3.
- Migration intermediate representation = frozen versioned body structs —
  ADR 0004 §9.
- Registries with stable integer ids deferred to the first id-keyed content
  (Phase 4) — ADR 0004 §8.

## Known limitations (clean seams, not fakes)

- No catch-up controller (SPEC §5): Phase 8 scope, per SPEC §15.
- Replay-from-input-log still deferred (ADR 0003); no inputs exist yet.
- `Events::read` decodes on every call (allocation per read); fine at
  Phase 1 scale, revisit in the Phase 9 performance pass if profiling
  says so.
- The narrative consumer over the event log (SPEC §7) belongs to
  `debug_tools` and arrives with high-signal simulation events (Phase 7);
  the log itself, its ring bound, and its name resolution are live now.

## Performance note (no budget applies until Phase 9)

Full workspace test suite ≈ 12 s debug, including the 1M-tick empty run
and 50k-tick fixture runs with events in every hash.
