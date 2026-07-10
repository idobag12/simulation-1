# ADR 0013 — Phase 10 design: the viewer & polish

- Status: Accepted
- Date: 2026-07-10
- Phase: 10

SPEC §15 Phase 10 ("Full egui viewer: map, overlays, inspector,
dashboards, event browser, time controls, snapshot protocol. Exit: an
observer can follow one citizen's week and reconstruct their story
entirely from tooling") against §13's tool list and §4's dependency
rule (`viewer → headless → sim_* → core_*`, snapshots only). Per SPEC
§16.7 the decisions precede the code; per §16.3 each pane ships in its
smallest honest form the exit criterion exercises.

## 1. The load-bearing decision: logic below, paint above

The exit criterion is about TOOLING, and this environment (like CI)
has no display. So every pane is split in two:

- **The tooling layer** (`viewer::snapshot`, `viewer::panes::*` data
  builders — plain functions and structs, no egui types): everything a
  pane KNOWS — what the inspector assembles for an entity, what rows
  the event browser yields for a filter, what points a dashboard plot
  holds, what the map lays out where, what an overlay colors how.
  This layer is unit-tested and carries the exit criterion.
- **The paint layer** (`viewer::app`, egui/eframe): draws the tooling
  layer's outputs and forwards clicks. It contains NO decisions worth
  testing — every `if` that matters lives below.

CI proves the exit criterion by driving the tooling layer over a real
week-long run: if the data structures the widgets render contain the
citizen's whole week, the observer following the widgets sees it.

## 2. The snapshot protocol

`viewer::snapshot::Snapshot`: an owned, read-only view of one tick,
built by `Snapshot::capture(&World, tick)` — the ONLY function that
touches `World`, and the viewer's whole dependency on the sim.
Contents: citizens (entity, name, position, district, tier, needs,
wallet, employment, home), locations (kind, district, prices, stock),
the housing/bank/treasury/labor ledgers, the metrics registry's
current series, the event page for the retained window, the narrative
lines, and the last tick's per-system timing table. Capture cost is
bounded (one pass per store); the sim thread captures on demand, not
per tick.

## 3. The app: one sim thread, one UI thread

`embervale-viewer` (a `viewer` binary target): loads data + save (or
genesis) exactly like the CLI, then runs the simulation on ITS OWN
thread. Time controls send commands over a channel (`Pause`, `Step {
ticks }`, `Run { ticks_per_frame }`, `Seek` is NOT offered — time only
moves forward, SPEC §5); the sim thread replies with fresh snapshots.
The UI never blocks the sim; the sim never paints. Determinism is
untouched: the sim thread steps the same schedule the CLI runs, and
commands only choose HOW MANY ticks to step — never what happens in
them.

## 4. The panes

- **Map**: no geometry exists (ADR 0012 §1), so the map is HONEST
  about being abstract: districts render as columns ordered by data
  index; locations stack within their district (kind glyph + name);
  citizens dot their current location. Selecting anything focuses the
  inspector.
- **Overlays** (SPEC §13's list, one color rule each, all derived in
  the tooling layer): wealth choropleth (wallet+deposits, quantile
  bands), need heatmap (chosen need's level), price by store, LOD
  tier coloring, commute flows (home→workplace district pairs,
  thickness = count), and the selected citizen's relationship graph
  (edges drawn to bond targets, kind-colored).
- **Inspector**: search by name/id; click-to-focus; follow mode (the
  focused entity's row refreshes with every snapshot). Shows the full
  SPEC §13 list: needs, wallet/deposit ledger, employment + schedule
  (plan, current action), skills, relationships (kind, name,
  strength), beliefs, the entity's last 50 events (from the event
  page, subject-filtered), and the last decision's full scored
  candidate dump.
- **Dashboard**: the `debug_tools` METRICS REGISTRY — a named list of
  world-state extractors (population, money supply, unemployment,
  mean posted price, homelessness, tier counts, treasury receipts…)
  sampled once per day into bounded ring series by a day-rate
  `MetricsSystem`; the pane plots any selected series as polylines.
  Registered in code (a pull registry — systems do not push; the
  "any system can publish" wording ships as "any crate can register
  an extractor", documented deferral of a push API).
- **Event browser**: filter by entity, event type, and tick range
  over the retained ring; the narrative composer view sits on top
  (the same `debug_tools::stories`, subject-filterable).
- **Time controls + timing**: pause/step-tick/step-day/run at N
  ticks-per-frame; the per-system timing table (SPEC §13's tracing
  row — delivered as `Schedule`-collected per-system elapsed micros
  for the LAST tick, exposed read-only; wall time never enters world
  state, it is observability only). This also closes ADR 0012 §5's
  deferral of per-system attribution.

## 5. Metrics and timing plumbing

- `debug_tools::metrics`: `MetricsRegistry { series: Vec<(name,
  ring)> }` as a persisted-free RESOURCE — it lives in the viewer/CLI
  layer, not the world (it is observability, not state; saves ignore
  it, catch-up skips it, determinism cannot see it). A day-rate
  sampler fills it from `Snapshot`-grade reads.
- `core_ecs::Schedule` gains an optional timing collector: when
  enabled (viewer only), each system run records elapsed micros into
  a table the app reads. Off by default; the CLI and tests never pay.

## 6. Dependencies

`eframe`/`egui` (SPEC §2 sanctions them for this crate), pinned at the
workspace level, used ONLY by `viewer`. No other crate gains a
dependency. The paint layer is `cfg`-free plain code; CI builds it but
never opens a window (`cargo build -p viewer` compiles the app; tests
exercise the tooling layer).

## 7. Exit criterion mapping

*An observer can follow one citizen's week and reconstruct their story
entirely from tooling*: a CI test runs a real town seven days,
capturing a snapshot each day and driving ONLY the tooling layer — the
inspector's assembled view (identity, home, job, needs, bonds, last
decision), the event browser filtered to the citizen, the dashboard
series, and the narrative view — and asserts the reconstruction is
genuinely a WEEK OF LIFE: the citizen is findable by name search;
their position/action changes across the days; their purchases and
wage flows appear as events attributable to them; their decision dump
carries scored candidates every day; and the composed story lines
cite them. Plus unit tests per pane builder (overlay color mapping,
event filtering, metric sampling, follow-mode focus, map layout
determinism).

## 8. Deferrals (documented)

- A push-API for metrics (systems publishing mid-tick): the pull
  registry covers every SPEC §13 dashboard need today.
- Map geometry (coordinates, sprites): the district-column layout is
  the honest rendering of an abstract-graph town.
- Seek/rewind time controls: time only moves forward (SPEC §5); replay
  is load-a-save.

## 9. Amendments (Phase 10 adversarial review)

The review bench (three reviewers over the phase diff) forced these
corrections to the sections above; the code ships the amended form.

- **§3 run semantics.** `Run { ticks_per_frame }` as a sticky sim-side
  mode was wrong twice: an unthrottled sim loop (the slider controlled
  nothing; snapshots flooded an unbounded channel) and a starved pause
  (one command consumed per iteration while the UI enqueued one per
  frame). Amended: `Run(n)` steps `n` ticks FOR THAT COMMAND; the UI
  sends one per painted frame while running, so the frame rate paces
  the sim and each command yields exactly one snapshot. The sim thread
  collapses any command backlog to the LATEST command, so pause wins
  immediately.
- **§3 failure honesty.** The reply channel carries `SimMessage::
  {Snapshot, Halted(reason)}` — a halted audit, a failed capture, or a
  failed metrics sample is a rendered message, never a silent freeze.
- **§3 load parity.** The viewer's `--load` now runs the same
  `sim_people::validate_town` guard as the CLI (a save/data mismatch
  is a startup error, never a silent reinterpretation) and prints the
  CLI's note when `--seed`/`--citizens` are ignored; the binary is
  really named `embervale-viewer` (`[[bin]]`).
- **§4/§5 metrics plumbing.** There is no day-rate `MetricsSystem` —
  the sim thread samples the registry at every crossed day boundary
  (a multi-day step samples every day, not just the last), and the
  series travel INSIDE the snapshot, so the dashboard renders from
  the same object as every other pane. §2's "metrics registry's
  current series" is literal.
- **§4 inspector scope.** Locations (firms, homes, venues) are
  inspectable too: `panes::inspect_location` assembles kind, district,
  price, stock, employees, presence, and the location's events.
- **§4 narrative attribution.** Story lines carry their subject entity
  indices (`debug_tools::stories_with_subjects`); the inspector
  filters by INDEX, never by name matching (names collide).
- **Classification fixes** (each a reviewer-proven defect): the need
  heatmap's top band was unreachable (`/250_001` → `/200_000` over the
  clamped 0..=1M scale); wealth quantiles ranked ties HIGH (a uniform
  town rendered all-richest; ties now rank low); corrupt district ids
  reached the commute pane (now filtered, agreeing with the map's
  "(unsited)" column); `econ.loan_granted`/`home_sold`/`home_built`/
  `tax_collected`/`price_changed` had no decode arms, so a citizen's
  wage-flow taxes and housing deals were invisible to the subject
  filter (arms added); `Schedule::run_tick_coarse` never cleared the
  timing table (rows accumulated across coarse ticks; now cleared like
  `run_tick`).

## Dependencies

`eframe` + `egui` (new, `viewer` only).
