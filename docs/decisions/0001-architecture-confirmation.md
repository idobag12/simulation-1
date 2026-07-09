# ADR 0001 — Architecture confirmation for Embervale

- Status: Accepted
- Date: 2026-07-09
- Phase: 0

## Context

The project specification (`docs/SPEC.md`) requires, before any code is
written, a restatement of the architecture in the implementer's own words, a
list of the first risks, and an explicit confirm-or-challenge of every fixed
technology decision in SPEC §2.

## Architecture restated

Embervale is a **headless, deterministic, fixed-timestep world simulation**
organized as a strictly layered Cargo workspace:

- **Core layer** (`core_types`, `core_rng`, `core_ecs`, `core_events`):
  unit-safe newtypes and canonical binary codec; named deterministic RNG
  streams; a thin generational-index ECS with explicitly ordered system
  schedules and a command buffer; a typed event bus with a logged ring buffer.
  Nothing in this layer knows what a citizen or a firm is.
- **Simulation layer** (`sim_time`, `sim_people`, `sim_ai`, `sim_economy`,
  `sim_goods`, `sim_world`, `sim_lod`): the actual town. Sim crates never call
  each other directly; they interact only through components and events whose
  shared definitions live in a `sim_interface` module inside `core_ecs`.
  `sim_time` owns the tick loop (1 tick = 1 simulated minute), the calendar,
  and multi-rate scheduling (tick/hour/day/season/year), later including the
  catch-up integrator.
- **Service layer** (`persistence`, `data_defs`, `debug_tools`): versioned
  save/load with a migration pipeline and replay verification; RON content
  registries with startup validation; auditors, metrics, and the narrative
  observer. All authored content and every tunable number lives in `data/`.
- **Front layer** (`headless`, `viewer`): a CLI runner that can run, hash,
  audit, and diff worlds without any rendering dependency; later an egui
  viewer that consumes read-only snapshots only.

Causality flows exclusively through ticks: systems run in a literal,
documented order; mutations during iteration go through a command buffer;
randomness comes only from named per-system PCG64 streams; all persisted or
conserved quantities are 64-bit integers. Determinism (bit-identical state
from equal seed + inputs), conservation (money/goods only created or
destroyed at modeled sources/sinks), no-magic state changes, and headless
purity are the four permanent invariants, each backed by permanently-running
test suites rather than by convention.

## Confirmation of SPEC §2 fixed decisions

| Decision | Verdict | Notes |
|---|---|---|
| Rust, stable toolchain | **Confirmed** | Toolchain in this environment: stable 1.94.1. `Cargo.lock` is committed so dependency behavior is reproducible. We do not pin `rust-toolchain.toml` (the execution environment provides one stable toolchain); revisit via ADR if cross-machine drift is ever observed. |
| Cargo workspace, strictly layered crates | **Confirmed** | One deliberate mapping: the repository root *is* the `embervale/` root from SPEC §4 (the repo is the project; no extra nesting directory). |
| Thin custom ECS, no bevy_ecs/specs, ≤ ~800 lines | **Confirmed** | Design in ADR 0002: generational indices, per-component choice of dense `Vec<Option<T>>` vs sparse `BTreeMap<u32, T>`, type-erased store registry with **stable explicit component names** (never `type_name!`), canonical serialized form shared by saves and hashing. No `unsafe` needed so far — `#![forbid(unsafe_code)]` everywhere until profiling proves otherwise (stricter than spec). |
| Fixed-point 64-bit economic quantities | **Confirmed** | `Money(i64)` in mills from Phase 0 with checked, typed-error arithmetic. Further unit newtypes (`Quantity`, etc.) are added in the phase that first uses them (SPEC §16.3 YAGNI). |
| `rand_pcg::Pcg64` via named-stream `RngRegistry` | **Confirmed** | Stream seeds are derived by an **in-house, documented, frozen** function (FNV-1a name hash + splitmix64 expansion, ADR 0002) rather than `SeedableRng::seed_from_u64`, so save-format stability never depends on `rand_core` internals. Streams are created lazily; creation is a pure function of (master seed, stream name), so first-touch order cannot affect determinism. |
| `serde` + `bincode` saves; RON data | **Confirmed** | bincode v2 with its serde bridge and the `standard()` config, wrapped in a single `core_types::codec` module so there is exactly one encoding configuration in the codebase. RON arrives in Phase 1 with `data_defs`. |
| `egui` + `eframe` viewer in separate crate | **Confirmed** | The `viewer` crate exists as an empty shell now; the egui/eframe dependencies are added only in the phase that builds it, keeping Phase 0–9 builds free of rendering dependencies (this also serves invariant 4). |
| No other dependencies without ADR | **Confirmed** | Phase 0's full dependency set and justification: ADR 0002 §Dependencies. |

## First risks identified

1. **Float creep into persisted state.** One `f64` in a component silently
   breaks cross-platform bit-determinism. Mitigation: floats are confined to
   AI scoring/rendering by policy; the determinism suite hash-compares full
   state; review checklist item on every phase.
2. **Save-format identity drift.** Anything that leaks unstable identity into
   saves or hashes (Rust `type_name`, `HashMap` order, `Vec<Option<T>>`
   trailing-`None` padding, RNG crate internals) breaks "load = continue".
   Mitigation: explicit `Component::NAME` constants, canonical sorted-pair
   store serialization, in-house seed derivation, registration order fixed in
   one function per application, save/load/continue equality as a permanent
   CI test.
3. **Accidental nondeterministic iteration.** `HashMap` is allowed only for
   lookup, never iteration, in simulation code. Mitigation: coding-standard
   lint discipline, the single `by_type` lookup map is documented as
   lookup-only, review greps for `HashMap` in `sim_*`/`core_*` iteration
   positions.
4. **The ECS line budget vs. later needs (LOD storage, read/write
   declarations for future parallelism).** Mitigation: keep the ECS free of
   speculative features now; the erased-store design already permits swapping
   a store's layout per component without touching call sites.
5. **Performance at 10k citizens with `Vec<Option<T>>` and per-tick
   serialization-based hashing.** Accepted until Phase 9 by explicit spec
   instruction (no speculative optimization); hashing is test/CLI-path only,
   not part of a normal tick.
6. **zstd/bincode version upgrades changing byte output.** State hashes are
   computed over *uncompressed canonical bytes*, so compression can never
   affect determinism; a version bump that changes serialized layout must
   bump `format_version` and ship a migration, enforced by save-compat tests.
7. **Cross-phase coupling temptation** (e.g., economy reaching into people).
   Mitigation: the dependency rule is encoded in each crate's manifest and
   checked in review; sim↔sim only via `sim_interface` components/events.

## Consequences

Phase 0 builds exactly: workspace skeleton with all layered crates,
`core_types`, `core_rng`, `core_ecs` (entities, stores, world, schedule,
command buffer), a minimal `sim_time` tick loop, canonical world hashing,
versioned save/load in `persistence`, the `headless` CLI, and the
determinism suite — no calendar, no events, no content. Concrete Phase 0
design decisions are recorded in ADR 0002.
