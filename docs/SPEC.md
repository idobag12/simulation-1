# EMBERVALE — PROJECT SPECIFICATION (PERMANENT CONTRACT)

> This is the permanent specification for the Embervale simulation project.
> Read it in full before writing any code, and re-read the relevant section
> before starting each phase. When any instruction here conflicts with a
> shortcut, this document wins.

## 1. Vision and non-goals

What this is: A deterministic, headless-first simulation of a living town.
Every citizen is an individual with needs, memories, personality,
relationships, wealth, health, education, goals, and a daily schedule. Every
business is a real firm with owners, employees, payroll, inventory, suppliers,
customers, debts, and taxes. Goods move through complete supply chains —
nothing spawns from nowhere, and money is conserved. Emergent narrative arises
from the interaction of systems, never from scripted events.

What this is NOT:

* Not a city builder. The player observes, nudges, and inhabits; they do not place zones.
* Not a graphics project. Rendering is a thin, replaceable layer over a headless core.
* Not a demo. Architecture decisions must survive months of expansion.

The four invariants (violating any of these is a bug of the highest severity):

1. **Determinism.** Same seed + same inputs ⇒ bit-identical world state at any tick, across runs.
2. **Conservation.** Money and goods are never created or destroyed except at explicitly modeled sources/sinks (central bank issuance, imports/exports, production recipes, spoilage). Every phase ships an audit that proves this.
3. **No magic.** No entity receives goods, money, employees, or state changes except through a modeled mechanism.
4. **Headless purity.** The simulation core compiles and runs with zero rendering dependencies.

## 2. Technology decisions (FIXED — do not revisit)

* Language: Rust (stable toolchain). Chosen for performance at 10k+ agents, fearless refactoring over a long project, and strong determinism guarantees.
* Workspace layout: Cargo workspace with strictly layered crates (§4).
* ECS: A thin custom archetype-free ECS built on generational indices and typed component stores (§6). Do NOT pull in bevy_ecs or specs; we need total control over iteration order for determinism and over storage for LOD tiers. Keep it under ~800 lines; it is infrastructure, not a product.
* Math: All economic quantities are 64-bit fixed-point integers (`Money = i64` in mills, `Quantity = i64` in item-specific base units). Floating point is permitted only in utility-AI scoring and rendering, never in anything persisted or economically conserved.
* RNG: `rand_pcg::Pcg64` exclusively, via a `RngRegistry` that hands out named, independently seeded streams per system (`rng("weather")`, `rng("ai.social")`). Never share one stream across systems — that couples their determinism.
* Serialization: `serde` + `bincode` for saves; RON for authored data definitions.
* Rendering/debug UI: `egui` + `eframe` in a separate crate, consuming read-only snapshots. Swappable later.
* No other dependencies without written justification in `docs/decisions/` (ADR format).

## 3. Coding standards

* `rustfmt` defaults and `clippy` with `-D warnings` on CI-equivalent local script (`scripts/check.sh`). No `#[allow]` without a comment explaining why.
* No `unwrap()`/`expect()` in simulation code. Errors are typed (`thiserror`) and propagate. `expect()` is permitted only in `main()`, tests, and tooling.
* No `unsafe` anywhere except the ECS storage layer, and only with a `// SAFETY:` proof comment.
* Every public type and function has a doc comment stating its invariants, not just its purpose.
* Units in types, not comments. `struct Mills(i64)`, `struct Ticks(u64)`, `struct Grams(i64)` — newtypes for every unit. Adding dollars to grams must not compile.
* Functions over 60 lines require justification; modules over 500 lines must be split.
* Iteration order is part of correctness. Any iteration over entities, hash maps, or events in simulation code must be over deterministic structures (sorted keys, `Vec`, `BTreeMap`, or entity-index order). `HashMap` iteration in a simulation system is a bug even if tests pass.
* Every commit message references the phase and system it advances.

## 4. Project structure

