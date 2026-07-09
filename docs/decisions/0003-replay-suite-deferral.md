# ADR 0003 — Deferral of replay-from-input-log (SPEC §9/§14c) until inputs exist

- Status: Accepted
- Date: 2026-07-09
- Phase: 0

## Context

SPEC §9 defines replay mode: "because the sim is deterministic and player
actions are the only external inputs, a save can be `{seed + input log}`",
and SPEC §14(c) lists "replay from input log matches full-state run" in the
determinism suite, CI-blocking from Phase 0. SPEC §16.4 requires any
deviation to be recorded in an ADR before the deviating code.

## Decision

The input-log replay test is deferred until the first external-input path
exists in the simulation. It is **not** implemented in Phase 0.

Rationale: in Phase 0 (and through at least Phase 2) there are no player
actions and no external inputs of any kind; the input log of every run is
empty by construction. A replay of `{seed + empty log}` is definitionally
identical to a same-seed fresh run — which the suite already tests as case
(a) (`two_fresh_empty_runs_of_one_million_ticks_are_hash_identical`,
`two_fresh_fixture_runs_are_hash_identical`). Implementing an input-log
recorder, a replay driver, and a log format with nothing that can ever
populate the log would be a placeholder mechanism (SPEC §16.1) and
speculative complexity (SPEC §16.3): the log schema would be invented
without a single real input type to constrain it.

## Consequences

- The phase that introduces the first external input (player action,
  observer nudge — expected no later than the viewer/inhabit work, possibly
  earlier) MUST in the same phase deliver: the input log in the save format
  (`format_version` bump + migration), the replay driver, and determinism
  suite case (c) comparing a replayed run against the full-state run every
  10k ticks. That phase's report must reference this ADR as discharged.
- Until then, determinism cases (a) and (b) are the complete suite; the
  deferral is restated in `tests/tests/determinism.rs` and each phase
  report so it cannot be silently forgotten.
