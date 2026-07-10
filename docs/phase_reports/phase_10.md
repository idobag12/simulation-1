# Phase 10 report — the viewer & polish

- Date: 2026-07-10
- Scope: SPEC §15 Phase 10 (full egui viewer: map, overlays, inspector,
  dashboards, event browser, time controls, snapshot protocol) against
  §13's tool list and §4's dependency rule
- Design decisions: ADR 0013 (written before the code)
- Verdict: **complete** — the exit criterion is demonstrated by a
  CI-blocking test over the tooling layer; `scripts/check.sh` green.

## What was built

| Deliverable | Where |
|---|---|
| The load-bearing split: a TOOLING layer (snapshot protocol + pane builders, plain data, unit-tested — it carries the exit criterion) under a PAINT layer (egui, draws and forwards clicks, computes nothing) — CI has no display, and neither does the proof need one | `crates/viewer/src/{snapshot,panes,events}.rs` vs `app.rs` (ADR 0013 §1) |
| The snapshot protocol: `Snapshot::capture(&World, …)` — the single function that touches the world; citizens (position, tier, needs, wallet+deposit, job, home, rent, current action, plan window, skills, bonds, beliefs, the last decision's full scored candidates), locations (kind, district, price, stock), the decoded event window, the narrative lines, district rent asks, and the last tick's timing table | `crates/viewer/src/snapshot.rs` |
| The app: one sim thread stepping the SAME schedule the CLI runs; time controls (`Pause`/`Step`/`Run{n}`) choose only HOW MANY ticks — never what happens in them; `Run(n)` steps n ticks PER COMMAND (the UI sends one per painted frame, so the frame rate paces the sim and the backlog collapses to the latest command); replies are `SimMessage::{Snapshot, Halted(reason)}` — failure is a rendered message, never a silent freeze; `--load` resumes any save through the CLI's `validate_town` guard, `--seed/--citizens` starts fresh | `crates/viewer/src/main.rs`, `app.rs::{SimCommand,SimMessage}` (ADR 0013 §9) |
| Map + overlays: district columns (the honest rendering of an abstract-matrix town), presence dots colored by the active overlay, click-to-focus (locations AND citizens); wealth quantile classes (ties rank low), need bands (five reachable), LOD tier coloring, posted-price location bands, commute flows (home→work district counts, corrupt ids filtered), the focused citizen's relationship graph resolved to live positions | `panes.rs::{map_columns,present_at,overlay_class,location_overlay_class,commute_flows,relationship_graph}` |
| Inspector: search by name/id, focus, follow mode (`follow_focus` re-resolves per snapshot; despawn renders a placeholder, never a silent blank); the full SPEC §13 state list (needs by name, action, sleep window, skills, bonds, beliefs, the scored decision dump) plus the citizen's last 50 attributable events and their story lines (attributed by SUBJECT INDEX, not name matching); locations are inspectable too (kind, district, price, stock, employees, presence, events) | `panes.rs::{search,inspect,follow_focus,inspect_location}`, `snapshot.rs::CitizenRow` |
| Event browser: the retained window decoded to subject-tagged readable rows — 15 decode arms including the wage-flow/housing set (`tax_collected`, `loan_granted`, `home_sold`, `home_built`, `price_changed`); conjunctive entity/type/tick-range filters; the narrative view on the same window | `events.rs::{decode_window,filter}` |
| Dashboard: the `debug_tools::metrics` registry — nine registered extractors (population, money supply, employment, homelessness, mean price, tiers, receipts, deposits) sampled by the sim thread at EVERY crossed day boundary into bounded series that travel inside the snapshot; the pane draws them as sparkline polylines (`normalize_series` is tooling, tested); a pull registry OUTSIDE the world (observability, not state — saves and determinism cannot see it) | `crates/debug_tools/src/metrics.rs`, `panes.rs::normalize_series` |
| The timing table: `Schedule` optionally collects per-system elapsed micros for the last tick (off by default; wall time never enters world state) — closing ADR 0012 §5's deferral of per-system attribution | `crates/core_ecs/src/system.rs::{enable_timing,last_timings}` |
| `System: Send` so an application may drive the schedule from its own thread — systems hold only immutable config | `crates/core_ecs/src/system.rs` |

## Exit criterion → proof

| Criterion (SPEC §15 Phase 10) | Test |
|---|---|
| An observer can follow one citizen's week and reconstruct their story entirely from tooling | `viewer::a_citizens_week_reconstructs_entirely_from_tooling` — a real 250-citizen town runs eight days; the observer finds an employed citizen by NAME SEARCH, then follows them through fourteen snapshots (noon + evening daily) using only the tooling layer: follow mode re-resolves them every sample, their position moves across distinct places, distinct activities are observed from the action field, finances track every sample, the decision dump carries scored candidates, the event browser attributes real events to them INCLUDING their economic life (filters proven conjunctive), the dashboard's full week of every registered series is read from inside the snapshot and normalizes into sparkline geometry, the map places them wherever they stand, every overlay classifies them, the timing table renders, and the narrative view speaks whenever chain-source facts remain in the window — with every line attributable by subject index |

Also: `viewer::pane_builders_classify_filter_and_lay_out_deterministically`
bites each pane builder's pure semantics (search edge cases, overlay
class bounds for citizens and locations, exhaustive + stable map
layout, real-district commute flows, location inspection, conjunctive
filters); `viewer::overlay_and_pane_edge_semantics` pins every
review-found classification defect (quantile ties, need band reach,
corrupt districts, follow/despawn, relationship graph resolution,
sparkline normalization); `viewer::metrics_registry_samples_and_evicts`
pins the registry's extractor arithmetic and CAPACITY eviction.