```
embervale/
├── Cargo.toml                    # workspace
├── crates/
│   ├── core_ecs/                 # entities, components, world, scheduler
│   ├── core_types/               # newtypes: Money, Ticks, Ids, fixed-point math
│   ├── core_rng/                 # RngRegistry, named deterministic streams
│   ├── core_events/              # event bus, event log, deterministic dispatch
│   ├── sim_time/                 # tick loop, calendar, scheduling, catch-up controller
│   ├── sim_people/               # needs, health, education, lifecycle, schedules
│   ├── sim_ai/                   # utility AI, planners, memory, relationships
│   ├── sim_economy/               # markets, prices, firms, labor, banking, taxes
│   ├── sim_goods/                 # item defs, recipes, inventories, logistics
│   ├── sim_world/                 # town map, buildings, housing, pathing (abstract)
│   ├── sim_lod/                   # simulation level-of-detail tiers and promotion
│   ├── persistence/               # save/load, versioning, migrations, replay
│   ├── data_defs/                 # RON schema types + loader + validator
│   ├── debug_tools/               # inspector queries, invariant auditors, tracing
│   ├── headless/                  # CLI runner: run N ticks, dump stats, verify
│   └── viewer/                    # egui app; depends on snapshots only
├── data/
│   ├── goods/*.ron                # item definitions
│   ├── recipes/*.ron              # production chains
│   ├── professions/*.ron
│   ├── firms/*.ron                # firm archetypes
│   ├── names/*.ron
│   └── balance/*.ron              # tunable constants — NOTHING tunable lives in code
├── docs/
│   ├── decisions/                 # ADRs, numbered
│   └── phase_reports/             # written verification report per phase
├── scripts/check.sh               # fmt + clippy + test + determinism suite
└── tests/                         # cross-crate integration + determinism harness
```

Dependency rule: arrows only point downward. `viewer` → `headless` → `sim_*` → `core_*`. No `sim_` crate may depend on another `sim_` crate except through events and shared components declared in a small `sim_interface` module inside `core_ecs`. If two systems need to talk, they do it through components or events — never direct calls.

## 5. Time simulation

* Fixed timestep. 1 tick = 1 simulated minute. 1440 ticks/day. The tick is the only unit of causality; wall-clock time never touches simulation code.
* `sim_time` owns a `Calendar` (tick → year/season/day/hour/minute) and a `Scheduler` for future callbacks: a deterministic `BTreeMap<Ticks, Vec<ScheduledEvent>>` (vec order stable by insertion sequence number).
* Three simulation rates coexist (see §10 LOD): per-tick systems (active agents), per-hour systems (needs decay, firm bookkeeping), per-day systems (payroll, market clearing for slow goods, rent), per-season and per-year systems (harvests, tax filing, aging).
* Offline catch-up: when a save is loaded with elapsed real time (or the player fast-forwards), the catch-up controller runs the same systems in aggregate mode: per-day and per-hour systems run normally, but per-tick agent behavior is replaced by each agent's statistical day model (§10). Catch-up is still deterministic — it is a defined coarse integrator, not a skip. Budget: 1 simulated week of catch-up must complete in under 5 real seconds at 10k citizens.
* Speed controls (pause, 1×, 10×, 100×) live entirely outside the core; they just change how many ticks the runner requests per frame.

## 6. ECS and scheduling architecture

* `Entity = { index: u32, generation: u32 }`. Component stores are `Vec<Option<T>>` or sparse `BTreeMap<u32, T>` depending on density (choose per component, document the choice).
* Systems are plain structs implementing `System { fn run(&mut self, world: &mut World, ctx: &TickContext); }` registered in an explicit, ordered list per rate (tick/hour/day/season/year). The schedule is a literal `Vec` in one file — no dependency-graph magic. Order is documented and is part of the spec.
* Systems communicate through components and the event bus only. A system may not hold references into the world between runs.
* Command buffer pattern: systems queue entity creation/deletion/component changes into a `CommandBuffer`, applied at a defined point after each system runs, so mid-iteration mutation never occurs.
* Queries are explicit typed iterators in entity-index order. No parallelism in Phase 0–8; the design must permit adding deterministic parallelism later (systems declare read/write component sets), but do not build it speculatively.

## 7. Event system

* `core_events` provides a typed event bus: `emit<E: Event>(e)`, consumed by systems that declare interest. Events are delivered in emission order, buffered per tick, and drained at defined schedule points (start-of-tick and end-of-tick queues).
* Every event is serializable and logged to a ring buffer (`EventLog`, capacity configurable) with its tick. This log powers the debugger, the story extractor, and replay verification.
* Events are facts, not commands: `PersonHired { person, firm, wage }`, `PriceChanged { good, market, old, new }`, `PersonDied { person, cause }`. Systems react to facts; nothing "sends orders" through the bus.
* A `narrative` consumer in `debug_tools` subscribes to high-signal events and composes human-readable story lines ("Mara Voss lost her job at the mill, defaulted on rent, and moved in with her sister"). This is how emergent stories surface — by observation, never by injection.

## 8. Data-driven definitions

* All content — goods, recipes, professions, firm archetypes, need curves, personality trait definitions, balance constants — lives in RON files under `data/`, loaded at startup into immutable registries with stable integer ids.
* `data_defs` includes a validator run at startup and in tests: every recipe input references a defined good; every profession references defined workplaces; every good has a producer or an import source; the recipe graph has no orphaned goods. Validation failure is a startup error with a precise message, never a silent default.
* No hardcoded values. If a number could plausibly be tuned (a decay rate, a wage floor, a price elasticity), it lives in `data/balance/`. The reviewer test: grep the sim crates for numeric literals other than 0, 1, and array indices — each hit must be justified.

## 9. Save / load / replay

* A save is: `{ format_version, seed, tick, full world state (all components, registries hashes, RNG stream states, scheduler queue, event ring tail) }`, serialized with bincode, zstd-compressed.
* RNG streams are saved and restored exactly. Loading a save and running 1000 ticks must equal running the original world those same 1000 ticks. This is a permanent CI test.
* `format_version` gates a migration pipeline: `migrations/vN_to_vN+1.rs`, pure functions on an intermediate dynamic representation. Old saves must load forever; a broken migration fails tests.
* Replay mode: because the sim is deterministic and player actions are the only external inputs, a save can be `{ seed + input log }`. Implement both full-state saves (fast load) and input-log replays (verification). The determinism CI test runs a replay and hash-compares world state every 10k ticks.
* World state hashing: a canonical `hash_world()` that walks components in stable order. This is the backbone of all determinism testing.

## 10. Simulation level-of-detail (the 10k-citizen strategy)

Every citizen is always real — permanent identity, ledger, relationships, employment — but not every citizen burns CPU every tick. Three tiers:

* Tier A — Embodied (target ≤ ~200 citizens): full per-tick simulation — utility AI, movement between locations, minute-level schedule execution, conversations. Membership: citizens near the player's focus, plus any citizen currently involved in a high-signal situation (crime, bankruptcy, birth, strike).
* Tier B — Scheduled (target ≤ ~2,000): simulated per-hour. The agent's day is executed as schedule blocks (sleep, commute, work shift, shop, socialize) with needs integrated analytically over each block instead of per tick. Purchases and social interactions resolve as aggregate draws from the agent's own distributions. Same decisions, coarser integration.
* Tier C — Statistical (everyone else): simulated per-day. Each agent owns a compact `DayModel` — expected spending by category, labor supplied, need trajectories, relationship drift — derived from their traits, job, and wealth. Tier C agents still hold jobs, still get paid through real payroll, still pay real rent to real landlords, still buy from real firms (their demand aggregates into market orders tagged with their identity so conservation and audit trails hold per-person).
* Promotion/demotion is deterministic (rule-based on focus distance and event flags, evaluated in entity order) and lossless: promoting a Tier C citizen materializes a plausible instantaneous state (position, current activity, need levels) from their DayModel and the current time — deterministically from their RNG stream — and demoting integrates their embodied state back into the model. An audit test proves a citizen cycled A→C→A conserves money exactly and needs within tolerance.
* Catch-up simulation (§5) is simply "everyone runs Tier C."
* Performance budget, enforced by a benchmark in CI: 10,000 citizens, mixed tiers as above, ≥ 200 ticks/second on a mid-range desktop core in release mode.