## Verification process

`scripts/check.sh` green at the tagged commit (fmt, clippy `-D
warnings`, full workspace suite including the viewer crate and its
exit suite, release LOD + map suites). Adversarial multi-agent review
(ultracode): three reviewers over the phase diff (conformance,
correctness/threading, edge cases — the phase touches no persisted
state and no economy path, so those dimensions have no new surface),
findings verified against the code before acting.

### Findings and how they were resolved

**Conformance (15 findings).** The paint layer was thinner than SPEC
§13: overlay radios and the follow checkbox were dead controls, the
dashboard rendered three headline strings, firms weren't inspectable,
and the inspector showed a fraction of `CitizenRow`. All fixed: the
map colors presence dots by `overlay_class` and locations by the price
overlay, every need is selectable by name, `follow_focus`/placeholder
semantics are tooling functions, `inspect_location` makes every
location a first-class inspector target, the inspector renders the
full row (needs, action, sleep window, skills, bonds as a clickable
relationship graph, beliefs, the scored decision dump), and the event
browser gained the tick-range inputs. `decode_window` is now
crate-internal, `Snapshot::capture` propagates the composer's error
instead of swallowing it, and stories carry subject indices
(`stories_with_subjects`) so attribution never name-matches.

**Correctness/threading (9 findings, 3 major).** (1) The metrics
registry was sampled on the sim thread but never reached the UI — the
dashboard's series feature was dead code; series now ride inside the
snapshot and the sampler fires at every crossed day boundary (a
multi-day step records every day). (2) The sim loop consumed one
command per iteration while the UI produced one per frame, so a pause
could sit behind an unbounded backlog of stale runs; the loop now
collapses the backlog to the latest command. (3) `Run` free-ran the
sim unthrottled, flooding an unbounded channel with discarded
snapshots; `Run(n)` now steps n ticks per received command, so the
frame rate paces the sim and each command yields one snapshot. Also
fixed: the sim thread dying silently on audit-halt/capture error (a
`Halted(reason)` message renders in the time bar), `run_tick_coarse`
never clearing the timing table (regression-tested in `core_ecs`),
the viewer's `--load` skipping the CLI's `validate_town` guard, and
the missing event decode arms.

**Edge cases (13 findings).** The overlapping majors above, plus: the
need heatmap's top band was mathematically unreachable
(`level/250_001` maxes at 3 on a 0..=1M clamped scale; now
`/200_000`); wealth quantiles ranked ties high, so a uniform-wealth
town rendered entirely "richest" (ties now rank low — all class 0);
corrupt district ids flowed verbatim into the commute pane while the
map filed them under "(unsited)" (now filtered, panes agree); the
inspector went silently blank when the followed citizen died (now a
placeholder, their events stay findable); `--seed/--citizens` were
silently ignored with `--load` (now the CLI's note); the documented
`embervale-viewer` binary didn't exist (now a `[[bin]]` target). Each
classification defect is pinned by `viewer::overlay_and_pane_edge_semantics`;
the registry's extractor arithmetic and CAPACITY eviction by
`viewer::metrics_registry_samples_and_evicts`.

All amendments are recorded in ADR 0013 §9.

## Deviations from SPEC

- The dashboard's metrics registry is PULL (extractors registered in
  code) rather than a push API systems write to — every SPEC §13
  dashboard need is covered; the push API is ADR 0013 §8's documented
  deferral.
- The map is district columns, not geometry: the honest rendering of a
  town whose whole spatial model is the travel matrix (ADR 0012 §1,
  ADR 0013 §8).
- Time controls have no seek/rewind: time only moves forward (SPEC
  §5); replay is load-a-save (ADR 0013 §8).
- Per-system timing uses `Instant` around system calls rather than
  `tracing` spans — same table, no new dependency; wall time stays
  outside world state.

## Tag

`phase-10` (local; tag pushes are blocked by the environment's proxy —
the branch carries the content).