Optimization discipline: no speculative optimization before Phase 9. Until then, correct and clear beats fast, but never design something that structurally prevents the tier system (e.g., agent logic that only works per-tick).

## 11. AI architecture

Utility AI, layered, no behavior trees, no scripts.

* Needs: each citizen has a vector of needs (hunger, rest, shelter, safety, social, esteem, purpose) with data-defined decay curves and satisfaction sources. Need urgency is a nonlinear function (low hunger is ignorable; critical hunger dominates everything).
* Personality: trait vector (data-defined; e.g., industriousness, sociability, risk tolerance, frugality, ambition) sampled at creation from distributions, heritable with mutation. Traits are weights in utility scoring, which is how two citizens in identical situations behave differently.
* Decision loop (Tier A): enumerate available actions from context (afforded by location, time, inventory, relationships, job) → score each: `utility = Σ (need_gain × urgency × trait_weight) − costs (money via marginal utility of wealth, time, effort, social risk)` → pick argmax with deterministic tie-breaking (action id order). Scoring uses floats; the chosen action and its effects are exact.
* Planning: a lightweight hierarchical layer above action scoring: goals (get a job, save for a house, court someone, start a firm) decompose into standing intentions that bias daily action scoring. No full GOAP; goals inject scored candidate actions, they do not dictate sequences.
* Schedules: each citizen compiles a next-day schedule each evening from obligations (work shift, school) and intentions, leaving flexible blocks. Tier B executes the schedule directly; Tier A treats it as the default plan that live utility scoring can override.
* Memory: bounded episodic memory (ring of N salient events with decay) plus semantic memory (learned facts: "the bakery is cheap", "Jonas is unreliable") stored as weighted beliefs updated by experience and gossip. Memory influences scoring (a citizen cheated by a shop discounts it). Salience rules and capacities are data-defined.
* Relationships: a sparse weighted graph (kinship, friendship, romance, rivalry, professional), edges updated by interaction events and decayed by absence. Relationship values feed utility (visiting friends satisfies social need proportional to bond) and economics (hiring bias, informal loans, gossip propagation).
* The AI must be inspectable: every Tier A decision can dump its full scored candidate list to the debugger ("why did Mara skip work?" must be answerable in one click).

## 12. Economic simulation

Grounded in mechanism, not in faked aggregates. Macro numbers (inflation, unemployment) are measured from micro activity, never set.

* Goods & recipes: directed production graph loaded from data (grain → flour → bread; timber → lumber → furniture). Firms transform inputs to outputs over time with labor, equipment, and recipe efficiency. Spoilage and depreciation are explicit sinks.
* Markets: per-good local markets. Perishable/retail goods use posted prices: firms set prices via cost-plus with an adjustment controller (inventory above target → cut price, stockouts → raise), bounded per-day movement. Commodities and labor use a daily double-auction clearing (limit orders, price-time priority). Which mechanism a good uses is data-defined.
* Firms: full double-entry ledger — assets, liabilities, equity; payroll, rent, input purchases, loan service, dividends, taxes. Hiring/firing from marginal-productivity heuristics weighted by owner traits. Entry: citizens with capital + relevant skill + ambition trait may found firms where measured demand is unmet. Exit: insolvency → bankruptcy event → asset liquidation through the market, employees to the labor market. No firm is immortal.
* Labor: a matching market — firms post vacancies (wage, skill req), citizens search with reservation wages from needs, wealth, and outside options. Wages emerge. Unemployment is counted, never assigned.
* Housing: discrete stock owned by citizens/firms; rental and purchase markets with the same matching machinery; construction firms add stock when price/cost ratios justify it. Rent burden feeds citizen budgets and migration pressure.
* Banking: one bank (Phase 6+): deposits, collateralized loans with risk-scored interest, a policy rate set by a simple Taylor-style rule reacting to measured inflation — the only macro lever, and it is a modeled actor.
* External world: imports/exports at boundary prices with elasticity and transport cost — the explicit source/sink that closes the conservation audit.
* Consumer behavior: budget allocation from needs, prices, wealth, and traits (frugality, brand-loyalty via memory beliefs), with substitution when relative prices shift.
* Taxes: data-defined income/sales/property taxes → town treasury → public wages and services (school, clinic), closing the loop into education and health.
* Auditor (permanent): every day, sum all money across all ledgers; assert equality with issuance minus destruction. Same for every good against production/consumption/trade/spoilage records. Any drift halts a debug build with a diff of the offending ledgers.

## 13. Debugging & visualization tools

Built alongside systems, not after. Each phase's tooling is part of its deliverable.

* Headless CLI (`headless`): run seed for N ticks; dump world hash; dump time-series CSVs (prices, unemployment, population, money supply); run invariant audits; diff two runs.
* Inspector (viewer): click any citizen/firm → full state: needs, ledger, schedule, memory, relationships, last 50 events, last decision's scored candidates. Search by name/id. Follow-entity mode.
* Overlays: toggleable map layers — wealth choropleth, need heatmaps, price by store, LOD tier coloring, commute flows, relationship graph for a selected citizen.
* Time-series dashboard: in-viewer plots of any registered metric; metrics registry in `debug_tools` that any system can publish to.
* Event log browser: filterable by entity/type/tick range; the narrative composer view sits on top of it.
* Tracing: `tracing` crate spans per system per tick; a per-tick timing table in the viewer for the performance phase.

## 14. Testing strategy

* Unit tests per crate for all pure logic (utility scoring, price controllers, ledger ops, fixed-point math edge cases).
* Determinism suite (CI-blocking from Phase 0): (a) same seed, two fresh runs, hash-compare every 10k ticks; (b) save at tick T, load, run to T+N, compare with uninterrupted run; (c) replay from input log matches full-state run.
* Conservation suite: money and goods audits over long runs (100+ simulated days) at multiple population scales, including across save/load and across LOD promotions.
* Property tests (`proptest`): ledgers never go inconsistent under random valid operation sequences; markets never clear at negative prices; schedule compiler always covers 24h.
* Statistical regression tests: golden-seed runs assert macro measurements stay within bands (unemployment 2–15%, inflation −2–10%/yr, Gini within range, no good's price pinned at a bound for >30 days). These catch balance-destroying changes without asserting exact values.
* Benchmarks (`criterion`): ticks/sec at 1k/5k/10k citizens; catch-up throughput. Regressions >15% fail the check script.
* Target: every bug fixed gets a test that would have caught it. No phase is complete with failing or skipped tests.

## 15. Development phases

Work strictly in order. Each phase ends with: all tests green, `scripts/check.sh` clean, a written report in `docs/phase_reports/phase_N.md` (what was built, how verified, known limitations, deviations from spec with justification), and a tagged commit. Do not begin phase N+1 until phase N's report exists. If a phase reveals a flaw in this spec, write an ADR proposing the change and proceed with the amended design — never silently diverge.

* Phase 0 — Skeleton & determinism harness. Workspace, core_types, core_rng, core_ecs, minimal tick loop, world hashing, save/load of a trivial world, CLI runner, determinism CI tests. Exit: two runs of 1M empty ticks hash-identical; save/load/continue matches.
* Phase 1 — Time, events, data loading. Calendar, scheduler, event bus + log, RON registries + validator, balance-file plumbing. Exit: data validation catches seeded errors; scheduled events fire deterministically across save/load.
* Phase 2 — People exist. Citizen entities: identity, traits, needs with decay, lifecycle (age, deterministic mortality curves), households, name generation. No AI yet — needs decay and people age. Inspector v1 (headless queries). Exit: 10k citizens simulate a year; demographics within data-defined bands; conservation trivially holds.
* Phase 3 — Utility AI core. Action framework, need-based scoring with traits, schedules, locations as abstract nodes (no map yet), Tier A only at small scale (500 citizens). Decision-dump debugging. Exit: citizens visibly satisfy needs with individually distinct patterns; every decision inspectable; determinism holds.
* Phase 4 — Goods, firms, first market. Inventories, recipes, a 5-good chain (grain→flour→bread + water + timber→firewood), firms with double-entry ledgers, posted-price retail market, citizens buy food with seeded wealth. Money & goods auditors go live. Exit: audits pass over 100 days; prices respond to seeded supply shocks in the correct direction.
* Phase 5 — Labor & payroll. Vacancies, job search, hiring, wages, payroll, firing, unemployment measurement. Citizens earn instead of living on seeded wealth. Exit: labor market clears; wage distribution emerges; a firm's collapse produces measurable local unemployment.
* Phase 6 — Housing, banking, taxes. Housing stock, rental/purchase matching, the bank (deposits/loans/policy rule), tax pipeline, treasury, public employees. Exit: full monetary loop closes; conservation audit spans every ledger including bank and treasury; a rate change measurably shifts credit and construction.
* Phase 7 — Social layer. Relationships graph, memory/beliefs, gossip, courtship/marriage/reproduction, education pipeline affecting skills. Exit: multi-generation run produces kinship networks and skill mobility; narrative composer surfaces coherent story lines from real event chains.
* Phase 8 — LOD tiers & catch-up. DayModels, Tier B/C execution, promotion/demotion, catch-up controller. Exit: 10k citizens hit the 200 ticks/sec budget; A↔C cycling conserves money exactly; catch-up of one week < 5s; macro time-series statistically indistinguishable (defined tolerance) between all-Tier-A small towns and tiered ones.
* Phase 9 — Performance & town map. Profile-guided optimization, spatial layout with districts and travel times, commute costs entering utility and housing prices. Exit: budgets met with headroom; location visibly stratifies rents and shop revenues.
* Phase 10 — Viewer & polish. Full egui viewer: map, overlays, inspector, dashboards, event browser, time controls, snapshot protocol. Exit: an observer can follow one citizen's week and reconstruct their story entirely from tooling.

## 16. Anti-shortcut directives (BINDING)

1. No placeholders. Never write a function that returns a canned value "for now," a `todo!()`, or a stub system registered in the schedule. If a system isn't built, it doesn't exist in the schedule. Partial features are cut at a clean seam (e.g., "no banking yet"), never faked (e.g., "loans always approved").
2. No hardcoded tunables. Per §8 — balance values live in data files, loaded and validated. The grep test applies.
3. No speculative complexity. Build what the current phase's exit criteria require, structured so later phases extend rather than rewrite. No plugin frameworks, no scripting languages, no generic "modding API" until a concrete phase needs one. YAGNI, enforced.
4. No silent spec deviations. Any deviation requires an ADR in `docs/decisions/` written before the deviating code.
5. No untested claims. "It works" means a test demonstrates it. Phase reports must reference the specific tests/benchmarks that prove each exit criterion.
6. Determinism is never deferred. Any PR-equivalent change runs the determinism suite. A change that breaks it is reverted or fixed immediately, not logged as known-broken.
7. When uncertain, ask or decide via ADR — never guess silently. If the spec underdetermines something (e.g., exact need-decay shape), make a documented, data-driven choice with the reasoning recorded.

## 17. How to begin

Start with Phase 0. Before writing code, produce:

1. `docs/decisions/0001-architecture-confirmation.md` — restate the architecture, list the first risks, and confirm or challenge (via ADR) each fixed decision in §2.
2. The workspace skeleton with empty crates and the dependency rules encoded in Cargo.toml.
3. The determinism harness — before any simulation content exists.

Then proceed phase by phase. Treat this document as the contract; treat the phase reports as the lab notebook; treat the determinism and conservation suites as the ground truth that outranks any intuition.
